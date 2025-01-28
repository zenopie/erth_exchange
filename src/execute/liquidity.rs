use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdResult, StdError, Uint128, Addr, to_binary,
    CosmosMsg, WasmMsg,};
use secret_toolkit::snip20;

use crate::state::{CONFIG, STATE, PoolInfo, POOL_INFO, UserInfo, USER_INFO};
use crate::msg::{SendMsg};
use crate::execute::SCALING_FACTOR;






pub fn add_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount_erth: Uint128,
    amount_b: Uint128,
    pool: String,
    stake: bool,
) -> Result<Response, StdError> {
    // Load the configuration data from storage
    let config = CONFIG.load(deps.storage)?;
    let pool_addr = deps.api.addr_validate(&pool)?;

    // Retrieve pool information from storage or return an error if not found
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool not found"))?;

    // Determine LP shares to mint and adjust token amounts if the pool is not empty
    let (shares, adjusted_amount_erth, adjusted_amount_b) = if pool_info.state.total_shares.is_zero() {
        let shares = amount_erth + amount_b;
        (shares, amount_erth, amount_b)
    } else {
        let share_erth = amount_erth * pool_info.state.total_shares / pool_info.state.erth_reserve;
        let share_b = amount_b * pool_info.state.total_shares / pool_info.state.token_b_reserve;
        let shares = share_erth.min(share_b);

        let adjusted_amount_erth =
            (shares * pool_info.state.erth_reserve) / pool_info.state.total_shares;
        let adjusted_amount_b =
            (shares * pool_info.state.token_b_reserve) / pool_info.state.total_shares;

        (shares, adjusted_amount_erth, adjusted_amount_b)
    };

    // Identify which token and how much of it should be refunded to the sender
    let (excess_token, excess_amount) = if amount_erth > adjusted_amount_erth {
        (config.erth_token_contract.clone(), amount_erth - adjusted_amount_erth)
    } else {
        (pool_info.config.token_b_contract.clone(), amount_b - adjusted_amount_b)
    };

    // Prepare messages to transfer the adjusted token amounts from the user to this contract
    let mut messages = vec![];
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
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
    }));
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
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
    }));

    // If excess tokens exist, create a message to refund them to the user
    if excess_amount > Uint128::from(2u32) {
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: excess_token.to_string(),
            code_hash: if excess_token == config.erth_token_contract {
                config.erth_token_hash.clone()
            } else {
                pool_info.config.token_b_hash.clone()
            },
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: info.sender.to_string(),
                amount: excess_amount,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        }));
    }

    // Update the pool's token reserves and total shares
    pool_info.state.erth_reserve += adjusted_amount_erth;
    pool_info.state.token_b_reserve += adjusted_amount_b;
    pool_info.state.total_shares += shares;

    // Decide who will receive the minted LP tokens
    let mint_recipient = if stake {
        env.contract.address.clone()
    } else {
        info.sender.clone()
    };

    // Mint the new LP tokens
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.lp_token_contract.to_string(),
        code_hash: pool_info.config.lp_token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::Mint {
            recipient: mint_recipient.to_string(),
            amount: shares,
            memo: None,
            padding: None,
        })?,
        funds: vec![],
    }));

    // If staking is requested, update the user's staking information accordingly
    if stake {
        let user_info_by_pool = USER_INFO.add_suffix(pool_addr.as_bytes());
        let mut user_info = user_info_by_pool
            .get(deps.storage, &info.sender)
            .unwrap_or_default();

        // Update any outstanding rewards if the user has previously staked
        if user_info.amount_staked > Uint128::zero() {
            update_user_rewards(&pool_info, &mut user_info)?;
        }

        // Increase the user's staked amount by the newly minted shares
        user_info.amount_staked += shares;
        pool_info.state.total_staked += shares;
        user_info.reward_debt =
            user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;

        // Persist changes in storage
        user_info_by_pool.insert(deps.storage, &info.sender, &user_info)?;
    }
    // Store updated pool information
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "add_liquidity")
        .add_attribute("from", info.sender)
        .add_attribute("shares", shares.to_string())
        .add_attribute("adjusted_amount_erth", adjusted_amount_erth.to_string())
        .add_attribute("adjusted_amount_b", adjusted_amount_b.to_string())
    )
}




