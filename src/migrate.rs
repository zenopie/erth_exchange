// src/migrate.rs
use cosmwasm_std::{DepsMut, Env, Response, StdResult, to_binary, CosmosMsg, WasmMsg,
    StdError, Addr, Uint128,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit_storage::Keymap;

use crate::msg::MigrateMsg;
use crate::state::{CONFIG, POOL_INFO};

use secret_toolkit::snip20;

pub fn perform_migration(
    deps: DepsMut, 
    env: Env, 
    msg: MigrateMsg,
) -> StdResult<Response> {
    match msg {
        MigrateMsg::RemoveTotalStaked {} => remove_total_staked_migration(deps, env),
    }
}

// Old struct for migration from total_staked to direct staking
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct OldPoolStateWithStaked {
    pub total_shares: Uint128,
    pub total_staked: Uint128,
    pub reward_per_token_scaled: Uint128,
    pub erth_reserve: Uint128,
    pub token_b_reserve: Uint128,
    pub daily_rewards: [Uint128; 7],
    pub daily_volumes: [Uint128; 7],
    pub last_updated_day: u64,
    pub unbonding_shares: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct OldPoolInfoWithStaked {
    pub state: OldPoolStateWithStaked,
    pub config: crate::state::PoolConfig,
}

static OLD_POOL_INFO_WITH_STAKED: Keymap<Addr, OldPoolInfoWithStaked> = Keymap::new(b"pool_info");

fn remove_total_staked_migration(deps: DepsMut, env: Env) -> Result<Response, StdError> {
    let config = CONFIG.load(deps.storage)?;
    let mut messages: Vec<CosmosMsg> = vec![];

    // Iterate over all old pools and convert to new format
    let items: Vec<_> = OLD_POOL_INFO_WITH_STAKED
        .iter(deps.storage)?
        .collect::<Result<Vec<_>, _>>()?;
    
    let pools_count = items.len();
    
    for (pool_addr, old_pool_info) in items {
        // Convert old pool state to new pool state (removing total_staked field)
        let new_pool_state = crate::state::PoolState {
            total_shares: old_pool_info.state.total_shares,
            reward_per_token_scaled: old_pool_info.state.reward_per_token_scaled,
            erth_reserve: old_pool_info.state.erth_reserve,
            token_b_reserve: old_pool_info.state.token_b_reserve,
            daily_rewards: old_pool_info.state.daily_rewards,
            daily_volumes: old_pool_info.state.daily_volumes,
            last_updated_day: old_pool_info.state.last_updated_day,
            unbonding_shares: old_pool_info.state.unbonding_shares,
        };

        // Create new pool info with updated state
        let new_pool_info = crate::state::PoolInfo {
            state: new_pool_state,
            config: old_pool_info.config,
        };
        
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
        contract_addr: config.erth_token_contract.to_string(),
        code_hash: config.erth_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });
    messages.push(erth_register);

    Ok(Response::new()
        .add_attribute("action", "remove_total_staked_migration")
        .add_attribute("status", "success")
        .add_attribute("pools_migrated", pools_count.to_string())
        .add_messages(messages))
}