// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, DirBuilder, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command},
    str::FromStr,
    sync::Arc,
};

use ethers::{prelude::*, types::Address as EthAddress};
use iota_config::local_ip_utils::get_available_port;
use iota_json_rpc_types::{
    IotaEvent, IotaTransactionBlockResponse, IotaTransactionBlockResponseOptions,
    IotaTransactionBlockResponseQuery, TransactionFilter,
};
use iota_sdk::{IotaClient, wallet_context::WalletContext};
use iota_test_transaction_builder::TestTransactionBuilder;
use iota_types::{
    IOTA_BRIDGE_OBJECT_ID,
    base_types::IotaAddress,
    bridge::BridgeChainId,
    committee::TOTAL_VOTING_POWER,
    crypto::{EncodeDecodeBase64, KeypairTraits, get_key_pair},
    digests::TransactionDigest,
    transaction::{ObjectArg, TransactionData},
};
use move_core_types::language_storage::StructTag;
use prometheus::Registry;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use serde::{Deserialize, Serialize};
use tempfile::tempdir;
use test_cluster::{TestCluster, TestClusterBuilder};
use tokio::{join, task::JoinHandle};
use tracing::{error, info};

use crate::{
    BRIDGE_ENABLE_PROTOCOL_VERSION,
    abi::{EthBridgeCommittee, EthBridgeConfig},
    config::{BridgeNodeConfig, EthConfig, IotaConfig},
    crypto::{BridgeAuthorityKeyPair, BridgeAuthorityPublicKeyBytes},
    events::*,
    iota_client::IotaBridgeClient,
    node::run_bridge_node,
    server::BridgeNodePublicMetadata,
    types::BridgeAction,
    utils::{EthSigner, get_eth_signer_client},
};

const BRIDGE_COMMITTEE_NAME: &str = "BridgeCommittee";
const IOTA_BRIDGE_NAME: &str = "IotaBridge";
const BRIDGE_CONFIG_NAME: &str = "BridgeConfig";
const BRIDGE_LIMITER_NAME: &str = "BridgeLimiter";
const BRIDGE_VAULT_NAME: &str = "BridgeVault";
const BTC_NAME: &str = "BTC";
const ETH_NAME: &str = "ETH";
const USDC_NAME: &str = "USDC";
const USDT_NAME: &str = "USDT";
const KA_NAME: &str = "KA";

pub const TEST_PK: &str = "0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356";

/// A helper struct that holds TestCluster and other Bridge related
/// structs that are needed for testing.
pub struct BridgeTestCluster {
    pub num_validators: usize,
    pub test_cluster: TestCluster,
    bridge_client: IotaBridgeClient,
    eth_environment: EthBridgeEnvironment,
    bridge_node_handles: Option<Vec<JoinHandle<()>>>,
    approved_governance_actions_for_next_start: Option<Vec<Vec<BridgeAction>>>,
    bridge_tx_cursor: Option<TransactionDigest>,
    eth_chain_id: BridgeChainId,
    iota_chain_id: BridgeChainId,
}

pub struct BridgeTestClusterBuilder {
    with_eth_env: bool,
    with_bridge_cluster: bool,
    num_validators: usize,
    approved_governance_actions: Option<Vec<Vec<BridgeAction>>>,
    eth_chain_id: BridgeChainId,
    iota_chain_id: BridgeChainId,
}

impl Default for BridgeTestClusterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BridgeTestClusterBuilder {
    pub fn new() -> Self {
        BridgeTestClusterBuilder {
            with_eth_env: false,
            with_bridge_cluster: false,
            num_validators: 4,
            approved_governance_actions: None,
            eth_chain_id: BridgeChainId::EthCustom,
            iota_chain_id: BridgeChainId::IotaCustom,
        }
    }

    pub fn with_eth_env(mut self, with_eth_env: bool) -> Self {
        self.with_eth_env = with_eth_env;
        self
    }

    pub fn with_bridge_cluster(mut self, with_bridge_cluster: bool) -> Self {
        self.with_bridge_cluster = with_bridge_cluster;
        self
    }

    pub fn with_num_validators(mut self, num_validators: usize) -> Self {
        self.num_validators = num_validators;
        self
    }

