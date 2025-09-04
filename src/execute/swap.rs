use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdError, Uint128, Addr, to_binary,
    CosmosMsg, WasmMsg, BankMsg, Coin};
use secret_toolkit::snip20;

use crate::state::{Config, CONFIG, STATE, PoolInfo, POOL_INFO, SSCRT_TOKEN_CONTRACT, SSCRT_TOKEN_HASH};


#[derive(Debug, Clone)]
pub struct SwapResult {
    pub output_amount: Uint128,
    pub intermediate_amount: Option<Uint128>,
    pub total_fee: Uint128,
    pub burn_messages: Vec<CosmosMsg>,
    pub transfer_messages: Vec<CosmosMsg>,
    pub trade_volume: Uint128,
}

pub fn swap(
    mut deps: DepsMut,
    info: MessageInfo,
    from: Addr,
    amount: Uint128,
    output_token: String,
) -> Result<Response, StdError> {
    let output_token_addr = deps.api.addr_validate(&output_token)?;
    let input_token = info.sender.clone();

    // Execute swap using the generic helper
    let swap_result = execute_swap_logic(
        &mut deps,
        &input_token,
        &output_token_addr,
        amount,
        true, // with_fees = true
        &from,
        None, // Use default receiver (from)
    )?;

    // Build response with messages and attributes
    let mut response = Response::new()
        .add_messages(swap_result.burn_messages)
        .add_messages(swap_result.transfer_messages)
        .add_attribute("from", from.to_string())
        .add_attribute("input_amount", amount.to_string())
        .add_attribute("output_amount", swap_result.output_amount.to_string())
        .add_attribute("protocol_fee", swap_result.total_fee.to_string());

    // Add appropriate action and volume attributes based on swap type
    if let Some(intermediate) = swap_result.intermediate_amount {
        response = response
            .add_attribute("action", "double_swap")
            .add_attribute("intermediate_amount", intermediate.to_string());
    } else {
        let config = CONFIG.load(deps.storage)?;
        let action = if input_token == config.erth_token_contract {
            "swap_erth_in"
        } else {
            "swap_token_in"
        };
        response = response
            .add_attribute("action", action)
            .add_attribute("trade_volume_in_erth", swap_result.trade_volume.to_string());
    }

    Ok(response)
}










