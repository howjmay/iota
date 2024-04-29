//! Contains the logic for the migration process.
use std::{
    collections::HashMap,
    io::{BufWriter, Write},
    sync::Arc,
};

use anyhow::Result;
use fastcrypto::hash::HashFunction;
use iota_sdk::types::block::output::{
    AliasOutput, BasicOutput, FoundryOutput, NftOutput, TokenId, TreasuryOutput,
};
use move_vm_runtime_v2::move_vm::MoveVM;
use sui_adapter_v2::{
    adapter::new_move_vm, gas_charger::GasCharger, programmable_transactions,
    temporary_store::TemporaryStore,
};
use sui_framework::BuiltInFramework;
use sui_move_build::CompiledPackage;
use sui_move_natives_v2::all_natives;
use sui_protocol_config::{Chain, ProtocolConfig, ProtocolVersion};
use sui_types::{
    base_types::{ObjectID, SuiAddress, TxContext},
    crypto::DefaultHash,
    digests::TransactionDigest,
    epoch_data::EpochData,
    execution_mode,
    in_memory_storage::InMemoryStorage,
    inner_temporary_store::InnerTemporaryStore,
    metrics::LimitsMetrics,
    move_package::UpgradeCap,
    object::Object,
    programmable_transaction_builder::ProgrammableTransactionBuilder,
    transaction::{
        CheckedInputObjects, Command, InputObjectKind, ObjectReadResult, ProgrammableTransaction,
    },
    MOVE_STDLIB_PACKAGE_ID, STARDUST_PACKAGE_ID, SUI_FRAMEWORK_PACKAGE_ID,
};

use crate::process_package;

/// The dependencies of the generated packages for native tokens.
pub const PACKAGE_DEPS: [ObjectID; 3] = [
    MOVE_STDLIB_PACKAGE_ID,
    SUI_FRAMEWORK_PACKAGE_ID,
    STARDUST_PACKAGE_ID,
];

}

/// Creates the objects that map to the stardust UTXO ledger.
///
/// Internally uses an unmetered Move VM.
struct Executor {
    protocol_config: ProtocolConfig,
    tx_context: TxContext,
    store: InMemoryStorage,
    move_vm: Arc<MoveVM>,
    metrics: Arc<LimitsMetrics>,
    /// Map the stardust token id [`TokenId`] to the [`ObjectID`] of the
    /// coin minted by the foundry.
    native_tokens: HashMap<TokenId, ObjectID>,
}

impl Executor {
    /// Setup the execution environment backed by an in-memory store that holds
    /// all the system packages.
    fn new(protocol_version: ProtocolVersion) -> Result<Self> {
        let mut tx_context = create_migration_context();
        // Use a throwaway metrics registry for transaction execution.
        let metrics = Arc::new(LimitsMetrics::new(&prometheus::Registry::new()));
        let mut store = InMemoryStorage::new(Vec::new());
        // We don't know the chain ID here since we haven't yet created the genesis checkpoint.
        // However since we know there are no chain specific protool config options in genesis,
        // we use Chain::Unknown here.
        let protocol_config = ProtocolConfig::get_for_version(protocol_version, Chain::Unknown);
        // Get the correct system packages for our protocol version. If we cannot find the snapshot
        // that means that we must be at the latest version and we should use the latest version of the
        // framework.
        let mut system_packages =
            sui_framework_snapshot::load_bytecode_snapshot(protocol_version.as_u64())
                .unwrap_or_else(|_| BuiltInFramework::iter_system_packages().cloned().collect());
        // TODO: Remove when we have bumped the protocol to include the stardust packages
        // into the system packages.
        //
        // See also: https://github.com/iotaledger/kinesis/pull/149
        system_packages.extend(BuiltInFramework::iter_stardust_packages().cloned());

        let silent = true;
        let executor = sui_execution::executor(&protocol_config, silent, None)
            .expect("Creating an executor should not fail here");
        for system_package in system_packages.into_iter() {
            process_package(
                &mut store,
                executor.as_ref(),
                &mut tx_context,
                &system_package.modules(),
                system_package.dependencies().to_vec(),
                &protocol_config,
                metrics.clone(),
            )?;
        }
        let move_vm = Arc::new(new_move_vm(all_natives(silent), &protocol_config, None)?);
        Ok(Self {
            protocol_config,
            tx_context,
            store,
            move_vm,
            metrics,
            native_tokens: Default::default(),
        })
    }

    fn load_dependencies(&self) -> CheckedInputObjects {
        CheckedInputObjects::new_for_genesis(
            PACKAGE_DEPS
                .iter()
                .zip(self.store.get_objects(&PACKAGE_DEPS))
                .filter_map(|(dependency, object)| {
                    Some(ObjectReadResult::new(
                        InputObjectKind::MovePackage(*dependency),
                        object?.clone().into(),
                    ))
                })
                .collect(),
        )
    }