    pub fn with_approved_governance_actions(
        mut self,
        approved_governance_actions: Vec<Vec<BridgeAction>>,
    ) -> Self {
        assert_eq!(approved_governance_actions.len(), self.num_validators);
        self.approved_governance_actions = Some(approved_governance_actions);
        self
    }

    pub fn with_iota_chain_id(mut self, chain_id: BridgeChainId) -> Self {
        self.iota_chain_id = chain_id;
        self
    }

    pub fn with_eth_chain_id(mut self, chain_id: BridgeChainId) -> Self {
        self.eth_chain_id = chain_id;
        self
    }

    pub async fn build(self) -> BridgeTestCluster {
        init_all_struct_tags();
        std::env::set_var("__TEST_ONLY_CONSENSUS_USE_LONG_MIN_ROUND_DELAY", "1");
        let mut bridge_keys = vec![];
        let mut bridge_keys_copy = vec![];
        for _ in 0..self.num_validators {
            let (_, kp): (_, BridgeAuthorityKeyPair) = get_key_pair();
            bridge_keys.push(kp.copy());
            bridge_keys_copy.push(kp);
        }
        let start_cluster_task =
            tokio::task::spawn(Self::start_test_cluster(bridge_keys, self.num_validators));
        let start_eth_env_task = tokio::task::spawn(Self::start_eth_env(bridge_keys_copy));
        let (start_cluster_res, start_eth_env_res) = join!(start_cluster_task, start_eth_env_task);
        let test_cluster = start_cluster_res.unwrap();
        let eth_environment = start_eth_env_res.unwrap();

        let mut bridge_node_handles = None;
        if self.with_bridge_cluster {
            let approved_governace_actions = self
                .approved_governance_actions
                .clone()
                .unwrap_or(vec![vec![]; self.num_validators]);
            bridge_node_handles = Some(
                start_bridge_cluster(&test_cluster, &eth_environment, approved_governace_actions)
                    .await,
            );
        }
        let bridge_client = IotaBridgeClient::new(&test_cluster.fullnode_handle.rpc_url)
            .await
            .unwrap();
        BridgeTestCluster {
            num_validators: self.num_validators,
            test_cluster,
            bridge_client,
            eth_environment,
            bridge_node_handles,
            approved_governance_actions_for_next_start: self.approved_governance_actions,
            bridge_tx_cursor: None,
            iota_chain_id: self.iota_chain_id,
            eth_chain_id: self.eth_chain_id,
        }
    }

    async fn start_test_cluster(
        bridge_keys: Vec<BridgeAuthorityKeyPair>,
        num_validators: usize,
    ) -> TestCluster {
        assert_eq!(bridge_keys.len(), num_validators);
        let test_cluster: test_cluster::TestCluster = TestClusterBuilder::new()
            .with_num_validators(num_validators)
            .with_protocol_version(BRIDGE_ENABLE_PROTOCOL_VERSION.into())
            .build_with_bridge(bridge_keys, true)
            .await;
        info!("Test cluster built");
        test_cluster
            .trigger_reconfiguration_if_not_yet_and_assert_bridge_committee_initialized()
            .await;
        info!("Bridge committee is finalized");
        test_cluster
    }

    async fn start_eth_env(bridge_keys: Vec<BridgeAuthorityKeyPair>) -> EthBridgeEnvironment {
        let anvil_port = get_available_port("127.0.0.1");
        let anvil_url = format!("http://127.0.0.1:{anvil_port}");
        let mut eth_environment = EthBridgeEnvironment::new(&anvil_url, anvil_port)
            .await
            .unwrap();
        // Give anvil a bit of time to start
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        let (eth_signer, eth_pk_hex) = eth_environment
            .get_signer(TEST_PK)
            .await
            .unwrap_or_else(|e| panic!("Failed to get eth signer from anvil at {anvil_url}: {e}"));
        let deployed_contracts =
            deploy_sol_contract(&anvil_url, eth_signer, bridge_keys, eth_pk_hex).await;
        info!("Deployed contracts: {:?}", deployed_contracts);
        eth_environment.contracts = Some(deployed_contracts);
        eth_environment
    }
}

