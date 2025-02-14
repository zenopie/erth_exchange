use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdResult, StdError, Uint128, to_binary,
    CosmosMsg, WasmMsg, SubMsg};
use secret_toolkit::snip20;

use crate::state::{CONFIG, STATE, PoolInfo, POOL_INFO, UserInfo, USER_INFO, State};
use crate::msg::{SendMsg};
use crate::execute::SCALING_FACTOR;
use crate::POOL_REWARDS_UPDATE_REPLY_ID;


pub fn claim_rewards(
    deps: DepsMut,
    info: MessageInfo,
    pools: Vec<String>,
) -> StdResult<Response> {
    let state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let mut total_rewards = Uint128::zero();

    for pool_addr_str in pools.iter() {
        let pool_addr = deps.api.addr_validate(pool_addr_str)?;

        let pool_info = POOL_INFO
            .get(deps.storage, &pool_addr)
            .ok_or_else(|| StdError::generic_err("Pool info not found"))?;
        let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());

        let mut user_info = user_info_by_pool
            .get(deps.storage, &info.sender)
            .ok_or_else(|| StdError::generic_err("User info not found"))?;

        update_user_rewards(&pool_info, &mut user_info)?;

        let amount_to_claim = user_info.pending_rewards;
        total_rewards += amount_to_claim;
        user_info.pending_rewards = Uint128::zero();

        user_info_by_pool.insert(deps.storage, &info.sender, &user_info)?;
        POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
    }

    STATE.save(deps.storage, &state)?;

    let transfer_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.erth_token_contract.to_string(),
        code_hash: config.erth_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::Transfer {
            recipient: info.sender.to_string(),
            amount: total_rewards,
            padding: None,
            memo: None,
        })?,
        funds: vec![],
    });

    let allocation_claim_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.allocation_contract.to_string(),
        code_hash: config.allocation_hash.clone(),
        msg: to_binary(&SendMsg::ClaimAllocation {
            allocation_id: 1,
        })?,
        funds: vec![],
    });

    Ok(Response::new()
        .add_attribute("action", "claim_rewards_and_allocation_multi")
        .add_attribute("total_claimed", total_rewards.to_string())
        .add_message(transfer_msg)
        .add_message(allocation_claim_msg))
}



pub fn update_user_rewards(
    pool_info: &PoolInfo,
    user_info: &mut UserInfo,
) -> StdResult<()> {

    // Calculate the pending rewards for the user
    let pending_reward = (user_info.amount_staked * pool_info.state.reward_per_token_scaled) / SCALING_FACTOR - user_info.reward_debt;

    // Update the user's pending rewards
    user_info.pending_rewards += pending_reward;

    // Update the user's reward debt to the current state of the pool
    user_info.reward_debt = (user_info.amount_staked * pool_info.state.reward_per_token_scaled) / SCALING_FACTOR;

    Ok(())
}

pub fn pool_rewards_upkeep(
    deps: DepsMut,
    env: Env,
    state: &mut State,
) -> Result<(), StdError> {
    if state.pending_reward.is_zero() {
        return Ok(());
    }

    let mut pools = Vec::new();
    let mut total_volume = Uint128::zero();
    let current_day = env.block.time.seconds() / 86400;

    let mut iter = POOL_INFO.iter(deps.storage)?;
    while let Some(item_result) = iter.next() {
        let (addr, pool_info) = item_result?;
        total_volume += pool_info.state.pending_volume;
        pools.push((addr, pool_info));
    }

    if !total_volume.is_zero() {
        let reward_pool = state.pending_reward;
        for (addr, mut pool_info) in pools {
            let days_passed = current_day.saturating_sub(pool_info.state.last_updated_day);
            if days_passed > 0 {
                if days_passed >= 7 {
                    pool_info.state.daily_rewards = [Uint128::zero(); 7];
                } else {
                    for _ in 0..days_passed {
                        pool_info.state.daily_rewards.rotate_right(1);
                        pool_info.state.daily_rewards[0] = Uint128::zero();
                    }
                }
                pool_info.state.last_updated_day = current_day;
            }

            if !pool_info.state.pending_volume.is_zero() {
                let pool_share = pool_info
                    .state
                    .pending_volume
                    .multiply_ratio(reward_pool, total_volume);

                if !pool_info.state.total_staked.is_zero() {
                    let increment = pool_share
                        .saturating_mul(SCALING_FACTOR)
                        .checked_div(pool_info.state.total_staked)
                        .unwrap_or(Uint128::zero());
                    pool_info.state.reward_per_token_scaled += increment;
                }
                pool_info.state.daily_rewards[0] += pool_share;
            }

            pool_info.state.pending_volume = Uint128::zero();
            POOL_INFO.insert(deps.storage, &addr, &pool_info)?;
        }
        state.pending_reward = Uint128::zero();
    }

    Ok(())
}


pub fn update_pool_rewards(
    deps: DepsMut,
    info: MessageInfo,
) -> StdResult<Response> {
    // Load config to get the allocation contract info
    let config = CONFIG.load(deps.storage)?;

    // Submessage to claim allocation #1
    let allocation_claim_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.allocation_contract.to_string(),
        code_hash: config.allocation_hash.clone(),
        msg: to_binary(&SendMsg::ClaimAllocation {
            allocation_id: 1,
        })?,
        funds: vec![],
    });

    // Wrap as SubMsg, reply_on_success with an ID
    let sub_msg = SubMsg::reply_on_success(allocation_claim_msg, POOL_REWARDS_UPDATE_REPLY_ID);

    Ok(Response::new()
        .add_submessage(sub_msg)
        .add_attribute("action", "manual_update_pool_rewards_initiated")
        .add_attribute("sender", info.sender))
}

pub fn handle_pool_rewards_update_reply(mut deps: DepsMut, env: Env) -> StdResult<Response> {
    let mut state = STATE.load(deps.storage)?;
    pool_rewards_upkeep(deps.branch(), env, &mut state)?;
    STATE.save(deps.storage, &state)?;
    Ok(Response::new().add_attribute("action", "manual_update_pool_rewards_complete"))
}