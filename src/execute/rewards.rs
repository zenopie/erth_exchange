use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdError, Uint128, Addr, to_binary,
    CosmosMsg, StdResult, WasmMsg, };
use secret_toolkit::snip20;

use crate::state::{CONFIG, STATE, State, PoolInfo, POOL_INFO, UserInfo, USER_INFO};
use crate::msg::{SendMsg};

const SCALING_FACTOR: Uint128 = Uint128::new(1_000_000);

// add test if lp token is the correct token for the pool
pub fn deposit_lp_tokens(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    from: Addr,
    amount: Uint128,
    pool: String,
) -> Result<Response, StdError> {

    let pool_addr = deps.api.addr_validate(&pool)?;

    // Load pool info from storage
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool info not found"))?;
    
    // Access user info for the specific pool
    let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());
    
    // Retrieve user info or initialize a new one using Default if not found
    let mut user_info = user_info_by_pool
        .get(deps.storage, &from)
        .unwrap_or_else(|| UserInfo::default());

    // First, update the user's rewards based on the current amount_staked
    if user_info.amount_staked > Uint128::zero() {
        
        // Update the user's rewards based on the latest pool state
        update_user_rewards(&pool_info, &mut user_info)?;     
    }

    // Now, add the new deposit to the user's staked amount
    user_info.amount_staked += amount;
    pool_info.state.total_staked += amount;

    // Update the user's reward debt to the current state of the pool
    user_info.reward_debt = user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;

    // Save the updated or new user info back to storage
    user_info_by_pool.insert(deps.storage, &from, &user_info)?;
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    Ok(Response::new()
        .add_attribute("action", "deposit")
        .add_attribute("deposit_amount", amount))
}

pub fn withdraw_lp_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    pool: String,
    amount: Uint128,
    _unbond: bool,
) -> StdResult<Response> {
    // Verify pool address
    let pool_addr = deps.api.addr_validate(&pool)?;

    // Load State, PoolInfo, and UserInfo
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool info not found"))?;
    let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());
    let mut user_info = user_info_by_pool
        .get(deps.storage, &info.sender)
        .ok_or_else(|| StdError::generic_err("User info not found"))?;

    // Update the pool's rewards based on the latest allocation
    update_pool_rewards(&env, &mut state, &mut pool_info, None)?;

    // Update the user's rewards based on the latest pool state
    update_user_rewards(&pool_info, &mut user_info)?;

    // Ensure the user has enough staked to withdraw
    if user_info.amount_staked < amount {
        return Err(StdError::generic_err("Insufficient staked amount to withdraw"));
    }

    // Subtract the withdrawal amount from the user's staked amount and pool info
    user_info.amount_staked -= amount;
    pool_info.state.total_staked -= amount;

    // Prepare to transfer pending rewards if they are greater than zero
    let mut messages = Vec::new();
    if user_info.pending_rewards > Uint128::zero() {
        let transfer_rewards_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.erth_token_contract.to_string(),
            code_hash: config.erth_token_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: info.sender.to_string(),
                amount: user_info.pending_rewards,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        });
        messages.push(transfer_rewards_msg);
        user_info.pending_rewards = Uint128::zero();
    }

    // Update the user's reward debt to the current state of the pool
    if user_info.amount_staked.is_zero() {
        // Remove user info if no more staked amount
        user_info_by_pool.remove(deps.storage, &info.sender)?;
    } else {
        user_info.reward_debt = user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;
        // Save the updated user info back to storage
        user_info_by_pool.insert(deps.storage, &info.sender, &user_info)?;
    }

    // Save updated pool info
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
    STATE.save(deps.storage, &state)?;

    // Create transfer message to return the LP tokens to the user
    let transfer_lp_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.lp_token_contract.to_string(),
        code_hash: pool_info.config.lp_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::Transfer {
            recipient: info.sender.to_string(),
            amount,
            padding: None,
            memo: None,
        })?,
        funds: vec![],
    });
    messages.push(transfer_lp_msg);

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "withdraw")
        .add_attribute("withdraw_amount", amount.to_string())
        .add_attribute("rewards_transferred", user_info.pending_rewards.to_string()))
}



pub fn claim_rewards(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    pools: Vec<String>,
) -> StdResult<Response> {
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let mut total_rewards = Uint128::zero();

    for pool_addr_str in pools.iter() {
        let pool_addr = deps.api.addr_validate(pool_addr_str)?;

        let mut pool_info = POOL_INFO
            .get(deps.storage, &pool_addr)
            .ok_or_else(|| StdError::generic_err("Pool info not found"))?;
        let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());

        let mut user_info = user_info_by_pool
            .get(deps.storage, &info.sender)
            .ok_or_else(|| StdError::generic_err("User info not found"))?;

        update_pool_rewards(&env, &mut state, &mut pool_info, None)?;
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






pub fn update_pool_rewards(
    env: &Env,
    state: &mut State,
    pool_info: &mut PoolInfo,
    volume: Option<Uint128>,
) -> StdResult<()> {
    // Convert timestamp to days
    let current_day = env.block.time.seconds() / 86400;

    let mut day_change = false;

    if current_day > state.last_updated_day {
        day_change = true;

        // Shift total volumes and rewards arrays
        state.daily_total_volumes.rotate_right(1);
        state.daily_total_volumes[0] = Uint128::zero();

        state.daily_total_rewards.rotate_right(1);
        state.daily_total_rewards[0] = state.pending_reward;
        state.pending_reward = Uint128::zero();

        // Update last updated day
        state.last_updated_day = current_day;
    }

    // Check if we need to update the pool's day
    if current_day > pool_info.state.last_updated_day {
        day_change = true;

        // Shift daily rewards for the pool
        pool_info.state.daily_rewards.rotate_right(1);
        pool_info.state.daily_rewards[0] = Uint128::zero(); // Reset today's reward

        // Shift daily volumes for the pool
        pool_info.state.daily_volumes.rotate_right(1);
        pool_info.state.daily_volumes[0] = Uint128::zero(); // Reset today's volume

        // Update the pool's last updated day
        pool_info.state.last_updated_day = current_day;
    }

    // If there is a provided volume, add it to today's volume
    if let Some(vol) = volume {
        pool_info.state.daily_volumes[0] += vol;
        state.daily_total_volumes[0] += vol;
    }

    // Only recalculate rewards if a day change occurred
    if day_change {
        // Sum last 7 days of pool volume (excluding today)
        let last_7_days_pool_volume: Uint128 = pool_info.state.daily_volumes[1..8].iter().sum();
        if last_7_days_pool_volume.is_zero() {
            return Ok(()); // No volume in the past 7 days, skip
        }

        // Sum last 7 days of total volume (excluding today)
        let last_7_days_total_volume: Uint128 = state.daily_total_volumes[1..8].iter().sum();
        if last_7_days_total_volume.is_zero() {
            return Ok(()); // No total volume in the past 7 days, skip
        }

        // Calculate pool's share of today's rewards
        let pool_reward_share = (last_7_days_pool_volume * state.daily_total_rewards[0])
            / last_7_days_total_volume;

        // Add calculated rewards to today's pool rewards
        pool_info.state.daily_rewards[0] += pool_reward_share;

        // Update reward per token if the pool has staked liquidity
        if !pool_info.state.total_staked.is_zero() {
            let reward_per_token = (pool_reward_share * SCALING_FACTOR)
                / pool_info.state.total_staked;
            pool_info.state.reward_per_token_scaled += reward_per_token;
        }
    }

    Ok(())
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