impl BridgeTestCluster {
    pub async fn get_eth_signer_and_private_key(&self) -> anyhow::Result<(EthSigner, String)> {
        self.eth_environment.get_signer(TEST_PK).await
    }

    pub async fn get_eth_signer_and_address(&self) -> anyhow::Result<(EthSigner, EthAddress)> {
        let (eth_signer, _) = self.get_eth_signer_and_private_key().await?;
        let eth_address = eth_signer.address();
        Ok((eth_signer, eth_address))
    }

    pub async fn get_eth_signer(&self) -> EthSigner {
        let (eth_signer, _) = self.get_eth_signer_and_private_key().await.unwrap();
        eth_signer
    }

    pub fn bridge_client(&self) -> &IotaBridgeClient {
        &self.bridge_client
    }

    pub fn iota_client(&self) -> &IotaClient {
        &self.test_cluster.fullnode_handle.iota_client
    }

    pub fn iota_user_address(&self) -> IotaAddress {
        self.test_cluster.get_address_0()
    }

    pub fn iota_chain_id(&self) -> BridgeChainId {
        self.iota_chain_id
    }

    pub fn eth_chain_id(&self) -> BridgeChainId {
        self.eth_chain_id
    }

    pub(crate) fn eth_env(&self) -> &EthBridgeEnvironment {
        &self.eth_environment
    }

    pub fn contracts(&self) -> &DeployedSolContracts {
        self.eth_environment.contracts()
    }

    pub fn iota_bridge_address(&self) -> String {
        self.eth_environment.contracts().iota_bridge_address_hex()
    }

    pub fn wallet_mut(&mut self) -> &mut WalletContext {
        self.test_cluster.wallet_mut()
    }

    pub fn wallet(&self) -> &WalletContext {
        &self.test_cluster.wallet
    }

    pub fn bridge_authority_key(&self, index: usize) -> BridgeAuthorityKeyPair {
        self.test_cluster.bridge_authority_keys.as_ref().unwrap()[index].copy()
    }

    pub fn iota_rpc_url(&self) -> String {
        self.test_cluster.fullnode_handle.rpc_url.clone()
    }

    pub fn eth_rpc_url(&self) -> String {
        self.eth_environment.rpc_url.clone()
    }

    pub async fn get_mut_bridge_arg(&self) -> Option<ObjectArg> {
        self.test_cluster.get_mut_bridge_arg().await
    }

    pub async fn test_transaction_builder_with_sender(
        &self,
        sender: IotaAddress,
    ) -> TestTransactionBuilder {
        self.test_cluster
            .test_transaction_builder_with_sender(sender)
            .await
    }

    pub async fn wait_for_bridge_cluster_to_be_up(&self, timeout_sec: u64) {
        self.test_cluster
            .wait_for_bridge_cluster_to_be_up(timeout_sec)
            .await;
    }

    pub async fn sign_and_execute_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> IotaTransactionBlockResponse {
        self.test_cluster
            .sign_and_execute_transaction(tx_data)
            .await
    }

    pub fn set_approved_governance_actions_for_next_start(
        &mut self,
        approved_governance_actions: Vec<Vec<BridgeAction>>,
    ) {
        assert_eq!(approved_governance_actions.len(), self.num_validators);
        self.approved_governance_actions_for_next_start = Some(approved_governance_actions);
    }

    pub async fn start_bridge_cluster(&mut self) {
        assert!(self.bridge_node_handles.is_none());
        let approved_governace_actions = self
            .approved_governance_actions_for_next_start
            .clone()
            .unwrap_or(vec![vec![], vec![], vec![], vec![]]);
        self.bridge_node_handles = Some(
            start_bridge_cluster(
                &self.test_cluster,
                &self.eth_environment,
                approved_governace_actions,
            )
            .await,
        );
    }

