use cosmwasm_std::{
    to_binary, CosmosMsg, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Uint128, WasmMsg,
};
use secret_toolkit::{snip20,};

use crate::{
    execute::{update_user_rewards, SCALING_FACTOR},
    state::{
        CONFIG, STATE, POOL_INFO, USER_INFO, UnbondRecord, UNBONDING_REQUESTS,
    },
};


// Simple integer square root for u128
fn sqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = 1;
    while x > y {
        x = (x + y) / 2;
        y = n / x;
    }
    x
}

// -------------------------
// Add Liquidity
// -------------------------
pub fn add_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount_erth: Uint128,
    amount_b: Uint128,
    pool: String,
) -> Result<Response, StdError> {
    let config = CONFIG.load(deps.storage)?;
    let pool_addr = deps.api.addr_validate(&pool)?;

    // Load or error
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool not found"))?;

    // Determine LP shares
    let (shares, adjusted_amount_erth, adjusted_amount_b) =
        if pool_info.state.total_shares.is_zero() {
            // Use square root of product for initial shares
            let product = amount_erth
                .checked_mul(amount_b)?;
            let shares = Uint128::from(sqrt_u128(product.u128()));
            (shares, amount_erth, amount_b)
        } else {
            let share_erth = amount_erth * pool_info.state.total_shares / pool_info.state.erth_reserve;
            let share_b = amount_b * pool_info.state.total_shares / pool_info.state.token_b_reserve;
            let shares = share_erth.min(share_b);
            let adjusted_amount_erth = (shares * pool_info.state.erth_reserve) / pool_info.state.total_shares;
            let adjusted_amount_b = (shares * pool_info.state.token_b_reserve) / pool_info.state.total_shares;
            (shares, adjusted_amount_erth, adjusted_amount_b)
        };

    // Messages: transfer in the adjusted amounts
    let messages = vec![
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.erth_token_contract.to_string(),
            code_hash: config.erth_token_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::TransferFrom {
                owner: info.sender.to_string(),
                recipient: env.contract.address.to_string(),
                amount: adjusted_amount_erth,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        }),
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_info.config.token_b_contract.to_string(),
            code_hash: pool_info.config.token_b_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::TransferFrom {
                owner: info.sender.to_string(),
                recipient: env.contract.address.to_string(),
                amount: adjusted_amount_b,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        }),
    ];

    // Update pool reserves
    pool_info.state.erth_reserve += adjusted_amount_erth;
    pool_info.state.token_b_reserve += adjusted_amount_b;
    pool_info.state.total_shares += shares;


    // Always stake the shares directly
    let user_info_by_pool = USER_INFO.add_suffix(pool_addr.as_bytes());
    let mut user_info = user_info_by_pool
        .get(deps.storage, &info.sender)
        .unwrap_or_default();

    if user_info.amount_staked > Uint128::zero() {
        update_user_rewards(&pool_info, &mut user_info)?;
    }
    user_info.amount_staked += shares;
    pool_info.state.total_staked += shares;
    user_info.reward_debt =
        user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;
    user_info_by_pool.insert(deps.storage, &info.sender, &user_info)?;

    // Save pool
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "add_liquidity")
        .add_attribute("from", info.sender)
        .add_attribute("shares", shares.to_string())
        .add_attribute("adjusted_amount_erth", adjusted_amount_erth.to_string())
        .add_attribute("adjusted_amount_b", adjusted_amount_b.to_string()))
}


// -------------------------
// Remove Liquidity (withdraw staked shares)
// -------------------------
pub fn remove_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    pool: String,
    amount: Uint128,
) -> StdResult<Response> {
    let pool_addr = deps.api.addr_validate(&pool)?;
    let state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool info not found"))?;

    // 1) Load user staking info
    let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());
    let mut user_info = user_info_by_pool
        .get(deps.storage, &info.sender)
        .ok_or_else(|| StdError::generic_err("User info not found"))?;

    // 2) Update user rewards
    update_user_rewards(&pool_info, &mut user_info)?;
    if user_info.amount_staked < amount {
        return Err(StdError::generic_err("Insufficient staked amount"));
    }

    // 3) Decrease staked
    user_info.amount_staked = user_info.amount_staked.checked_sub(amount)?;
    pool_info.state.total_staked = pool_info.state.total_staked.checked_sub(amount)?;

    // 4) Transfer pending rewards if any
    let mut messages: Vec<CosmosMsg> = Vec::new();
    if !user_info.pending_rewards.is_zero() {
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

    // 5) Update or remove user info
    if user_info.amount_staked.is_zero() {
        user_info_by_pool.remove(deps.storage, &info.sender)?;
    } else {
        user_info.reward_debt =
            user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;
        user_info_by_pool.insert(deps.storage, &info.sender, &user_info)?;
    }
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
    STATE.save(deps.storage, &state)?;

    // All withdrawals require unbonding period - create unbond record

    // increment unbonding_shares in the pool
    pool_info.state.unbonding_shares += amount;
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    // Store unbond record in UNBONDING for pool
    let unbonding_by_pool = UNBONDING_REQUESTS.add_suffix(pool_addr.as_bytes());
    let mut unbond_records = unbonding_by_pool
        .get(deps.storage, &info.sender)
        .unwrap_or_default();

    let now = env.block.time.seconds();
    unbond_records.push(UnbondRecord {
        pool: pool_addr,
        amount,
        start_time: now,
    });
    unbonding_by_pool.insert(deps.storage, &info.sender, &unbond_records)?;


    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "remove_liquidity_and_unbond")
        .add_attribute("shares_withdrawn", amount.to_string())
        .add_attribute("claimable_at", (now + config.unbonding_seconds).to_string()))
}