pub fn anml_buyback_swap(
    deps: DepsMut,
    _env: Env,
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

    // Calculate swap details (no fees)
    let output_amount = calculate_amm_swap(&config, &anml_pool_info, &input_token, amount, false)?.output_amount;

    // Update pool reserves
    anml_pool_info.state.erth_reserve += amount;
    anml_pool_info.state.token_b_reserve -= output_amount;
    anml_pool_info.state.daily_volumes[0] += amount;
    POOL_INFO.insert(deps.storage, &config.anml_token_contract, &anml_pool_info)?;

    // Track total ANML burned
    state.anml_burned += output_amount;

    // Save updated state
    STATE.save(deps.storage, &state)?;

    // Burn message
    let burn_msg = snip20::HandleMsg::Burn {
        amount: output_amount,
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




#[derive(Debug, Clone)]
pub struct SwapCalculation {
    pub output_amount: Uint128,
    pub protocol_fee: Uint128,
    pub trade_volume: Uint128,
    pub price_impact: Uint128, // Price impact in basis points (e.g., 250 = 2.5%)
}

pub fn calculate_amm_swap(
    config: &Config,
    pool_info: &PoolInfo,
    input_token: &Addr,
    input_amount: Uint128,
    apply_fees: bool,
) -> Result<SwapCalculation, StdError> {
    let protocol_fee = if apply_fees {
        input_amount * config.protocol_fee / Uint128::from(10000u128)
    } else {
        Uint128::zero()
    };
    
    let amount_after_fee = input_amount - protocol_fee;

    // Get reserves and calculate trade volume
    let (input_reserve, output_reserve, trade_volume) = if input_token == &config.erth_token_contract {
        (
            pool_info.state.erth_reserve,
            pool_info.state.token_b_reserve,
            input_amount, // Trade volume is input amount in ERTH
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

    // Calculate output using constant product formula
    let output_amount = (amount_after_fee * output_reserve) / (input_reserve + amount_after_fee);

    // Check liquidity
    if output_amount > output_reserve {
        return Err(StdError::generic_err("Insufficient liquidity in reserves"));
    }

    // Calculate price impact in basis points (10000 = 100%)
    // Ideal output would be: amount_after_fee * (output_reserve / input_reserve)
    // This represents the spot price without slippage
    let ideal_output = amount_after_fee * output_reserve / input_reserve;
    let price_impact = if ideal_output > output_amount && !ideal_output.is_zero() {
        ((ideal_output - output_amount) * Uint128::from(10000u128)) / ideal_output
    } else {
        Uint128::zero()
    };

    Ok(SwapCalculation {
        output_amount,
        protocol_fee,
        trade_volume,
        price_impact,
    })
}

fn update_pool_reserves(
    config: &Config,
    pool_info: &mut PoolInfo,
    input_token: &Addr,
    input_amount: Uint128,
    output_amount: Uint128,
    protocol_fee: Uint128,
) -> Result<Uint128, StdError> {
    let mut final_protocol_fee = protocol_fee;
    
    if input_token == &config.erth_token_contract {
        // ERTH -> token swap
        let amount_after_fee = input_amount - protocol_fee;
        pool_info.state.erth_reserve += amount_after_fee;
        pool_info.state.token_b_reserve -= output_amount;
    } else if input_token == &pool_info.config.token_b_contract {
        // token -> ERTH swap
        let amount_after_fee = input_amount - protocol_fee;
        pool_info.state.token_b_reserve += amount_after_fee;
        pool_info.state.erth_reserve -= output_amount;

        // Convert protocol fee to ERTH if needed
        if !protocol_fee.is_zero() {
            let protocol_fee_in_erth = calculate_amm_swap(config, pool_info, &pool_info.config.token_b_contract, protocol_fee, false)?.output_amount;
            pool_info.state.token_b_reserve += protocol_fee;
            pool_info.state.erth_reserve -= protocol_fee_in_erth;
            final_protocol_fee = protocol_fee_in_erth;
        }
    } else {
        return Err(StdError::generic_err("Invalid input token"));
    }

    Ok(final_protocol_fee)
}


pub fn execute_swap_logic(
    deps: &mut DepsMut,
    input_token: &Addr,
    output_token: &Addr,
    amount: Uint128,
    with_fees: bool,
    from: &Addr,
    receiver: Option<&Addr>,
) -> Result<SwapResult, StdError> {
    let receiver = receiver.unwrap_or(from);
    let config = CONFIG.load(deps.storage)?;
    let mut state = STATE.load(deps.storage)?;
    
    // Check if we need double swap (input != ERTH and output != ERTH)
    if input_token != &config.erth_token_contract && output_token != &config.erth_token_contract {
        // ============== DOUBLE SWAP ==============
        // Step 1: input_token -> ERTH
        let mut input_pool_info = POOL_INFO
            .get(deps.storage, input_token)
            .ok_or_else(|| StdError::generic_err("No pool found for input token"))?;

        let calc1 = calculate_amm_swap(&config, &input_pool_info, input_token, amount, with_fees)?;
        
        let fee_step1 = if with_fees {
            update_pool_reserves(&config, &mut input_pool_info, input_token, amount, calc1.output_amount, calc1.protocol_fee)?
        } else {
            // Manual reserve update for feeless
            if input_token == &config.erth_token_contract {
                input_pool_info.state.erth_reserve += amount;
                input_pool_info.state.token_b_reserve -= calc1.output_amount;
            } else {
                input_pool_info.state.token_b_reserve += amount;
                input_pool_info.state.erth_reserve -= calc1.output_amount;
            }
            Uint128::zero()
        };
        
        let (intermediate_amount, vol_step1) = (calc1.output_amount, calc1.trade_volume);

        input_pool_info.state.daily_volumes[0] += vol_step1;
        POOL_INFO.insert(deps.storage, input_token, &input_pool_info)?;

        // Step 2: ERTH -> output_token
        let mut output_pool_info = POOL_INFO
            .get(deps.storage, output_token)
            .ok_or_else(|| StdError::generic_err("No pool found for output token"))?;

        let calc2 = calculate_amm_swap(&config, &output_pool_info, &config.erth_token_contract, intermediate_amount, with_fees)?;
        
        let fee_step2 = if with_fees {
            update_pool_reserves(&config, &mut output_pool_info, &config.erth_token_contract, intermediate_amount, calc2.output_amount, calc2.protocol_fee)?
        } else {
            output_pool_info.state.erth_reserve += intermediate_amount;
            output_pool_info.state.token_b_reserve -= calc2.output_amount;
            Uint128::zero()
        };
        
        let (final_output_amount, vol_step2) = (calc2.output_amount, calc2.trade_volume);

        output_pool_info.state.daily_volumes[0] += vol_step2;
        POOL_INFO.insert(deps.storage, output_token, &output_pool_info)?;

        let total_fee = fee_step1 + fee_step2;
        let total_volume = vol_step1 + vol_step2;

        // Create burn message if there are fees
        let burn_messages = if !total_fee.is_zero() {
            state.erth_burned += total_fee;
            STATE.save(deps.storage, &state)?;
            
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.erth_token_contract.to_string(),
                code_hash: config.erth_token_hash.clone(),
                msg: to_binary(&snip20::HandleMsg::Burn {
                    amount: total_fee,
                    memo: None,
                    padding: None,
                })?,
                funds: vec![],
            })]
        } else {
            STATE.save(deps.storage, &state)?;
            vec![]
        };

        // Create transfer message for output token to receiver
        let transfer_messages = vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: output_pool_info.config.token_b_contract.to_string(),
            code_hash: output_pool_info.config.token_b_hash,
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: receiver.to_string(),
                amount: final_output_amount,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        })];

        Ok(SwapResult {
            output_amount: final_output_amount,
            intermediate_amount: Some(intermediate_amount),
            total_fee,
            burn_messages,
            transfer_messages,
            trade_volume: total_volume,
        })

    } else {
        // ============== SINGLE SWAP ==============
        if input_token == &config.erth_token_contract {
            // ERTH -> output_token
            let pool_addr = output_token.clone();
            let mut pool_info = POOL_INFO
                .get(deps.storage, &pool_addr)
                .ok_or_else(|| StdError::generic_err("No pool found for output token"))?;

            let calc = calculate_amm_swap(&config, &pool_info, &config.erth_token_contract, amount, with_fees)?;
            
            let protocol_fee = if with_fees {
                update_pool_reserves(&config, &mut pool_info, &config.erth_token_contract, amount, calc.output_amount, calc.protocol_fee)?
            } else {
                pool_info.state.erth_reserve += amount;
                pool_info.state.token_b_reserve -= calc.output_amount;
                Uint128::zero()
            };
            
            let (output_amount, trade_volume) = (calc.output_amount, calc.trade_volume);

            pool_info.state.daily_volumes[0] += trade_volume;
            POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

            // Create burn message if there are fees
            let burn_messages = if !protocol_fee.is_zero() {
                state.erth_burned += protocol_fee;
                STATE.save(deps.storage, &state)?;
                
                vec![CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.erth_token_contract.to_string(),
                    code_hash: config.erth_token_hash.clone(),
                    msg: to_binary(&snip20::HandleMsg::Burn {
                        amount: protocol_fee,
                        memo: None,
                        padding: None,
                    })?,
                    funds: vec![],
                })]
            } else {
                STATE.save(deps.storage, &state)?;
                vec![]
            };

            // Transfer output token to receiver
            let transfer_messages = vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: pool_info.config.token_b_contract.to_string(),
                code_hash: pool_info.config.token_b_hash,
                msg: to_binary(&snip20::HandleMsg::Transfer {
                    recipient: receiver.to_string(),
                    amount: output_amount,
                    padding: None,
                    memo: None,
                })?,
                funds: vec![],
            })];

            Ok(SwapResult {
                output_amount,
                intermediate_amount: None,
                total_fee: protocol_fee,
                burn_messages,
                transfer_messages,
                trade_volume,
            })
        } else {
            // input_token -> ERTH
            let pool_addr = input_token.clone();
            let mut pool_info = POOL_INFO
                .get(deps.storage, &pool_addr)
                .ok_or_else(|| StdError::generic_err("No pool found for input token"))?;

            let calc = calculate_amm_swap(&config, &pool_info, input_token, amount, with_fees)?;
            
            let protocol_fee = if with_fees {
                update_pool_reserves(&config, &mut pool_info, input_token, amount, calc.output_amount, calc.protocol_fee)?
            } else {
                pool_info.state.token_b_reserve += amount;
                pool_info.state.erth_reserve -= calc.output_amount;
                Uint128::zero()
            };
            
            let (output_amount, trade_volume) = (calc.output_amount, calc.trade_volume);

            pool_info.state.daily_volumes[0] += trade_volume;
            POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;

            // Create burn message if there are fees  
            let burn_messages = if !protocol_fee.is_zero() {
                state.erth_burned += protocol_fee;
                STATE.save(deps.storage, &state)?;
                
                vec![CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: config.erth_token_contract.to_string(),
                    code_hash: config.erth_token_hash.clone(),
                    msg: to_binary(&snip20::HandleMsg::Burn {
                        amount: protocol_fee,
                        memo: None,
                        padding: None,
                    })?,
                    funds: vec![],
                })]
            } else {
                STATE.save(deps.storage, &state)?;
                vec![]
            };

            // Transfer ERTH to receiver
            let transfer_messages = vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.erth_token_contract.to_string(),
                code_hash: config.erth_token_hash.clone(),
                msg: to_binary(&snip20::HandleMsg::Transfer {
                    recipient: receiver.to_string(),
                    amount: output_amount,
                    padding: None,
                    memo: None,
                })?,
                funds: vec![],
            })];

            Ok(SwapResult {
                output_amount,
                intermediate_amount: None,
                total_fee: protocol_fee,
                burn_messages,
                transfer_messages,
                trade_volume,
            })
        }
    }
}