    /// Returns new bridge transaction. It advanaces the stored tx digest
    /// cursor. When `assert_success` is true, it asserts all transactions
    /// are successful.
    pub async fn new_bridge_transactions(
        &mut self,
        assert_success: bool,
    ) -> Vec<IotaTransactionBlockResponse> {
        let resps = self
            .iota_client()
            .read_api()
            .query_transaction_blocks(
                IotaTransactionBlockResponseQuery {
                    filter: Some(TransactionFilter::InputObject(IOTA_BRIDGE_OBJECT_ID)),
                    options: Some(IotaTransactionBlockResponseOptions::full_content()),
                },
                self.bridge_tx_cursor,
                None,
                false,
            )
            .await
            .unwrap();
        self.bridge_tx_cursor = resps.next_cursor;

        for tx in &resps.data {
            if assert_success {
                assert!(tx.status_ok().unwrap());
            }
            let events = &tx.events.as_ref().unwrap().data;
            if events
                .iter()
                .any(|e| &e.type_ == TokenTransferApproved.get().unwrap())
            {
                assert!(
                    events
                        .iter()
                        .any(|e| &e.type_ == TokenTransferClaimed.get().unwrap()
                            || &e.type_ == TokenTransferApproved.get().unwrap())
                );
            } else if events
                .iter()
                .any(|e| &e.type_ == TokenTransferAlreadyClaimed.get().unwrap())
            {
                assert!(
                    events
                        .iter()
                        .all(|e| &e.type_ == TokenTransferAlreadyClaimed.get().unwrap()
                            || &e.type_ == TokenTransferAlreadyApproved.get().unwrap())
                );
            }
            // TODO: check for other events e.g. TokenRegistrationEvent,
            // NewTokenEvent etc
        }
        resps.data
    }

    /// Returns events that are emitted in new bridge transaction and match
    /// `event_types`. It advanaces the stored tx digest cursor.
    /// See `new_bridge_transactions` for `assert_success`.
    pub async fn new_bridge_events(
        &mut self,
        event_types: HashSet<StructTag>,
        assert_success: bool,
    ) -> Vec<IotaEvent> {
        let txes = self.new_bridge_transactions(assert_success).await;
        let events = txes
            .iter()
            .flat_map(|tx| {
                tx.events
                    .as_ref()
                    .unwrap()
                    .data
                    .iter()
                    .filter(|e| event_types.contains(&e.type_))
                    .cloned()
            })
            .collect();
        events
    }
}

