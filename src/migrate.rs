// src/migrate.rs
use cosmwasm_std::{DepsMut, Env, Response, StdResult, to_binary, CosmosMsg, WasmMsg,
    StdError,
};

use crate::msg::MigrateMsg;
use crate::state::{CONFIG, POOL_INFO, SSCRT_TOKEN_CONTRACT, SSCRT_TOKEN_HASH};

use secret_toolkit::snip20;

fn register_all_tokens(deps: &DepsMut, env: &Env) -> Result<Vec<CosmosMsg>, StdError> {
    let config = CONFIG.load(deps.storage)?;
    let mut messages: Vec<CosmosMsg> = vec![];

    // Register receive for all pool tokens
    let items: Vec<_> = POOL_INFO
        .iter(deps.storage)?
        .collect::<Result<Vec<_>, _>>()?;

    for (pool_addr, pool_info) in items {
        let register_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_addr.to_string(),
            code_hash: pool_info.config.token_b_hash.clone(),
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

    Ok(messages)
}

pub fn perform_migration(
    deps: DepsMut,
    env: Env,
    msg: MigrateMsg,
) -> StdResult<Response> {
    match msg {
        MigrateMsg::InitializeSscrt {} =>
            initialize_sscrt_migration(deps, env),
    }
}


fn initialize_sscrt_migration(
    deps: DepsMut,
    env: Env,
) -> Result<Response, StdError> {
    let sscrt_token_contract = "secret1k0jntykt7e4g3y88ltc60czgjuqdy4c9e8fzek";
    let sscrt_token_hash = "af74387e276be8874f07bec3a87023ee49b0e7ebe08178c49d0a49c3c98ed60e";

    let sscrt_addr = deps.api.addr_validate(sscrt_token_contract)?;

    SSCRT_TOKEN_CONTRACT.save(deps.storage, &sscrt_addr)?;
    SSCRT_TOKEN_HASH.save(deps.storage, &sscrt_token_hash.to_string())?;

    // Register receive for all tokens including the new sScrt
    let messages = register_all_tokens(&deps, &env)?;

    Ok(Response::new()
        .add_attribute("action", "initialize_sscrt_migration")
        .add_attribute("status", "success")
        .add_attribute("sscrt_contract", sscrt_token_contract)
        .add_attribute("sscrt_hash", sscrt_token_hash)
        .add_messages(messages))
}