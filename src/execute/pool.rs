use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdError, Uint128, Addr, to_binary,
    CosmosMsg, StdResult, SubMsgResponse, Reply, SubMsg, WasmMsg, };
use secret_toolkit::snip20;

use crate::state::{CONFIG, PoolInfo, POOL_INFO, PENDING_POOL,
    PoolConfig, PoolState,};
use crate::msg::{Snip20InstantiateMsg, InitConfig,};
use crate::INSTANTIATE_LP_TOKEN_REPLY_ID;



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
        total_staked: Uint128::zero(),
        reward_per_token_scaled: Uint128::zero(),
        daily_volumes: [Uint128::zero(); 8],
        daily_rewards: [Uint128::zero(); 8],
        last_updated_day: 0,
        token_erth_reserve: Uint128::zero(),
        token_b_reserve: Uint128::zero(),
    };

    let pool_config = PoolConfig {
        token_b_contract: pool_addr.clone(),
        token_b_hash: hash.clone(),
        token_b_symbol: symbol.clone(),
        lp_token_contract: Addr::unchecked(""),
        lp_token_hash: config.lp_token_hash.clone(),
    };

    let pool_info = PoolInfo {
        state: pool_state,
        config: pool_config,
    };

    let lp_token_name = format!("ERTH-{} Earth Exchange LP Token", symbol);
    let lp_token_symbol = format!("{}LP", symbol);

    let init_config = InitConfig {
        public_total_supply: Some(true),
        enable_deposit: Some(false),
        enable_redeem: Some(false),
        enable_mint: Some(true),
        enable_burn: Some(true),
        can_modify_denoms: Some(false),
    };

    // Construct the SNIP-20 instantiation message
    let lp_token_instantiate_msg = Snip20InstantiateMsg {
        name: lp_token_name.clone(),
        admin: Some(env.contract.address.to_string()), // Use the validated address
        symbol: lp_token_symbol,
        decimals: 6,
        initial_balances: None,
        prng_seed: to_binary(&env.block.time.seconds())?,
        config: Some(init_config),
        supported_denoms: None,
    };

    // Instantiate the LP token contract
    let lp_token_msg = WasmMsg::Instantiate {
        admin: Some(config.contract_manager.to_string()), 
        code_id: config.lp_token_code_id,
        code_hash: config.lp_token_hash.clone(),
        msg: to_binary(&lp_token_instantiate_msg)?,
        funds: vec![],
        label: lp_token_name,
    };

    // Submessage for LP token instantiation
    let sub_msg_lp = SubMsg::reply_on_success(CosmosMsg::Wasm(lp_token_msg), INSTANTIATE_LP_TOKEN_REPLY_ID);


    // Save the new PoolInfo in POOL_INFO using the pool address as the key
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
    PENDING_POOL.save(deps.storage, &pool_addr)?;

    Ok(Response::new()
        .add_submessage(sub_msg_lp)
        .add_attribute("action", "add_pool")
        .add_attribute("pool_address", pool_addr.to_string())
        .add_attribute("lp_token_hash", config.lp_token_hash))
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


pub fn handle_instantiate_lp_token_reply(
    deps: DepsMut,
    env: Env,
    msg: Reply,
) -> StdResult<Response> {

    //let config = CONFIG.load(deps.storage)?;
    let pending_pool = PENDING_POOL.load(deps.storage)?;

    // Load the existing PoolInfo
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pending_pool)
        .ok_or_else(|| StdError::generic_err("Pool not found"))?;

    // Extract the SubMsgExecutionResponse from the reply
    let res: SubMsgResponse = msg.result.unwrap();

    // Find the event that contains the contract address
    let contract_address_event = res
        .events
        .iter()
        .find(|event| event.ty == "instantiate");

    // Ensure we found the instantiate event
    let contract_address_event = match contract_address_event {
        Some(event) => event,
        None => return Err(StdError::generic_err("Failed to find instantiate event")),
    };

    // Find the attribute that contains the contract address
    let contract_address_attr = contract_address_event
        .attributes
        .iter()
        .find(|attr| attr.key == "contract_address");

    // Ensure we found the contract address attribute
    let contract_address = match contract_address_attr {
        Some(attr) => &attr.value,
        None => return Err(StdError::generic_err("Failed to find contract address")),
    };

    // Validate the contract address
    let lp_token_contract_addr = deps.api.addr_validate(contract_address)?;

    pool_info.config.lp_token_contract = lp_token_contract_addr.clone();
    // Save the updated PoolInfo back to storage
    POOL_INFO.insert(deps.storage, &pending_pool, &pool_info)?;


    // Register this contract as a receiver for the LP token
    let register_lp_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: lp_token_contract_addr.to_string(),
        code_hash: pool_info.config.lp_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,  // Optional padding
        })?,
        funds: vec![],
    });

    // Register the contract as a receiver for the B token
    let register_b_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pending_pool.to_string(),
        code_hash: pool_info.config.token_b_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });


    Ok(Response::new()
        .add_message(register_lp_msg)
        .add_message(register_b_msg)
        .add_attribute("action", "instantiate_lp_token")
        .add_attribute("lp_token_contract", lp_token_contract_addr.to_string()))
}
