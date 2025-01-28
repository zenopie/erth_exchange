use cosmwasm_std::{DepsMut, MessageInfo, Response, StdError, Uint128, Addr, to_binary,
    CosmosMsg, WasmMsg};
use secret_toolkit::snip20;

use crate::state::{Config, CONFIG, STATE, PoolInfo, POOL_INFO};
use crate::execute::SCALING_FACTOR;

pub fn swap(
    deps: DepsMut,
    info: MessageInfo,
    from: Addr,
    amount: Uint128,
    output_token: String,
) -> Result<Response, StdError> {
    let output_token_addr = deps.api.addr_validate(&output_token)?;
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;

    // Determine if a double swap is needed (input != ERTH and output != ERTH)
    if info.sender != config.erth_token_contract && output_token != config.erth_token_contract {
        // Double swap scenario

        // Swap input_token -> ERTH
        let mut input_pool_info = POOL_INFO
            .get(deps.storage, &info.sender)
            .ok_or_else(|| StdError::generic_err("No pool found for input token"))?;

        let (fee_step1, intermediate_amount, vol_step1) =
            calculate_swap(&config, &mut input_pool_info, amount, &info.sender)?;

        
        input_pool_info.state.pending_volume += vol_step1;
        POOL_INFO.insert(deps.storage, &info.sender, &input_pool_info)?;

        // 2) Swap ERTH -> output_token
        let mut output_pool_info = POOL_INFO
            .get(deps.storage, &output_token_addr)
            .ok_or_else(|| StdError::generic_err("No pool found for output token"))?;

        let (fee_step2, final_output_amount, vol_step2) =
            calculate_swap(&config, &mut output_pool_info, intermediate_amount, &config.erth_token_contract.clone())?;

        output_pool_info.state.pending_volume += vol_step2;
        POOL_INFO.insert(deps.storage, &output_token_addr, &output_pool_info)?;
        
        let total_fee = fee_step1 + fee_step2;
        state.erth_burned += total_fee;
        let total_vol = vol_step1 + vol_step2;
        state.pending_volume += total_vol;
        STATE.save(deps.storage, &state)?;

        // Construct transfer message for final output
        let transfer_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: output_pool_info.config.token_b_contract.to_string(),
            code_hash: output_pool_info.config.token_b_hash,
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: from.to_string(),
                amount: final_output_amount,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        });

        let burn_msg = snip20::HandleMsg::Burn {
            amount: total_fee,
            memo: None,
            padding: None,
        };
        let burn_wasm_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.erth_token_contract.to_string(),
            code_hash: config.erth_token_hash.clone(),
            msg: to_binary(&burn_msg)?,
            funds: vec![],
        });

        Ok(Response::new()
            .add_message(transfer_msg)
            .add_message(burn_wasm_msg)
            .add_attribute("action", "double_swap")
            .add_attribute("from", from.to_string())
            .add_attribute("input_amount", amount.to_string())
            .add_attribute("intermediate_amount", intermediate_amount.to_string())
            .add_attribute("output_amount", final_output_amount.to_string())
            .add_attribute("protocol_fee", total_fee.to_string())
            .add_attribute("trade_volume_step1", vol_step1.to_string())
            .add_attribute("trade_volume_step2", vol_step2.to_string()))

    } else {
        // Single swap scenario (either input or output is ERTH)
        let pool_addr = if info.sender == config.erth_token_contract {
            output_token_addr.clone()
        } else {
            info.sender.clone()
        };
        let mut pool_info = POOL_INFO
            .get(deps.storage, &pool_addr)
            .ok_or_else(|| StdError::generic_err("No pool found for the given token"))?;

        let (protocol_fee, output_amount, trade_volume) =
            calculate_swap(&config, &mut pool_info, amount, &info.sender)?;

        pool_info.state.pending_volume += trade_volume;
        POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

        state.erth_burned += protocol_fee;
        state.pending_volume += trade_volume;
        STATE.save(deps.storage, &state)?;

        // Transfer the output tokens to the user
        let transfer_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pool_info.config.token_b_contract.to_string(),
            code_hash: pool_info.config.token_b_hash,
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: from.to_string(),
                amount: output_amount,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        });

        let burn_msg = snip20::HandleMsg::Burn {
            amount: protocol_fee,
            memo: None,
            padding: None,
        };

        let burn_wasm_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.erth_token_contract.to_string(),
            code_hash: config.erth_token_hash.clone(),
            msg: to_binary(&burn_msg)?,
            funds: vec![],
        });


        Ok(Response::new()
            .add_message(transfer_msg)
            .add_message(burn_wasm_msg)
            .add_attribute("action", "swap")
            .add_attribute("from", from.to_string())
            .add_attribute("input_amount", amount.to_string())
            .add_attribute("output_amount", output_amount.to_string())
            .add_attribute("protocol_fee_amount", protocol_fee.to_string())
            .add_attribute("trade_volume_in_erth", trade_volume.to_string()))
    }
}




