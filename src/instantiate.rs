// src/execute/instantiate.rs
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdResult, Uint128, to_binary,
    CosmosMsg, WasmMsg,};
use secret_toolkit::snip20;

use crate::msg::InstantiateMsg;
use crate::state::{Config, STATE, CONFIG, State};

pub fn perform_instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let erth_token_contract = deps.api.addr_validate(&msg.erth_token_contract)?;
    let contract_manager = deps.api.addr_validate(&msg.contract_manager)?;
    let anml_token_contract = deps.api.addr_validate(&msg.anml_token_contract)?;
    let allocation_contract_addr = deps.api.addr_validate(&msg.allocation_contract)?;


    let config = Config {
        contract_manager,
        erth_token_contract,
        erth_token_hash: msg.erth_token_hash.clone(),
        anml_token_contract,
        anml_token_hash: msg.erth_token_hash.clone(),
        allocation_contract: allocation_contract_addr,
        allocation_hash: msg.allocation_hash.clone(),
        lp_token_code_id: msg.lp_token_code_id,
        lp_token_hash: msg.lp_token_hash,
        unbonding_seconds: msg.unbonding_seconds,
        protocol_fee: Uint128::from(50u32),
    };

    let state = State {
        erth_burned: Uint128::zero(),
        anml_burned: Uint128::zero(),
        daily_total_volumes: [Uint128::zero(); 8],
        daily_total_rewards: [Uint128::zero(); 8],
        pending_reward: Uint128::zero(),
        last_updated_day: 0,
    };

    CONFIG.save(deps.storage, &config)?;
    STATE.save(deps.storage, &state)?;

    // Register this contract as a receiver for ERTH
    let register_erth_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.erth_token_contract.to_string(),
        code_hash: config.erth_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,  // Optional padding
        })?,
        funds: vec![],
    });

    let register_anml_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.anml_token_contract.to_string(),
        code_hash: config.anml_token_hash,
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,  // Optional padding
        })?,
        funds: vec![],
    });

    Ok(Response::new()
        .add_message(register_erth_msg)
        .add_message(register_anml_msg)
        .add_attribute("action", "instantiate")
        .add_attribute("erth_token_contract", msg.erth_token_contract)
        .add_attribute("erth_token_hash", msg.erth_token_hash))
}