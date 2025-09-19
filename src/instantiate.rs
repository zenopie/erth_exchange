// src/execute/instantiate.rs
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdResult, Uint128, to_binary,
    CosmosMsg, WasmMsg,};
use secret_toolkit::snip20;

use crate::msg::InstantiateMsg;
use crate::state::{Config, STATE, CONFIG, State, SSCRT_TOKEN_CONTRACT, SSCRT_TOKEN_HASH};

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
    let sscrt_token_contract = deps.api.addr_validate(&msg.sscrt_token_contract)?;


    let config = Config {
        contract_manager,
        erth_token_contract,
        erth_token_hash: msg.erth_token_hash.clone(),
        anml_token_contract,
        anml_token_hash: msg.anml_token_hash.clone(),
        allocation_contract: allocation_contract_addr,
        allocation_hash: msg.allocation_hash.clone(),
        unbonding_seconds: msg.unbonding_seconds,
        unbonding_window: msg.unbonding_window,
        protocol_fee: Uint128::from(50u32),
    };

    let state = State {
        erth_burned: Uint128::zero(),
        anml_burned: Uint128::zero(),
        pending_reward: Uint128::zero(),
    };

    CONFIG.save(deps.storage, &config)?;
    STATE.save(deps.storage, &state)?;
    SSCRT_TOKEN_CONTRACT.save(deps.storage, &sscrt_token_contract)?;
    SSCRT_TOKEN_HASH.save(deps.storage, &msg.sscrt_token_hash)?;

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