pub fn calculate_swap(
    config: &Config,
    pool_info: &mut PoolInfo,  // Mutably borrow the state so we can update reserves
    input_amount: Uint128,
    input_token: &Addr,
) -> Result<(Uint128, Uint128, Uint128), StdError> {
    // Calculate protocol fee in the input token
    let mut protocol_fee_amount = input_amount * config.protocol_fee / Uint128::from(10000u128);
    let amount_after_protocol_fee = input_amount - protocol_fee_amount;

    // Extract all necessary details from the state
    let (input_reserve, output_reserve, trade_volume_in_erth) = if input_token == &config.erth_token_contract {
        (
            pool_info.state.erth_reserve,
            pool_info.state.token_b_reserve,
            input_amount,  // Trade volume is the input amount in ERTH
        )
    } else if input_token == &pool_info.config.token_b_contract {
        (
            pool_info.state.token_b_reserve,
            pool_info.state.erth_reserve,
            // Convert input token volume to ERTH using reserve ratio
            (input_amount * pool_info.state.erth_reserve) / pool_info.state.token_b_reserve,
        )
    } else {
        return Err(StdError::generic_err("Invalid input token"));
    };

    // Calculate the output amount using the constant product formula
    let output_amount = (amount_after_protocol_fee * output_reserve)
        / (input_reserve + amount_after_protocol_fee);

    // Check if the liquidity is enough
    if output_amount > output_reserve {
        return Err(StdError::generic_err("Insufficient liquidity in reserves"));
    }

    // Update the reserves based on the swap
    if input_token == &config.erth_token_contract {
        pool_info.state.erth_reserve += amount_after_protocol_fee; // Add input amount to ERTH reserve
        pool_info.state.token_b_reserve -= output_amount; // Subtract output amount from token B reserve
    } else if input_token == &pool_info.config.token_b_contract {
        pool_info.state.token_b_reserve += amount_after_protocol_fee; // Add to token B reserve after protocol fee is deducted
        pool_info.state.erth_reserve -= output_amount;          // Subtract from ERTH reserve (as we are sending this amount)

        // Perform feeless swap to convert protocol fee to ERTH
        let protocol_fee_in_erth = calculate_feeless_swap(&config, pool_info, protocol_fee_amount, &pool_info.config.token_b_contract)?;

        //update reserves
        pool_info.state.token_b_reserve += protocol_fee_amount;
        pool_info.state.erth_reserve -= protocol_fee_in_erth;

        // The `protocol_fee_amount` now represents the amount in ERTH
        protocol_fee_amount = protocol_fee_in_erth;
    }

    // Return the result including the protocol fee (now in ERTH), output amount, and other details
    Ok((
        protocol_fee_amount, // Protocol fee in ERTH
        output_amount,
        trade_volume_in_erth,
    ))
}