pub fn unbond_liquidity(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    from: Addr,
    lp_token_amount: Uint128,
    pool: String,
) -> Result<Response, StdError> {

    let pool_addr = deps.api.addr_validate(&pool)?;
    let config = CONFIG.load(deps.storage)?;

    // Load pool info from storage
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool info not found"))?;


    if info.sender != pool_info.config.lp_token_contract {
        return Err(StdError::generic_err("Invalid LP token"));
    }

    // Calculate the amount of ERTH and B tokens to return
    let amount_erth = (lp_token_amount * pool_info.state.erth_reserve) / pool_info.state.total_shares;
    let amount_b = (lp_token_amount * pool_info.state.token_b_reserve) / pool_info.state.total_shares;

    // Update the state reserves and total shares
    pool_info.state.erth_reserve -= amount_erth;
    pool_info.state.token_b_reserve -= amount_b;

    // Adjust total shares based on the unbonding amounts
    pool_info.state.total_shares -= lp_token_amount;

    let mut messages = vec![];

    // Create message to burn the LP tokens
    let burn_lp_msg = snip20::HandleMsg::Burn {
        amount: lp_token_amount,
        memo: None,
        padding: None,
    };

    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.lp_token_contract.to_string(),
        code_hash: pool_info.config.lp_token_hash.clone(),
        msg: to_binary(&burn_lp_msg)?,
        funds: vec![],
    }));

    // Transfer the unbonded tokens to the user
    let transfer_erth_msg = snip20::HandleMsg::Transfer {
        recipient: from.clone().to_string(),
        amount: amount_erth,
        padding: None,
        memo: None,
    };
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.erth_token_contract.to_string(),
        code_hash: config.erth_token_hash.clone(),
        msg: to_binary(&transfer_erth_msg)?,
        funds: vec![],
    }));

    let transfer_b_msg = snip20::HandleMsg::Transfer {
        recipient: from.clone().to_string(),
        amount: amount_b,
        padding: None,
        memo: None,
    };
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.token_b_contract.to_string(),
        code_hash: pool_info.config.token_b_hash.clone(),
        msg: to_binary(&transfer_b_msg)?,
        funds: vec![],
    }));


    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "unbond_liquidity")
        .add_attribute("from", from)
        .add_attribute("erth_token_amount", amount_erth.to_string())
        .add_attribute("token_b_amount", amount_b.to_string())
        .add_attribute("lp_token_amount", lp_token_amount.to_string()))
}




pub fn deposit_lp_tokens(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    from: Addr,
    amount: Uint128,
    pool: String,
) -> Result<Response, StdError> {

    let pool_addr = deps.api.addr_validate(&pool)?;

    // Load pool info from storage
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool info not found"))?;

    if info.sender != pool_info.config.lp_token_contract {
        return Err(StdError::generic_err("Invalid LP token"));
    }
    
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
    info: MessageInfo,
    pool: String,
    amount: Uint128,
    unbond: bool,
) -> StdResult<Response> {
    // Validate the provided pool address
    let pool_addr = deps.api.addr_validate(&pool)?;

    // Load relevant contract and pool state
    let state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let mut pool_info = POOL_INFO
        .get(deps.storage, &pool_addr)
        .ok_or_else(|| StdError::generic_err("Pool info not found"))?;

    // Retrieve the user's staking information
    let user_info_by_pool = USER_INFO.add_suffix(&pool_addr.as_bytes());
    let mut user_info = user_info_by_pool
        .get(deps.storage, &info.sender)
        .ok_or_else(|| StdError::generic_err("User info not found"))?;


    // Update the user's reward information with the current pool state
    update_user_rewards(&pool_info, &mut user_info)?;

    // Verify that the user has sufficient staked LP tokens
    if user_info.amount_staked < amount {
        return Err(StdError::generic_err("Insufficient staked amount to withdraw"));
    }

    // Subtract the requested withdrawal amount from the user's staked tokens
    user_info.amount_staked -= amount;
    pool_info.state.total_staked -= amount;

    // Prepare messages to transfer pending rewards if any
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

    // Update or remove the user's information in storage
    if user_info.amount_staked.is_zero() {
        user_info_by_pool.remove(deps.storage, &info.sender)?;
    } else {
        user_info.reward_debt =
            user_info.amount_staked * pool_info.state.reward_per_token_scaled / SCALING_FACTOR;
        user_info_by_pool.insert(deps.storage, &info.sender, &user_info)?;
    }

    // Save the modified pool information and global state
    POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
    STATE.save(deps.storage, &state)?;

    // If unbond is true, burn the LP tokens and return underlying assets
    if unbond {
        let amount_erth =
            (amount * pool_info.state.erth_reserve) / pool_info.state.total_shares;
        let amount_b =
            (amount * pool_info.state.token_b_reserve) / pool_info.state.total_shares;

        pool_info.state.erth_reserve -= amount_erth;
        pool_info.state.token_b_reserve -= amount_b;
        pool_info.state.total_shares -= amount;
        POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

        let burn_lp_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_info.config.lp_token_contract.to_string(),
            code_hash: pool_info.config.lp_token_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::Burn {
                amount,
                memo: None,
                padding: None,
            })?,
            funds: vec![],
        });
        messages.push(burn_lp_msg);

        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.erth_token_contract.to_string(),
            code_hash: config.erth_token_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: info.sender.to_string(),
                amount: amount_erth,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        }));
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_info.config.token_b_contract.to_string(),
            code_hash: pool_info.config.token_b_hash.clone(),
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: info.sender.to_string(),
                amount: amount_b,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        }));

        Ok(Response::new()
            .add_messages(messages)
            .add_attribute("action", "withdraw_and_unbond")
            .add_attribute("withdrawn_lp", amount.to_string())
            .add_attribute("unbonded_erth", amount_erth.to_string())
            .add_attribute("unbonded_b", amount_b.to_string()))
    } else {
        // Otherwise, simply transfer the LP tokens back to the user
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
            .add_attribute("action", "withdraw_lp")
            .add_attribute("withdrawn_lp", amount.to_string()))
    }
}

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


