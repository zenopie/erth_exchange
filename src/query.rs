// src/query/mod.rs
use cosmwasm_std::{Deps, Env, Binary, StdResult, to_binary, Uint128, StdError };
use crate::msg::{QueryMsg, UserInfoResponse, SimulateSwapResponse};
use crate::state::{STATE, State, Config, CONFIG, UserInfo, USER_INFO, POOL_INFO, PoolInfo,
    UNBONDING_REQUESTS, UnbondRecord,
    };
use crate::execute::{update_user_rewards, calculate_swap};


pub fn query_dispatch(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::QueryState {} => to_binary(&query_state(deps)?),
        QueryMsg::QueryConfig {} => to_binary(&query_config(deps)?),
        QueryMsg::QueryPoolInfo {pools} => to_binary(&query_pool_info(deps, pools)?),
        QueryMsg::QueryUserInfo {pools, user} => to_binary(&query_user_info(deps, pools, user)?),
        QueryMsg::QueryUnbondingRequests { pool, user } => {
            to_binary(&query_unbonding_requests(deps, pool, user)?)
        },
        QueryMsg::SimulateSwap {
            input_token,
            amount,
            output_token,
        } => to_binary(&simulate_swap(deps, input_token, amount, output_token)?),
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


fn simulate_swap(
    deps: Deps,
    input_token_str: String,
    amount: Uint128,
    output_token_str: String,
) -> StdResult<SimulateSwapResponse> {
    let config = CONFIG.load(deps.storage)?;
    let input_token = deps.api.addr_validate(&input_token_str)?;
    let output_token = deps.api.addr_validate(&output_token_str)?;

    let mut total_fee = Uint128::zero();
    let mut intermediate_amount = Uint128::zero();

    // Instead of initializing to zero up front...
    let final_output_amount;

    if input_token != config.erth_token_contract && output_token != config.erth_token_contract {
        // double swap
        let mut input_pool_info = POOL_INFO
            .get(deps.storage, &input_token)
            .ok_or_else(|| StdError::generic_err("No pool found for input token"))?;
        let (fee_step1, inter, _vol1) =
            calculate_swap(&config, &mut input_pool_info, amount, &input_token)?;
        intermediate_amount = inter;
        total_fee += fee_step1;

        let mut output_pool_info = POOL_INFO
            .get(deps.storage, &output_token)
            .ok_or_else(|| StdError::generic_err("No pool found for output token"))?;
        let (fee_step2, final_out, _vol2) = calculate_swap(
            &config,
            &mut output_pool_info,
            inter,
            &config.erth_token_contract
        )?;
        final_output_amount = final_out; // <-- assigned here
        total_fee += fee_step2;
    } else {
        // single swap
        if input_token == config.erth_token_contract {
            // ERTH in -> token out
            let mut pool_info = POOL_INFO
                .get(deps.storage, &output_token)
                .ok_or_else(|| StdError::generic_err("No pool found for token"))?;
            let (fee, out, _vol) =
                calculate_swap(&config, &mut pool_info, amount, &config.erth_token_contract)?;
            final_output_amount = out;  // <-- assigned here
            total_fee += fee;
        } else {
            // token in -> ERTH out
            let mut pool_info = POOL_INFO
                .get(deps.storage, &input_token)
                .ok_or_else(|| StdError::generic_err("No pool found for token"))?;
            let (fee, out, _vol) =
                calculate_swap(&config, &mut pool_info, amount, &input_token)?;
            final_output_amount = out;  // <-- assigned here
            total_fee += fee;
        }
    }

    Ok(SimulateSwapResponse {
        output_amount: final_output_amount,
        intermediate_amount,
        total_fee,
    })
}
