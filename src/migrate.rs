// src/migrate.rs
use cosmwasm_std::{DepsMut, Env, Response, StdResult, to_binary, CosmosMsg, WasmMsg,
    StdError, Addr, Uint128,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit_storage::{Item, Keymap};

use crate::msg::MigrateMsg;
use crate::state::{CONFIG, POOL_INFO, PoolState, PoolInfo};

use secret_toolkit::snip20;

// Old struct definitions for migration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct OldConfig {
    pub contract_manager: Addr,
    pub erth_token_contract: Addr,
    pub erth_token_hash: String,
    pub anml_token_contract: Addr,
    pub anml_token_hash: String,
    pub allocation_contract: Addr,
    pub allocation_hash: String,
    pub lp_token_code_id: u64,
    pub lp_token_hash: String,
    pub unbonding_seconds: u64,
    pub protocol_fee: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct OldPoolConfig {
    pub token_b_contract: Addr,
    pub token_b_hash: String,
    pub token_b_symbol: String,
    pub lp_token_contract: Addr,
    pub lp_token_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct OldPoolInfo {
    pub state: PoolState,
    pub config: OldPoolConfig,
}

static OLD_CONFIG: Item<OldConfig> = Item::new(b"config");
static OLD_POOL_INFO: Keymap<Addr, OldPoolInfo> = Keymap::new(b"pool_info");



pub fn perform_migration(
    deps: DepsMut, 
    env: Env, 
    msg: MigrateMsg,
) -> StdResult<Response> {
    match msg {
        MigrateMsg::Migrate {} => migrate_state(deps, env),
    }
}

fn migrate_state(
    deps: DepsMut,
    env: Env,
) -> Result<Response, StdError> {
    let mut messages: Vec<CosmosMsg> = vec![];

    // Load old config and convert to new config
    let old_config = OLD_CONFIG.load(deps.storage)?;
    let new_config = crate::state::Config {
        contract_manager: old_config.contract_manager,
        erth_token_contract: old_config.erth_token_contract,
        erth_token_hash: old_config.erth_token_hash.clone(),
        anml_token_contract: old_config.anml_token_contract,
        anml_token_hash: old_config.anml_token_hash,
        allocation_contract: old_config.allocation_contract,
        allocation_hash: old_config.allocation_hash,
        unbonding_seconds: 604800,
        unbonding_window: 604800, // Default to 7 days (604800 seconds)
        protocol_fee: old_config.protocol_fee,
    };
    CONFIG.save(deps.storage, &new_config)?;

    // Iterate over all old pools and convert to new format
    let items: Vec<_> = OLD_POOL_INFO
        .iter(deps.storage)?
        .collect::<Result<Vec<_>, _>>()?;
    
    let pools_count = items.len();
    
    for (pool_addr, old_pool_info) in items {
        // Convert old pool config to new pool config (removing LP token fields)
        let new_pool_config = crate::state::PoolConfig {
            token_b_contract: old_pool_info.config.token_b_contract,
            token_b_hash: old_pool_info.config.token_b_hash.clone(),
            token_b_symbol: old_pool_info.config.token_b_symbol,
        };

        // Create new pool info with updated config
        let mut new_pool_info = PoolInfo {
            state: old_pool_info.state,
            config: new_pool_config,
        };

        // Set total_staked to match total_shares for direct staking
        new_pool_info.state.total_staked = new_pool_info.state.total_shares;
        
        // Save the converted pool info
        POOL_INFO.insert(deps.storage, &pool_addr, &new_pool_info)?;

        // Register this contract as a receiver for token_b
        let register_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_addr.to_string(),
            code_hash: new_pool_info.config.token_b_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::RegisterReceive {
                code_hash: env.contract.code_hash.clone(),
                padding: None,
            })?,
            funds: vec![],
        });
        messages.push(register_msg);
    }

    // Register receive for ERTH token
    let erth_register = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: new_config.erth_token_contract.to_string(),
        code_hash: new_config.erth_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });
    messages.push(erth_register);

    Ok(Response::new()
        .add_attribute("action", "migrate")
        .add_attribute("status", "success")
        .add_attribute("pools_migrated", pools_count.to_string())
        .add_messages(messages))
}