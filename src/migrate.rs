// src/migrate.rs
use cosmwasm_std::{DepsMut, Env, Response, StdResult, to_binary, CosmosMsg, WasmMsg,
    StdError, Addr, Uint128,
};

use crate::msg::MigrateMsg;
use crate::state::{Config, CONFIG, POOL_INFO, load_contracts};

use schemars::JsonSchema;
use secret_toolkit::snip20;
use secret_toolkit_storage::Item;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct OldConfig {
    pub contract_manager: Addr,
    pub erth_token_contract: Addr,
    pub erth_token_hash: String,
    pub anml_token_contract: Addr,
    pub anml_token_hash: String,
    pub allocation_contract: Addr,
    pub allocation_hash: String,
    pub unbonding_seconds: u64,
    pub unbonding_window: u64,
    pub protocol_fee: Uint128,
}

// Use the same storage key as CONFIG to read old format
pub static OLD_CONFIG: Item<OldConfig> = Item::new(b"config");

fn register_all_tokens(deps: &DepsMut, env: &Env, config: &Config) -> Result<Vec<CosmosMsg>, StdError> {
    let addrs = load_contracts(&deps.as_ref(), config)?;
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
        contract_addr: addrs.erth_token.address.to_string(),
        code_hash: addrs.erth_token.code_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });
    messages.push(erth_register);

    // Register receive for ANML token
    let anml_register = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: addrs.anml_token.address.to_string(),
        code_hash: addrs.anml_token.code_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });
    messages.push(anml_register);

    Ok(messages)
}

pub fn perform_migration(
    deps: DepsMut,
    env: Env,
    msg: MigrateMsg,
) -> StdResult<Response> {
    match msg {
        MigrateMsg::Migrate { registry_contract, registry_hash } =>
            migrate_to_registry(deps, env, registry_contract, registry_hash),
        MigrateMsg::Upgrade {} => {
            Ok(Response::new().add_attribute("action", "upgrade"))
        }
    }
}

fn migrate_to_registry(
    deps: DepsMut,
    env: Env,
    registry_contract: String,
    registry_hash: String,
) -> Result<Response, StdError> {
    // Load old config from storage (same storage key, different shape)
    let old_config: OldConfig = OLD_CONFIG.load(deps.storage)?;

    let registry_addr = deps.api.addr_validate(&registry_contract)?;

    let new_config = Config {
        contract_manager: old_config.contract_manager,
        registry_contract: registry_addr,
        registry_hash: registry_hash.clone(),
        unbonding_seconds: old_config.unbonding_seconds,
        unbonding_window: old_config.unbonding_window,
        protocol_fee: old_config.protocol_fee,
    };

    CONFIG.save(deps.storage, &new_config)?;

    // Register receive for all tokens using the new registry
    let messages = register_all_tokens(&deps, &env, &new_config)?;

    Ok(Response::new()
        .add_attribute("action", "migrate_to_registry")
        .add_attribute("status", "success")
        .add_attribute("registry_contract", registry_contract)
        .add_attribute("registry_hash", registry_hash)
        .add_messages(messages))
}