pub async fn get_eth_signer_client_e2e_test_only(
    eth_rpc_url: &str,
) -> anyhow::Result<(EthSigner, String)> {
    // This private key is derived from the default anvil setting.
    // Mnemonic:          test test test test test test test test test test test
    // junk Derivation path:   m/44'/60'/0'/0/
    // DO NOT USE IT ANYWHERE ELSE EXCEPT FOR RUNNING AUTOMATIC INTEGRATION TESTING
    let url = eth_rpc_url.to_string();
    let private_key_0 = "0x4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356";
    let signer_0 = get_eth_signer_client(&url, private_key_0).await?;
    Ok((signer_0, private_key_0.to_string()))
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DeployedSolContracts {
    pub iota_bridge: EthAddress,
    pub bridge_committee: EthAddress,
    pub bridge_limiter: EthAddress,
    pub bridge_vault: EthAddress,
    pub bridge_config: EthAddress,
    pub btc: EthAddress,
    pub eth: EthAddress,
    pub usdc: EthAddress,
    pub usdt: EthAddress,
    pub ka: EthAddress,
}

impl DeployedSolContracts {
    pub fn eth_address_to_hex(addr: EthAddress) -> String {
        format!("{:x}", addr)
    }

    pub fn iota_bridge_address_hex(&self) -> String {
        Self::eth_address_to_hex(self.iota_bridge)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SolDeployConfig {
    committee_member_stake: Vec<u64>,
    committee_members: Vec<String>,
    min_committee_stake_required: u64,
    source_chain_id: u64,
    supported_chain_ids: Vec<u64>,
    supported_chain_limits_in_dollars: Vec<u64>,
    supported_tokens: Vec<String>,
    token_prices: Vec<u64>,
}

pub(crate) async fn deploy_sol_contract(
    anvil_url: &str,
    eth_signer: EthSigner,
    bridge_authority_keys: Vec<BridgeAuthorityKeyPair>,
    eth_private_key_hex: String,
) -> DeployedSolContracts {
    let sol_path = format!("{}/../../bridge/evm", env!("CARGO_MANIFEST_DIR"));

    // Write the deploy config to a temp file then provide it to the forge late
    let deploy_config_path = tempfile::tempdir()
        .unwrap()
        .into_path()
        .join("sol_deploy_config.json");
    let node_len = bridge_authority_keys.len();
    let stake = TOTAL_VOTING_POWER / (node_len as u64);
    let committee_members = bridge_authority_keys
        .iter()
        .map(|k| {
            format!(
                "0x{:x}",
                BridgeAuthorityPublicKeyBytes::from(&k.public).to_eth_address()
            )
        })
        .collect::<Vec<_>>();
    let mut committee_member_stake = vec![stake; node_len];
    // Adjust it so that the total stake is equal to TOTAL_VOTING_POWER
    committee_member_stake[node_len - 1] = TOTAL_VOTING_POWER - stake * (node_len as u64 - 1);
    let deploy_config = SolDeployConfig {
        committee_member_stake: committee_member_stake.clone(),
        committee_members: committee_members.clone(),
        min_committee_stake_required: 10000,
        source_chain_id: 12,
        supported_chain_ids: vec![1, 2, 3],
        supported_chain_limits_in_dollars: vec![
            1000000000000000,
            1000000000000000,
            1000000000000000,
        ],
        supported_tokens: vec![], // this is set up in the deploy script
        token_prices: vec![12800, 432518900, 25969600, 10000, 10000],
    };

    let serialized_config = serde_json::to_string_pretty(&deploy_config).unwrap();
    tracing::debug!(
        "Serialized config written to {:?}: {:?}",
        deploy_config_path,
        serialized_config
    );
    let mut file = File::create(deploy_config_path.clone()).unwrap();
    file.write_all(serialized_config.as_bytes()).unwrap();

    // override for the deploy script
    std::env::set_var("OVERRIDE_CONFIG_PATH", deploy_config_path.to_str().unwrap());
    std::env::set_var("PRIVATE_KEY", eth_private_key_hex);
    std::env::set_var("ETHERSCAN_API_KEY", "n/a");

    // We provide a unique out path for each run to avoid conflicts
    let mut rng = SmallRng::from_entropy();
    let random_number = rng.gen::<u32>();
    let forge_out_path = PathBuf::from(format!("out-{random_number}"));
    let _dir = TempDir::new(
        PathBuf::from(sol_path.clone())
            .join(forge_out_path.clone())
            .as_path(),
    )
    .unwrap();
    std::env::set_var("FOUNDRY_OUT", forge_out_path.to_str().unwrap());

    info!("Deploying solidity contracts");
    Command::new("forge")
        .current_dir(sol_path.clone())
        .arg("clean")
        .status()
        .expect("Failed to execute `forge clean`");

    let mut child = Command::new("forge")
        .current_dir(sol_path)
        .arg("script")
        .arg("script/deploy_bridge.s.sol")
        .arg("--fork-url")
        .arg(anvil_url)
        .arg("--broadcast")
        .arg("--ffi")
        .arg("--chain")
        .arg("31337")
        .stdout(std::process::Stdio::piped()) // Capture stdout
        .stderr(std::process::Stdio::piped()) // Capture stderr
        .spawn()
        .unwrap();

    let mut stdout = child.stdout.take().expect("Failed to open stdout");
    let mut stderr = child.stderr.take().expect("Failed to open stderr");

    // Read stdout/stderr to String
    let mut s = String::new();
    stdout.read_to_string(&mut s).unwrap();
    let mut e = String::new();
    stderr.read_to_string(&mut e).unwrap();

    // Wait for the child process to finish and collect its status
    let status = child.wait().unwrap();
    if status.success() {
        info!("Solidity contract deployment finished successfully");
    } else {
        error!(
            "Solidity contract deployment exited with code: {:?}",
            status.code()
        );
    }
    println!("Stdout: {}", s);
    println!("Stdout: {}", e);

    let mut deployed_contracts = BTreeMap::new();
    // Process the stdout to parse contract addresses
    for line in s.lines() {
        if line.contains("[Deployed]") {
            let replaced_line = line.replace("[Deployed]", "");
            let trimmed_line = replaced_line.trim();
            let parts: Vec<&str> = trimmed_line.split(':').collect();
            if parts.len() == 2 {
                let contract_name = parts[0].to_string().trim().to_string();
                let contract_address = EthAddress::from_str(parts[1].to_string().trim()).unwrap();
                deployed_contracts.insert(contract_name, contract_address);
            }
        }
    }

    let contracts = DeployedSolContracts {
        iota_bridge: deployed_contracts.remove(IOTA_BRIDGE_NAME).unwrap(),
        bridge_committee: deployed_contracts.remove(BRIDGE_COMMITTEE_NAME).unwrap(),
        bridge_config: deployed_contracts.remove(BRIDGE_CONFIG_NAME).unwrap(),
        bridge_limiter: deployed_contracts.remove(BRIDGE_LIMITER_NAME).unwrap(),
        bridge_vault: deployed_contracts.remove(BRIDGE_VAULT_NAME).unwrap(),
        btc: deployed_contracts.remove(BTC_NAME).unwrap(),
        eth: deployed_contracts.remove(ETH_NAME).unwrap(),
        usdc: deployed_contracts.remove(USDC_NAME).unwrap(),
        usdt: deployed_contracts.remove(USDT_NAME).unwrap(),
        ka: deployed_contracts.remove(KA_NAME).unwrap(),
    };
    let eth_bridge_committee =
        EthBridgeCommittee::new(contracts.bridge_committee, eth_signer.clone().into());
    for (i, (m, s)) in committee_members
        .iter()
        .zip(committee_member_stake.iter())
        .enumerate()
    {
        let eth_address = EthAddress::from_str(m).unwrap();
        assert_eq!(
            eth_bridge_committee
                .committee_index(eth_address)
                .await
                .unwrap(),
            i as u8
        );
        assert_eq!(
            eth_bridge_committee
                .committee_stake(eth_address)
                .await
                .unwrap(),
            *s as u16
        );
        assert!(!eth_bridge_committee.blocklist(eth_address).await.unwrap());
    }
    contracts
}

#[derive(Debug)]
pub struct EthBridgeEnvironment {
    pub rpc_url: String,
    process: Child,
    contracts: Option<DeployedSolContracts>,
}

impl EthBridgeEnvironment {
    async fn new(anvil_url: &str, anvil_port: u16) -> anyhow::Result<EthBridgeEnvironment> {
        // Start eth node with anvil
        let eth_environment_process = std::process::Command::new("anvil")
            .arg("--port")
            .arg(anvil_port.to_string())
            .arg("--block-time")
            .arg("1") // 1 second block time
            .arg("--slots-in-an-epoch")
            .arg("3") // 3 slots in an epoch
            .spawn()
            .expect("Failed to start anvil");

        Ok(EthBridgeEnvironment {
            rpc_url: anvil_url.to_string(),
            process: eth_environment_process,
            contracts: None,
        })
    }

    pub(crate) async fn get_signer(
        &self,
        private_key: &str,
    ) -> anyhow::Result<(EthSigner, String)> {
        let signer = get_eth_signer_client(&self.rpc_url, private_key).await?;
        Ok((signer, private_key.to_string()))
    }

    pub(crate) fn contracts(&self) -> &DeployedSolContracts {
        self.contracts.as_ref().unwrap()
    }

    pub(crate) fn get_bridge_config(
        &self,
    ) -> EthBridgeConfig<ethers::prelude::Provider<ethers::providers::Http>> {
        let provider = Arc::new(
            ethers::prelude::Provider::<ethers::providers::Http>::try_from(&self.rpc_url)
                .unwrap()
                .interval(std::time::Duration::from_millis(2000)),
        );
        EthBridgeConfig::new(self.contracts().bridge_config, provider.clone())
    }

    pub(crate) async fn get_supported_token(&self, token_id: u8) -> (EthAddress, u8, u64) {
        let config = self.get_bridge_config();
        let token_address = config.token_address_of(token_id).call().await.unwrap();
        let token_iota_decimal = config.token_iota_decimal_of(token_id).call().await.unwrap();
        let token_price = config.token_price_of(token_id).call().await.unwrap();
        (token_address, token_iota_decimal, token_price)
    }
}

impl Drop for EthBridgeEnvironment {
    fn drop(&mut self) {
        self.process.kill().unwrap();
    }
}

pub(crate) async fn start_bridge_cluster(
    test_cluster: &TestCluster,
    eth_environment: &EthBridgeEnvironment,
    approved_governance_actions: Vec<Vec<BridgeAction>>,
) -> Vec<JoinHandle<()>> {
    let bridge_authority_keys = test_cluster
        .bridge_authority_keys
        .as_ref()
        .unwrap()
        .iter()
        .map(|k| k.copy())
        .collect::<Vec<_>>();
    let bridge_server_ports = test_cluster.bridge_server_ports.as_ref().unwrap();
    assert_eq!(bridge_authority_keys.len(), bridge_server_ports.len());
    assert_eq!(
        bridge_authority_keys.len(),
        approved_governance_actions.len()
    );

    let eth_bridge_contract_address = eth_environment
        .contracts
        .as_ref()
        .unwrap()
        .iota_bridge_address_hex();

    let mut handles = vec![];
    for (i, ((kp, server_listen_port), approved_governance_actions)) in bridge_authority_keys
        .iter()
        .zip(bridge_server_ports.iter())
        .zip(approved_governance_actions.into_iter())
        .enumerate()
    {
        // prepare node config (server + client)
        let tmp_dir = tempdir().unwrap().into_path().join(i.to_string());
        std::fs::create_dir_all(tmp_dir.clone()).unwrap();
        let db_path = tmp_dir.join("client_db");
        // write authority key to file
        let authority_key_path = tmp_dir.join("bridge_authority_key");
        let base64_encoded = kp.encode_base64();
        std::fs::write(authority_key_path.clone(), base64_encoded).unwrap();

        let config = BridgeNodeConfig {
            server_listen_port: *server_listen_port,
            metrics_port: get_available_port("127.0.0.1"),
            bridge_authority_key_path: authority_key_path,
            approved_governance_actions,
            run_client: true,
            db_path: Some(db_path),
            eth: EthConfig {
                eth_rpc_url: eth_environment.rpc_url.clone(),
                eth_bridge_proxy_address: eth_bridge_contract_address.clone(),
                eth_bridge_chain_id: BridgeChainId::EthCustom as u8,
                eth_contracts_start_block_fallback: Some(0),
                eth_contracts_start_block_override: None,
            },
            iota: IotaConfig {
                iota_rpc_url: test_cluster.fullnode_handle.rpc_url.clone(),
                iota_bridge_chain_id: BridgeChainId::IotaCustom as u8,
                bridge_client_key_path: None,
                bridge_client_gas_object: None,
                iota_bridge_module_last_processed_event_id_override: None,
            },
        };
        // Spawn bridge node in memory
        let config_clone = config.clone();
        handles.push(
            run_bridge_node(
                config_clone,
                BridgeNodePublicMetadata::empty_for_testing(),
                Registry::new(),
            )
            .await
            .unwrap(),
        );
    }
    handles
}

pub(crate) async fn get_signatures(
    iota_bridge_client: &IotaBridgeClient,
    nonce: u64,
    iota_chain_id: u8,
) -> Vec<Bytes> {
    let sigs = iota_bridge_client
        .get_token_transfer_action_onchain_signatures_until_success(iota_chain_id, nonce)
        .await
        .unwrap();

    sigs.into_iter()
        .map(|sig: Vec<u8>| Bytes::from(sig))
        .collect()
}

pub(crate) async fn send_eth_tx_and_get_tx_receipt<B, M, D>(
    call: FunctionCall<B, M, D>,
) -> TransactionReceipt
where
    M: Middleware,
    B: std::borrow::Borrow<M>,
    D: ethers::abi::Detokenize,
{
    call.send().await.unwrap().await.unwrap().unwrap()
}

/// A simple struct to create a temporary directory that
/// will be removed when it goes out of scope.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(dir_path: &Path) -> std::io::Result<TempDir> {
        DirBuilder::new().recursive(true).create(dir_path)?;
        Ok(TempDir {
            path: dir_path.to_path_buf(),
        })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