// ========== ANY TOKEN → ERTH (feeless) → BURN ==========
pub fn swap_to_erth_and_burn(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, StdError> {
    // Basic checks
    if amount.is_zero() {
        return Err(StdError::generic_err("amount must be greater than zero"));
    }

    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let input_token = info.sender.clone();

    // Case A: Input is already ERTH → direct burn (no pool interaction)
    if input_token == config.erth_token_contract {
        // Track burned ERTH
        state.erth_burned += amount;

        // Distribute any pending rewards prior to finalizing state
        STATE.save(deps.storage, &state)?;

        // Burn ERTH directly
        let burn_msg = snip20::HandleMsg::Burn {
            amount,
            memo: None,
            padding: None,
        };
        let burn_wasm_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.erth_token_contract.to_string(),
            code_hash: config.erth_token_hash.clone(),
            msg: to_binary(&burn_msg)?,
            funds: vec![],
        });

        return Ok(Response::new()
            .add_message(burn_wasm_msg)
            .add_attribute("action", "feeless_burn_erth")
            .add_attribute("input_token", input_token.to_string())
            .add_attribute("input_amount", amount.to_string())
            .add_attribute("erth_burned", amount.to_string()));
    }

    // Case B: TOKEN (token_b) → ERTH (feeless) → burn
    let mut pool_info = POOL_INFO
        .get(deps.storage, &input_token)
        .ok_or_else(|| StdError::generic_err("Pool not found for input token"))?;

    // Ensure this pool is actually keyed by its token_b contract
    if input_token != pool_info.config.token_b_contract {
        return Err(StdError::generic_err("Mismatched pool for input token"));
    }

    // Compute ERTH out without any protocol fee
    let erth_out = calculate_amm_swap(&config, &pool_info, &input_token, amount, false)?.output_amount;

    // Update reserves: add input token to token_b reserve, subtract ERTH output
    // This mirrors the on-chain movement implied by the AMM math
    pool_info.state.token_b_reserve += amount;
    pool_info.state.erth_reserve -= erth_out;

    // Daily volume is tracked in ERTH terms based on input value against current reserves
    let trade_volume = erth_out;
    pool_info.state.daily_volumes[0] += trade_volume;

    // Persist pool updates
    POOL_INFO.insert(deps.storage, &input_token, &pool_info)?;

    // Track total ERTH burned
    state.erth_burned += erth_out;


    // Persist state updates
    STATE.save(deps.storage, &state)?;

    // Burn ERTH output
    let burn_msg = snip20::HandleMsg::Burn {
        amount: erth_out,
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
        .add_message(burn_wasm_msg)
        .add_attribute("action", "swap_to_erth_and_burn")
        .add_attribute("input_token", input_token.to_string())
        .add_attribute("input_amount", amount.to_string())
        .add_attribute("erth_burned", erth_out.to_string()))
}

