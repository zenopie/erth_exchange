// src/lib.rs
use cosmwasm_std::{entry_point, Binary, Deps, DepsMut, Env, MessageInfo, Response, 
    StdResult, StdError, Reply,};
use crate::execute::{execute_dispatch, handle_instantiate_lp_token_reply, handle_pool_rewards_update_reply};

use crate::query::query_dispatch;
use crate::migrate::perform_migration;
use crate::instantiate::perform_instantiate;

pub mod msg;
pub mod state;
pub mod execute;
pub mod query;
pub mod migrate;
pub mod instantiate;

const INSTANTIATE_LP_TOKEN_REPLY_ID: u64 = 0;
const POOL_REWARDS_UPDATE_REPLY_ID: u64 = 1;

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: msg::InstantiateMsg,
) -> StdResult<Response> {
    perform_instantiate(deps, env, info, msg)
}

#[entry_point]
pub fn execute(
    deps: DepsMut, 
    env: Env, 
    info: MessageInfo, 
    msg: msg::ExecuteMsg
) -> StdResult<Response> {
    execute_dispatch(deps, env, info, msg)
}

#[entry_point]
pub fn migrate(
    deps: DepsMut, 
    env: Env, 
    msg: msg::MigrateMsg
) -> StdResult<Response> {
    perform_migration(deps, env, msg)
}

#[entry_point]
pub fn query(
    deps: Deps, 
    env: Env, 
    msg: msg::QueryMsg
) -> StdResult<Binary> {
    query_dispatch(deps, env, msg)
}

#[entry_point]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> StdResult<Response> {
    match msg.id {
        INSTANTIATE_LP_TOKEN_REPLY_ID => handle_instantiate_lp_token_reply(deps, env, msg),
        POOL_REWARDS_UPDATE_REPLY_ID => handle_pool_rewards_update_reply(deps, env),
        _ => Err(StdError::generic_err("Unknown reply ID")),
    }
}



