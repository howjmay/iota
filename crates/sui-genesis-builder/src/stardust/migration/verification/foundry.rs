// Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use iota_sdk::types::block::output::FoundryOutput;
use sui_types::in_memory_storage::InMemoryStorage;

use super::{created_objects::CreatedObjects, util::verify_parent};

pub fn verify_foundry_output(
    output: &FoundryOutput,
    _created_objects: &CreatedObjects,
    storage: &InMemoryStorage,
) -> anyhow::Result<()> {
    // TODO: Implementation. Returns Ok for now so the migration can be tested.
    verify_parent(
        output
            .unlock_conditions()
            .immutable_alias_address()
            .expect("foundry outputs always have an immutable alias address")
            .address(),
        storage,
    )?;
    Ok(())
}