pub fn swap_for_gas(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    from: Addr,
    amount: Uint128,
) -> Result<Response, StdError> {
    let sscrt_contract = SSCRT_TOKEN_CONTRACT.load(deps.storage)?;
    let sscrt_hash = SSCRT_TOKEN_HASH.load(deps.storage)?;
    let input_token = info.sender.clone();
    
    // If input is already sScrt, just unwrap and send
    if input_token == sscrt_contract {
        let unwrap_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: sscrt_contract.to_string(),
            code_hash: sscrt_hash,
            msg: to_binary(&snip20::HandleMsg::Redeem {
                amount,
                denom: None,
                padding: None,
            })?,
            funds: vec![],
        });

        let send_msg = CosmosMsg::Bank(BankMsg::Send {
            to_address: from.to_string(),
            amount: vec![Coin {
                denom: "uscrt".to_string(),
                amount,
            }],
        });

        return Ok(Response::new()
            .add_message(unwrap_msg)
            .add_message(send_msg)
            .add_attribute("action", "swap_for_gas_direct")
            .add_attribute("from", from.to_string())
            .add_attribute("scrt_amount", amount.to_string()));
    }

    // Execute swap to sScrt, sending to contract instead of user
    let swap_result = execute_swap_logic(
        &mut deps,
        &input_token,
        &sscrt_contract,
        amount,
        true, // with_fees = true
        &from,
        Some(&env.contract.address), // Send sScrt to contract for unwrapping
    )?;

    // Unwrap sScrt to native SCRT
    let unwrap_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: sscrt_contract.to_string(),
        code_hash: sscrt_hash,
        msg: to_binary(&snip20::HandleMsg::Redeem {
            amount: swap_result.output_amount,
            denom: None,
            padding: None,
        })?,
        funds: vec![],
    });

    // Send native SCRT to user
    let send_msg = CosmosMsg::Bank(BankMsg::Send {
        to_address: from.to_string(),
        amount: vec![Coin {
            denom: "uscrt".to_string(),
            amount: swap_result.output_amount,
        }],
    });

    // Build response with all messages
    let mut response = Response::new()
        .add_messages(swap_result.burn_messages)
        .add_messages(swap_result.transfer_messages)
        .add_message(unwrap_msg)
        .add_message(send_msg)
        .add_attribute("from", from.to_string())
        .add_attribute("input_amount", amount.to_string())
        .add_attribute("sscrt_amount", swap_result.output_amount.to_string())
        .add_attribute("scrt_amount", swap_result.output_amount.to_string())
        .add_attribute("protocol_fee", swap_result.total_fee.to_string());

    // Add action and intermediate amount attribute based on swap type
    if let Some(intermediate) = swap_result.intermediate_amount {
        response = response
            .add_attribute("action", "swap_for_gas_double")
            .add_attribute("intermediate_amount", intermediate.to_string());
    } else {
        response = response.add_attribute("action", "swap_for_gas_single");
    }

    Ok(response)
}

