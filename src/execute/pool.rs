use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdError, Uint128, to_binary,
    CosmosMsg, StdResult, WasmMsg, };
use secret_toolkit::snip20;

use crate::state::{CONFIG, PoolInfo, POOL_INFO,
    PoolConfig, PoolState,};



pub fn add_pool(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    token: String,
    hash: String,
    symbol: String,
) -> StdResult<Response> {

    // Ensure only the contract manager can add a pool
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.contract_manager {
        return Err(StdError::generic_err("unauthorized"));
    }

    let pool_addr = deps.api.addr_validate(&token)?;

    // Check if the pool already exists using is_some
    if POOL_INFO.get(deps.storage, &pool_addr).is_some() {
        return Err(StdError::generic_err("Pool already exists"));
    }

    // Initialize a new PoolInfo
    let pool_state = PoolState {
        total_shares: Uint128::zero(),
        reward_per_token_scaled: Uint128::zero(),
        erth_reserve: Uint128::zero(),
        token_b_reserve: Uint128::zero(),
        daily_rewards: [Uint128::zero(); 7],
        daily_volumes: [Uint128::zero(); 7],
        last_updated_day: 0,
        unbonding_shares: Uint128::zero(),
    };

    let pool_config = PoolConfig {
        token_b_contract: pool_addr.clone(),
        token_b_hash: hash.clone(),
        token_b_symbol: symbol.clone(),
    };

    let pool_info = PoolInfo {
        state: pool_state,
        config: pool_config,
    };


    // Register this contract as a receiver for the token
    let register_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_addr.to_string(),
        code_hash: hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,  // Optional padding
        })?,
        funds: vec![],
    });


    // Save the new PoolInfo in POOL_INFO using the pool address as the key
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    Ok(Response::new()
        .add_message(register_msg)
        .add_attribute("action", "add_pool")
        .add_attribute("pool_address", pool_addr.to_string()))
}

pub fn update_pool_config(
    deps: DepsMut,
    info: MessageInfo,
    pool: String,
    pool_config: PoolConfig,
) -> StdResult<Response> {
    // Ensure only the contract manager can update the LP token hash
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.contract_manager {
        return Err(StdError::generic_err("unauthorized"));
    }

    let pool_addr = deps.api.addr_validate(&pool)?;

    // Load the existing PoolInfo
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool not found"))?;

    // Update the config
    pool_info.config = pool_config;

    // Save the updated PoolInfo back to storage
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    Ok(Response::new()
        .add_attribute("action", "update_pool_config")
        .add_attribute("pool_address", pool_addr.to_string()))
}