    fn execute_pt_unmetered(
        &mut self,
        input_objects: CheckedInputObjects,
        pt: ProgrammableTransaction,
    ) -> Result<InnerTemporaryStore> {
        let input_objects = input_objects.into_inner();
        let mut temporary_store = TemporaryStore::new(
            &self.store,
            input_objects,
            vec![],
            self.tx_context.digest(),
            &self.protocol_config,
        );
        let mut gas_charger = GasCharger::new_unmetered(self.tx_context.digest());
        programmable_transactions::execution::execute::<execution_mode::Normal>(
            &self.protocol_config,
            self.metrics.clone(),
            &self.move_vm,
            &mut temporary_store,
            &mut self.tx_context,
            &mut gas_charger,
            pt,
        )?;
        temporary_store.update_object_version_and_prev_tx();
        Ok(temporary_store.into_inner())
    }

    fn update_store(&mut self, objects: Vec<Object>) {
        self.store
            .finish(objects.into_iter().map(|obj| (obj.id(), obj)).collect());
    }

    /// Process the foundry outputs as follows:
    ///
    /// * Publish the generated packages using a tailored unmetered executor.
    /// * For each native token, map the [`TokenId`] to the [`ObjectID`] of the
    ///   coin that holds its total supply.
    /// * Update the inner store with the created objects.
    fn create_foundries(
        &mut self,
        foundries: impl Iterator<Item = (FoundryOutput, CompiledPackage)>,
    ) -> Result<()> {
        for (foundry, pkg) in foundries {
            let modules = package_module_bytes(&pkg)?;
            let deps = self.load_dependencies();
            let pt = {
                let mut builder = ProgrammableTransactionBuilder::new();
                builder.command(Command::Publish(modules, PACKAGE_DEPS.into()));
                builder.finish()
            };
            let InnerTemporaryStore { written, .. } = self.execute_pt_unmetered(deps, pt)?;
            let minted_coin_id = written
                .iter()
                // Since the execution is unmetered, the only coin written
                // holds the total minted supply of native tokens
                .find_map(|(id, object)| object.is_coin().then_some(*id))
                .expect("the must be a coin create in the transaction");
            self.native_tokens
                .insert(*foundry.native_tokens()[0].token_id(), minted_coin_id);
            self.store.finish(
                written
                    .into_iter()
                    // We ignore the [`UpgradeCap`] objects.
                    .filter(|(_, object)| object.struct_tag() != Some(UpgradeCap::type_()))
                    .collect(),
            );
        }
        Ok(())
    }

    fn create_alias_objects(&mut self, _alias: AliasOutput) -> Result<Vec<Object>> {
        todo!();
    }

    fn create_basic_objects(&mut self, _basic_output: BasicOutput) -> Result<Vec<Object>> {
        todo!();
    }

    fn create_nft_objects(&mut self, _nft: NftOutput) -> Result<Vec<Object>> {
        todo!();
    }

    fn create_treasury_objects(&mut self, _treasury: TreasuryOutput) -> Result<Vec<Object>> {
        todo!();
    }
}

/// Verify the ledger state represented by the objects in [`InMemoryStorage`].
fn verify_ledger_state(_store: &InMemoryStorage) -> Result<()> {
    todo!();
}

/// Serialize the objects stored in [`InMemoryStorage`] into a file using
/// [`bcs`] encoding.
fn create_snapshot(store: InMemoryStorage, writer: impl Write) -> Result<()> {
    let mut writer = BufWriter::new(writer);
    let objects = store
        .into_inner()
        .into_values()
        .filter(|object| !object.is_system_package())
        .collect::<Vec<_>>();
    writer.write_all(&bcs::to_bytes(&objects)?)?;
    Ok(writer.flush()?)
}

/// Get the bytes of all bytecode modules (not including direct or transitive
/// dependencies) of [`CompiledPackage`].
fn package_module_bytes(pkg: &CompiledPackage) -> Result<Vec<Vec<u8>>> {
    pkg.get_modules()
        .map(|module| {
            let mut buf = Vec::new();
            module.serialize(&mut buf)?;
            Ok(buf)
        })
        .collect::<Result<_>>()
}

/// Create a [`TxContext]` that remains the same across invokations.
fn create_migration_context() -> TxContext {
    let mut hasher = DefaultHash::default();
    hasher.update(b"stardust-migration");
    let hash = hasher.finalize();
    let stardust_migration_transaction_digest = TransactionDigest::new(hash.into());

    TxContext::new(
        &SuiAddress::default(),
        &stardust_migration_transaction_digest,
        &EpochData::new_genesis(0),
    )
}

#[cfg(test)]
mod tests {
    use sui_types::{inner_temporary_store::WrittenObjects, object::Object};

    use super::*;
    #[test]
    fn migration_create_and_deserialize_snapshot() {
        let mut persisted: Vec<u8> = Vec::new();
        let mut store = InMemoryStorage::default();
        let objects: WrittenObjects = (0..4)
            .map(|_| {
                let object = Object::new_gas_for_testing();
                (object.id(), object)
            })
            .collect();
        store.finish(objects.clone());
        create_snapshot(store, &mut persisted).unwrap();
        let snapshot_objects: Vec<Object> = bcs::from_bytes(&persisted).unwrap();
        assert_eq!(objects.into_values().collect::<Vec<_>>(), snapshot_objects);
    }
}
