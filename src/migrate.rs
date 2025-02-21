// src/migrate.rs
use cosmwasm_std::{DepsMut, Env, Response, StdResult, to_binary, CosmosMsg, WasmMsg,};

use crate::msg::MigrateMsg;
use crate::state::{CONFIG, POOL_INFO};

use secret_toolkit::snip20;




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
) -> StdResult<Response> {

    let mut messages: Vec<CosmosMsg> = vec![];

    // Iterate over all pools using the Secret Network Toolkit keymap
    let items: Vec<_> = POOL_INFO
    .iter(deps.storage)?
    .collect::<StdResult<Vec<_>>>()?;
    for (pool_addr, info) in items {


        // Register this contract as a receiver for the token at pool_addr
        let register_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_addr.to_string(),
            code_hash: info.config.token_b_hash,
            msg: to_binary(&snip20::HandleMsg::RegisterReceive {
                code_hash: env.contract.code_hash.clone(),
                padding: None,
            })?,
            funds: vec![],
        });
        messages.push(register_msg);

        // Register receive for LP token
        let lp_register = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: info.config.lp_token_contract.to_string(),
            code_hash: info.config.lp_token_hash,
            msg: to_binary(&snip20::HandleMsg::RegisterReceive {
                code_hash: env.contract.code_hash.clone(),
                padding: None,
            })?,
            funds: vec![],
        });
        messages.push(lp_register);
    }

    let config = CONFIG.load(deps.storage)?;

    // Register receive for ERTH token
    let erth_register = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.erth_token_contract.to_string(), // update with actual address
        code_hash: config.erth_token_hash,
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
        .add_messages(messages))
}