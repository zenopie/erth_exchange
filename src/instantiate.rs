// src/execute/instantiate.rs
use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdResult, Uint128, to_binary,
    CosmosMsg, WasmMsg,};
use secret_toolkit::snip20;

use crate::msg::InstantiateMsg;
use crate::state::{Config, STATE, CONFIG, State, load_contracts};

pub fn perform_instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let contract_manager = deps.api.addr_validate(&msg.contract_manager)?;
    let registry_contract = deps.api.addr_validate(&msg.registry_contract)?;

    let config = Config {
        contract_manager,
        registry_contract,
        registry_hash: msg.registry_hash.clone(),
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

    // Query registry for contract addresses
    let addrs = load_contracts(&deps.as_ref(), &config)?;

    // Register this contract as a receiver for ERTH
    let register_erth_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: addrs.erth_token.address.to_string(),
        code_hash: addrs.erth_token.code_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });

    let register_anml_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: addrs.anml_token.address.to_string(),
        code_hash: addrs.anml_token.code_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });

    Ok(Response::new()
        .add_message(register_erth_msg)
        .add_message(register_anml_msg)
        .add_attribute("action", "instantiate")
        .add_attribute("erth_token_contract", addrs.erth_token.address.to_string())
        .add_attribute("anml_token_contract", addrs.anml_token.address.to_string()))
}