// -------------------------
//  CLAIM Unbond
// -------------------------
pub fn claim_unbond_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    pool: String,
) -> Result<Response, StdError> {
    let user = info.sender.clone();
    let pool_addr = deps.api.addr_validate(&pool)?;
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool not found"))?;

    let config = CONFIG.load(deps.storage)?;
    let now = env.block.time.seconds();

    let unbonding_by_pool = UNBONDING_REQUESTS.add_suffix(pool_addr.as_bytes());
    let records = unbonding_by_pool
        .get(deps.storage, &user)
        .unwrap_or_default();
    if records.is_empty() {
        return Err(StdError::generic_err("No unbond requests found"));
    }

    // Separate into those ready to claim vs still pending
    let required_wait = config.unbonding_seconds;
    let unbonding_window = config.unbonding_window;
    
    // Partition records into ready_to_claim, auto_restake, and still_pending
    let (ready_to_claim, past_ready): (Vec<UnbondRecord>, Vec<UnbondRecord>) =
        records.into_iter()
               .partition(|r| now >= r.start_time + required_wait && now <= r.start_time + required_wait + unbonding_window);
    
    let (auto_restake, still_pending): (Vec<UnbondRecord>, Vec<UnbondRecord>) =
        past_ready.into_iter()
                  .partition(|r| now > r.start_time + required_wait + unbonding_window);

    if ready_to_claim.is_empty() && auto_restake.is_empty() {
        return Err(StdError::generic_err("No unbonding requests are ready yet"));
    }

    // Handle auto-restaking first
    let auto_restake_shares: Uint128 = auto_restake.iter().map(|r| r.amount).sum();
    if !auto_restake_shares.is_zero() {
        // Auto-restake the shares
        let user_info_by_pool = USER_INFO.add_suffix(pool_addr.as_bytes());
        let mut user_info = user_info_by_pool
            .get(deps.storage, &user)
            .unwrap_or_default();

        if user_info.amount_staked > Uint128::zero() {
            update_user_rewards(&pool_info, &mut user_info)?;
        }
        
        user_info.amount_staked += auto_restake_shares;
        pool_info.state.total_staked += auto_restake_shares;
        user_info.reward_debt =
            user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;
        user_info_by_pool.insert(deps.storage, &user, &user_info)?;
        
        // Remove from unbonding_shares
        pool_info.state.unbonding_shares =
            pool_info.state.unbonding_shares.checked_sub(auto_restake_shares)?;
    }

    // Sum up total shares to claim (only ready_to_claim)
    let total_shares: Uint128 = ready_to_claim.iter().map(|r| r.amount).sum();

    // Overwrite storage with only still-pending
    unbonding_by_pool.insert(deps.storage, &user, &still_pending)?;

    // Calculate underlying tokens
    let amount_erth = total_shares * pool_info.state.erth_reserve / pool_info.state.total_shares;
    let amount_b    = total_shares * pool_info.state.token_b_reserve / pool_info.state.total_shares;

    // Decrement unbonding_shares by total_shares
    pool_info.state.unbonding_shares =
        pool_info.state.unbonding_shares.checked_sub(total_shares)?;
    
    // Update pool reserves
    pool_info.state.erth_reserve    = pool_info.state.erth_reserve.checked_sub(amount_erth)?;
    pool_info.state.token_b_reserve = pool_info.state.token_b_reserve.checked_sub(amount_b)?;
    pool_info.state.total_shares    = pool_info.state.total_shares.checked_sub(total_shares)?;
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    let transfer_erth_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.erth_token_contract.to_string(),
        code_hash: config.erth_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::Transfer {
            recipient: user.to_string(),
            amount: amount_erth,
            padding: None,
            memo: None,
        })?,
        funds: vec![],
    });
    let transfer_b_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.token_b_contract.to_string(),
        code_hash: pool_info.config.token_b_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::Transfer {
            recipient: user.to_string(),
            amount: amount_b,
            padding: None,
            memo: None,
        })?,
        funds: vec![],
    });

    let mut response = Response::new()
        .add_attribute("action", "claim_unbonding")
        .add_attribute("user", user)
        .add_attribute("pool", pool_addr);

    if !total_shares.is_zero() {
        response = response
            .add_message(transfer_erth_msg)
            .add_message(transfer_b_msg)
            .add_attribute("total_shares_claimed", total_shares.to_string())
            .add_attribute("erth_returned", amount_erth.to_string())
            .add_attribute("token_b_returned", amount_b.to_string());
    } else {
        response = response.add_attribute("total_shares_claimed", "0");
    }

    if !auto_restake_shares.is_zero() {
        response = response.add_attribute("auto_restaked_shares", auto_restake_shares.to_string());
    }

    Ok(response)
}
