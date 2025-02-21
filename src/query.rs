// src/query/mod.rs
use cosmwasm_std::{Deps, Env, Binary, StdResult, to_binary, Uint128 };
use crate::msg::{QueryMsg, UserInfoResponse};
use crate::state::{STATE, State, Config, CONFIG, UserInfo, USER_INFO, POOL_INFO, PoolInfo,
    UNBONDING_REQUESTS, UnbondRecord,
    };
use crate::execute::{update_user_rewards};


pub fn query_dispatch(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::QueryState {} => to_binary(&query_state(deps)?),
        QueryMsg::QueryConfig {} => to_binary(&query_config(deps)?),
        QueryMsg::QueryPoolInfo {pools} => to_binary(&query_pool_info(deps, pools)?),
        QueryMsg::QueryUserInfo {pools, user} => to_binary(&query_user_info(deps, pools, user)?),
        QueryMsg::QueryUnbondingRequests { pool, user } => {
            to_binary(&query_unbonding_requests(deps, pool, user)?)
        },
    }
}

fn query_state(deps: Deps) -> StdResult<State> {
    let state = STATE.load(deps.storage)?;
    Ok(state)
}

fn query_config(deps: Deps) -> StdResult<Config> {
    let config = CONFIG.load(deps.storage)?;
    Ok(config)
}

fn query_pool_info(
    deps: Deps,
    pools: Vec<String>,
) -> StdResult<Vec<PoolInfo>> {

    let mut results = vec![];

    for pool_str in pools {
        let pool_addr = deps.api.addr_validate(&pool_str)?;

        let pool_info = match POOL_INFO.get(deps.storage, &pool_addr) {
            Some(info) => info,
            None => {
                // If pool not found, skip or return an empty result for this pool.
                // You could also choose to continue or return an error.
                continue;
            }
        };

        results.push(pool_info);
    }

    Ok(results)
}

fn query_user_info(
    deps: Deps,
    pools: Vec<String>,
    user: String,
) -> StdResult<Vec<UserInfoResponse>> {
    let user_addr = deps.api.addr_validate(&user)?;

    let mut results = vec![];

    for pool_str in pools {
        let pool_addr = deps.api.addr_validate(&pool_str)?;

        let pool_info = match POOL_INFO.get(deps.storage, &pool_addr) {
            Some(info) => info,
            None => {
                // If pool not found, skip or return an empty result for this pool.
                // You could also choose to continue or return an error.
                continue;
            }
        };

        let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());
        // Default to zeroed user info if not found
        let mut user_info = user_info_by_pool
            .get(deps.storage, &user_addr)
            .unwrap_or(UserInfo {
                amount_staked: Uint128::zero(),
                reward_debt: Uint128::zero(),
                pending_rewards: Uint128::zero(),
            });

        // Only update rewards if user has any stake or pending rewards to begin with
        if user_info.amount_staked > Uint128::zero() || user_info.pending_rewards > Uint128::zero() {
            update_user_rewards(&pool_info, &mut user_info)?;
        }

        results.push(UserInfoResponse {
            pool_info: pool_info,
            user_info: user_info,
        });
    }

    Ok(results)
}


fn query_unbonding_requests(
    deps: Deps,
    pool: String,
    user: String,
) -> StdResult<Vec<UnbondRecord>> {
    let pool_addr = deps.api.addr_validate(&pool)?;
    let user_addr = deps.api.addr_validate(&user)?;

    let unbonding_by_pool = UNBONDING_REQUESTS.add_suffix(pool_addr.as_bytes());
    let records = unbonding_by_pool
        .get(deps.storage, &user_addr)
        .unwrap_or_default();

    Ok(records)
}