pub fn anml_buyback_swap(
    deps: DepsMut,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, StdError> {

    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let input_token = info.sender.clone();

    if input_token != config.erth_token_contract {
        return Err(StdError::generic_err("invalid input token"));
    }

    let mut anml_pool_info = POOL_INFO
            .get(deps.storage, &config.anml_token_contract)
            .ok_or_else(|| StdError::generic_err("ANML pool not found"))?;


    // Calculate the swap details without fees
    let output_amount = calculate_feeless_swap(&config, &anml_pool_info, amount, &input_token)?;

    // Update reserves
    anml_pool_info.state.erth_reserve += amount;
    anml_pool_info.state.token_b_reserve -= output_amount;
    anml_pool_info.state.pending_volume += amount;
    POOL_INFO.insert(deps.storage, &config.anml_token_contract, &anml_pool_info)?;

    // Save pool info
    state.anml_burned += output_amount;
    

    
    //start of pool rewards upkeep -move to cron once avail
    
    let mut pools = Vec::new();
    // 1) Get the iterator
    let mut iter = POOL_INFO.iter(deps.storage)?;

    while let Some(item_result) = iter.next() {
        let (addr, pool_info) = item_result?; // This is `(Addr, PoolInfo)`
        pools.push((addr, pool_info));        // Push the actual tuple
    }

    // Roll pending_reward into today's new daily_reward_pool
    let reward_pool = state.pending_reward;
    state.pending_reward = Uint128::zero();

    // Distribute yesterday's daily_reward_pool if there was any volume
    if !state.pending_volume.is_zero() {
        for (addr, ref mut pool_info) in pools.iter_mut() {
            if !pool_info.state.pending_volume.is_zero() {
                // Pool's share = (pool_volume / total_volume) * reward_pool
                let pool_share = pool_info.state.pending_volume.multiply_ratio(
                    reward_pool,
                    state.pending_volume
                );
                // Update reward_per_token_scaled if staked
                if !pool_info.state.total_staked.is_zero() {
                    let increment = pool_share
                        .saturating_mul(SCALING_FACTOR)
                        .checked_div(pool_info.state.total_staked)
                        .unwrap_or(Uint128::zero());
                        pool_info.state.reward_per_token_scaled += increment;
                }
            }
            // Reset each pool’s daily volume for the new day
            pool_info.state.pending_volume = Uint128::zero();
            POOL_INFO.insert(deps.storage, &addr, &pool_info)?;
        }
    }

    // Reset global daily volume
    state.pending_volume = Uint128::zero();

    STATE.save(deps.storage, &state)?;

    //end of upkeep

    // Burn message
    // If the received token is ERTH, burn it
    let burn_msg = snip20::HandleMsg::Burn { 
        amount,
        memo: None,
        padding: None,
    };

    let burn_wasm_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.anml_token_contract.to_string(),
        code_hash: config.anml_token_hash.clone(),
        msg: to_binary(&burn_msg)?,
        funds: vec![],
    });


    Ok(Response::new()
        .add_message(burn_wasm_msg)
        .add_attribute("action", "buyback_swap")
        .add_attribute("input_amount", amount.to_string())
        .add_attribute("output_amount", output_amount.to_string()))
}


fn calculate_feeless_swap(
    config: &Config,
    pool_info: &PoolInfo,
    input_amount: Uint128,
    input_token: &Addr,
) -> Result<Uint128, StdError> {
    // Extract the reserves immutably before mutating state
    let (input_reserve, output_reserve) = if input_token == &pool_info.config.token_b_contract {
        (pool_info.state.token_b_reserve, pool_info.state.erth_reserve)
    } else if input_token == &config.erth_token_contract {
        (pool_info.state.erth_reserve, pool_info.state.token_b_reserve)
    } else {
        return Err(StdError::generic_err("Invalid input token for feeless swap"));
    };

    // Calculate the output amount using the constant product formula
    let output_amount = (input_amount * output_reserve)
        / (input_reserve + input_amount);

    // Check if there is enough liquidity in the reserves
    if output_amount > output_reserve {
        return Err(StdError::generic_err(
            "Insufficient liquidity in reserves for feeless swap",
        ));
    }

    // Return the calculated output amount (which is in ERTH)
    Ok(output_amount)
}
