// src/migrate.rs
use cosmwasm_std::{DepsMut, Env, Response, StdResult,};

use crate::msg::MigrateMsg;



pub fn perform_migration(
    deps: DepsMut, 
    env: Env, 
    msg: MigrateMsg,
) -> StdResult<Response> {
    match msg {
        MigrateMsg::Migrate {} => migrate_state(deps, env),
    }
}

fn migrate_state(_deps: DepsMut, _env: Env) -> StdResult<Response> {
    

    Ok(Response::new()
        .add_attribute("action", "migrate")
        .add_attribute("status", "success"))
}
