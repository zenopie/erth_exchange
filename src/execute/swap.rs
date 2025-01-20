use cosmwasm_std::{
    DepsMut, Env, MessageInfo, Response, StdError, Uint128, Addr,
    to_binary, CosmosMsg, WasmMsg,
};
use secret_toolkit::snip20;

use crate::state::{STATE, PoolInfo, BUY_ORDERS, Order, Config, POOL_INFO, CONFIG, SELL_ORDERS,
};
use crate::execute::SCALING_FACTOR;





////////////////////////////////////////////////////////
// MAIN SWAP ENTRY
////////////////////////////////////////////////////////


pub fn swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    from: Addr,
    mut amount: Uint128,
    output_token: String,
) -> Result<Response, StdError> {
    let config = CONFIG.load(deps.storage)?;
    let output_addr = deps.api.addr_validate(&output_token)?;
    let mut response = Response::new().add_attribute("action", "swap");

    let input_is_erth  = info.sender == config.erth_token_contract;
    let output_is_erth = output_addr == config.erth_token_contract;

    if !input_is_erth && !output_is_erth {
        // double swap scenario
        let double_msgs = do_double_swap(
            deps,
            env,
            info,
            &config,
            from,
            amount,
            &output_addr
        )?;

        return Ok(Response::new().add_messages(double_msgs));
    }

    // single swap scenario
    let is_buy = input_is_erth;
    let pool_addr = if input_is_erth {
        &output_addr
    } else {
        &info.sender
    }
    let (fill_msgs, fill_gained, fill_fee) = fill_orders(
        deps.branch(),
        env.clone(),
        &config,
        &mut amount,
        is_buy,
        &from,
        &pool_addr,
    )?;
    let mut msgs = fill_msgs;

    let mut total_gained = fill_gained;
    let mut total_fee_in_erth = Uint128::zero(); // accumulate fallback fee here

    // If leftover, do amm swap
    if !amount.is_zero() {
        // fallback_single_swap returns (Response, leftover_out, fee_in_erth)
        let (amm_out, amm_fee, amm_volume) = amm_swap(
            &config,
            &pool_info,
            amount,
            is_buy
        )?;
        msgs.extend(fb_resp.messages);
        attrs.extend(fb_resp.attributes);

        // accumulate leftover user output
        total_gained = total_gained.checked_add(fb_out)?;
        // accumulate fee in ERTH
        total_fee_in_erth = total_fee_in_erth.checked_add(fb_fee)?;
    }

    // 1) single submessage to give user their final leftover output
    if !total_gained.is_zero() {
        // If is_buy => user obtains tokens => "token_contract_escrow"
        // If is_buy=false => user obtains ERTH => config.erth_token_contract
        let (output_addr, code_hash) = if is_buy {
            (Addr::unchecked("token_contract_escrow"), "token_contract_hash".to_string())
        } else {
            (config.erth_token_contract.clone(), config.erth_token_hash.clone())
        };

        let taker_xfer_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: output_addr.to_string(),
            code_hash: code_hash,
            msg: to_binary(&snip20::HandleMsg::Transfer {
                recipient: from.to_string(),
                amount: total_gained,
                padding: None,
                memo: None,
            })?,
            funds: vec![],
        });
        msgs.push(taker_xfer_msg);
    }

    // 2) single burn message for the fallback fee in ERTH
    if !total_fee_in_erth.is_zero() {
        // we burn from contract. If user was selling, we already converted token fee->ERTH
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
        msgs.push(burn_wasm_msg);
    }

    Ok(Response::new()
        .add_messages(msgs))
}


////////////////////////////////////////////////////////
// DOUBLE SWAP
////////////////////////////////////////////////////////

fn do_double_swap(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    config: &Config,
    taker_addr: Addr,
    mut taker_input_amount: Uint128,
    output_token: &Addr
) -> Result<Vec<CosmosMsg>, StdError> {

    // Leg 1: input->ERTH => is_buy=false
    let (fill1_msgs, fill1_gained, fill1_fee) = fill_orders(
        deps,
        env,
        &config,
        &mut taker_input_amount,
        false,
        &taker_addr,
        &info.sender,
    )?;
    let mut msgs = fill1_msgs;

    let mut pool1_info = POOL_INFO
            .get(deps.storage, &info.sender)
            .ok_or_else(|| StdError::generic_err("pool not found"))?;

    let mut intermediate_erth = fill1_gained;
    if !taker_input_amount.is_zero() {
        let (amm1_out, amm1_fee, amm1_volume) = amm_swap(
            &config,
            &mut pool1_info,
            taker_input_amount,
            false
        )?;

        intermediate_erth += amm1_out;
    }

    // Leg 2: ERTH->output => is_buy=true
    let mut leftover_erth = intermediate_erth;
    let (fill2_msgs, fill2_gained, fill2_fee) = fill_orders(
        deps.branch(),
        env.clone(),
        config,
        &mut leftover_erth,
        true,
        &taker_addr,
        &output_token,
    )?;
    msgs.extend(fill2_msgs);

    let mut pool2_info = POOL_INFO
            .get(deps.storage, &output_token)
            .ok_or_else(|| StdError::generic_err("pool not found"))?;

    let mut final_out = fill2_gained;
    if !leftover_erth.is_zero() {
        let (amm2_out, amm2_fee, amm2_volume) = amm_swap(
            &config,
            &mut pool2_info,
            leftover_erth,
            true
        )?;

        final_out += amm2_out;
    }

    Ok(msgs)
}

////////////////////////////////////////////////////
// FILL ORDERS
////////////////////////////////////////////////////


fn fill_orders(
    deps: DepsMut,
    env: Env,
    config: &Config,
    taker_input_amount: &mut Uint128,
    is_buy: bool,
    taker_addr: &Addr,
    pool_addr: &Addr,
) -> Result<(Vec<CosmosMsg>, Uint128, Uint128), StdError> {

    let mut state = STATE.load(deps.storage)?;
    let mut pool_info = POOL_INFO
            .get(deps.storage, &pool_addr)
            .ok_or_else(|| StdError::generic_err("pool not found"))?;

    let mut taker_accumulated_output = Uint128::zero(); // final single transfer to taker
    let mut total_fee_in_token_b = Uint128::zero(); 
    let mut total_fee_in_erth = Uint128::zero();        // single burn at the end
    let mut messages = Vec::new();              

    // Read pool for effective price
    if pool_info.state.erth_reserve.is_zero() || pool_info.state.token_b_reserve.is_zero() {
        return Err(StdError::generic_err("insufficient liquidity to calculate price"));
    }
    let out_est = feeless_amm_swap(&config, &pool_info, *taker_input_amount, is_buy)?;

    if out_est.is_zero() {
        return Err(StdError::generic_err("insufficient input to calculate price"));
    }

    let eff_price = if is_buy {
        // ERTH_in / token_out
        taker_input_amount.checked_div(out_est)?
    } else {
        // ERTH_out / token_in
        out_est.checked_div(*taker_input_amount)?
    };

    // Gather relevant orders
    if is_buy {
        // user pays ERTH => we want SELL_ORDERS with price <= eff_price, ascending
        let mut relevant_keys = Vec::new();
        let sell_orders_by_pool = SELL_ORDERS.add_suffix(&pool_addr.as_bytes());
        for key_res in sell_orders_by_pool.iter_keys(deps.storage)? {
            let price_key = key_res?;
            if price_key <= eff_price {
                relevant_keys.push(price_key);
            }
        }
        relevant_keys.sort();


        for price_key in relevant_keys {
            if taker_input_amount.is_zero() {
                break;
            }
            let mut orders_vec = sell_orders_by_pool.get(deps.storage, &price_key)?;
            let mut leftover_orders = Vec::new();

            for mut order in orders_vec {
                if taker_input_amount.is_zero() {
                    leftover_orders.push(order);
                    continue;
                }
                // Calculate the taker output based on the taker input and price
                let taker_output = taker_input_amount
                .checked_mul(Uint128::from(1_000_000u128))?
                .checked_div(price_key)?;

                // Fill amount is the smaller of the taker's output and the maker's remaining tokens
                let fill_amt = taker_output.min(order.remaining);


                let (taker_out, maker_msg, fee_in_erth, fee_in_token_b)
                    = fill_single_order(
                        &config,
                        &pool_info,
                        &mut order,
                        fill_amt,
                        true, // is_buy
                    )?;

                // Maker submessages for this fill
                messages.push(maker_msg);

                // accumulate taker’s output
                taker_accumulated_output += taker_out;
                total_fee_in_erth += fee_in_erth;
                total_fee_in_token_b += fee_in_token_b;

                *taker_input_amount = taker_input_amount.checked_sub(fill_amt)?;

                if !order.remaining.is_zero() {
                    leftover_orders.push(order);
                }
            }

            if leftover_orders.is_empty() {
                sell_orders_by_pool.remove(deps.storage, &price_key)?;
            } else {
                sell_orders_by_pool.insert(deps.storage, &price_key, &leftover_orders)?;
            }
        }

        POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
        STATE.save(deps.storage, &state)?;
    } else {
        // user is selling => fill BUY_ORDERS with price >= eff_price, descending
        let mut relevant_keys = Vec::new();
        let buy_orders_by_pool = BUY_ORDERS.add_suffix(&pool_addr.as_bytes());
        for key_res in buy_orders_by_pool.iter_keys(deps.storage)? {
            let price_key = key_res?;
            if price_key >= eff_price {
                relevant_keys.push(price_key);
            }
        }
        relevant_keys.sort();
        relevant_keys.reverse();

        for price_key in relevant_keys {
            if taker_input_amount.is_zero() {
                break;
            }
            let mut orders_vec = buy_orders_by_pool.get(deps.storage, &price_key)?;
            let mut leftover_orders = Vec::new();

            for mut order in orders_vec {
                if taker_input_amount.is_zero() {
                    leftover_orders.push(order);
                    continue;
                }
                // Calculate the taker output (ERTH) based on the taker input and price
                let taker_output = taker_input_amount
                .checked_mul(price_key)?
                .checked_div(Uint128::from(1_000_000u128))?;

                // Fill amount is the smaller of the taker's output and the maker's remaining ERTH
                let fill_amt = taker_output.min(order.input_remaining);


                let (taker_out, maker_msg, fee_in_erth, fee_in_token_b)
                    = fill_single_order(
                        &config,
                        &pool_info,
                        &mut order,
                        fill_amt,
                        false, // is_buy=false
                    )?;

                messages.push(maker_msg);
                taker_accumulated_output += taker_out;
                total_fee_in_erth += fee_in_erth;
                total_fee_in_token_b += fee_in_token_b;

                *taker_input_amount = taker_input_amount.checked_sub(fill_amt)?;

                if !order.remaining.is_zero() {
                    leftover_orders.push(order);
                }
            }

            if leftover_orders.is_empty() {
                buy_orders_by_pool.remove(deps.storage, &price_key)?;
            } else {
                buy_orders_by_pool.insert(deps.storage, &price_key, &leftover_orders)?;
            }
        }

        POOL_INFO.insert(deps.storage, &pool_addr, &pool_info)?;
        STATE.save(deps.storage, &state)?;
    }

    Ok((messages, taker_accumulated_output, total_fee_in_erth))
}


////////////////////////////////////////////////////
// Fill Single Order
////////////////////////////////////////////////////


fn fill_single_order(
    config: &Config,
    pool_info: &PoolInfo,
    order: &mut Order,
    fill_amt: Uint128,   // Amount in the input token (ERTH if buying, TOKEN_B if selling)
    is_buy: bool, 
) -> Result<(Uint128, CosmosMsg, Uint128, Uint128), StdError> {
    //returns -> taker_out, pay_maker_msg, erth_fee, token_b_fee

    let (taker_out, pay_maker_msg, erth_fee, token_b_fee) = if is_buy {
        // BUY ORDER: taker input is in ERTH.
        // Calculate gross token output based on the scaled price:
        let gross_tokens = fill_amt * SCALING_FACTOR / order.erth_price_scaled;
        // Taker fee is charged on the token output:
        let taker_fee_tokens = gross_tokens.multiply_ratio(config.taker_fee, Uint128::from(10_000u128));
        // Taker output = gross tokens minus taker fee (if you want to subtract fee on taker output)
        let tokens_out_net = gross_tokens.checked_sub(taker_fee_tokens)?;
        // Maker fee is charged on the taker’s ERTH input:
        let maker_fee_erth = fill_amt.multiply_ratio(config.maker_fee, Uint128::from(10_000u128));
        let net_erth_to_maker = fill_amt.checked_sub(maker_fee_erth)?;
        // Build submessage: Maker receives net_erth_to_maker in ERTH

        let maker_msg = if let Some(user) = &order.user {
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.erth_token_contract.to_string(),
                code_hash: config.erth_token_hash.clone(),
                msg: to_binary(&snip20::HandleMsg::Transfer {
                    recipient: user,
                    amount: net_erth_to_maker,
                    padding: None,
                    memo: None,
                })?,
                funds: vec![],
            })
        } else {
            // Construct and return the burn message when order.user is None
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.erth_token_contract.to_string(),
                code_hash: config.erth_token_hash.clone(),
                msg: to_binary(&snip20::HandleMsg::Burn {
                    amount: net_erth_to_maker,
                    memo: None,
                    padding: None,
                })?,
                funds: vec![],
            })
        };
        
        

        (tokens_out_net, , maker_msg, maker_fee_erth, taker_fee_tokens)
    } else {
        // SELL ORDER: taker input is in TOKEN_B.
        // Calculate the gross ERTH output using the scaled price:
        let gross_erth = fill_amt * order.erth_price_scaled / SCALING_FACTOR;
        // Taker fee is charged on the gross ERTH output:
        let taker_fee_erth = gross_erth.multiply_ratio(config.taker_fee, Uint128::from(10_000u128));
        let net_erth_to_taker = gross_erth.checked_sub(taker_fee_erth)?;
        // Maker fee is charged on the token input:
        let maker_fee_tokens = fill_amt.multiply_ratio(config.maker_fee, Uint128::from(10_000u128));
        let net_tokens_to_maker = fill_amt.checked_sub(maker_fee_tokens)?;
        // Build submessage: Maker receives net_tokens_to_maker.
        // Construct transfer message for final output
        let maker_msg = if let Some(user) = &order.user {
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: pool_info.config.token_b_contract.to_string(),
                code_hash: pool_info.config.token_b_hash.clone(),
                msg: to_binary(&snip20::HandleMsg::Transfer {
                    recipient: user,
                    amount: net_tokens_to_maker,
                    padding: None,
                    memo: None,
                })?,
                funds: vec![],
            })
        } else {
            // Construct and return the burn message when order.user is None
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: pool_info.config.token_b_contract.to_string(),
                code_hash: pool_info.config.token_b_hash.clone(),
                msg: to_binary(&snip20::HandleMsg::Burn {
                    amount: net_tokens_to_maker,
                    memo: None,
                    padding: None,
                })?,
                funds: vec![],
            })
        };
        (net_erth_to_taker, maker_msg, taker_fee_erth, maker_fee_tokens)
    };

    // Reduce the order's remaining quantity.
    order.output_remaining = order.output_remaining.checked_sub(fill_amt)?;

    Ok((taker_out, pay_maker_msg, erth_fee, token_b_fee))
}




////////////////////////////////////////////////////
// AMM SWAP
////////////////////////////////////////////////////

pub fn amm_swap(
    config: &Config,
    pool_info: &mut PoolInfo,
    input_amount: Uint128,
    is_buy: bool,
) -> Result<(Uint128, Uint128, Uint128), StdError> {
    // Determine reserves and trade volume
    let (input_reserve, output_reserve, trade_volume_in_erth) = if is_buy {
        (
            pool_info.state.erth_reserve,
            pool_info.state.token_b_reserve,
            input_amount, // Trade volume is the input amount in ERTH
        )
    } else {
        (
            pool_info.state.token_b_reserve,
            pool_info.state.erth_reserve,
            (input_amount * pool_info.state.erth_reserve) / pool_info.state.token_b_reserve,
        )
    };

    // Calculate protocol fee
    let mut protocol_fee = if is_buy {
        input_amount * config.taker_fee / Uint128::from(10000u128)
    } else {
        Uint128::zero() // Fee calculated on output later for sells
    };

    // Adjust input for protocol fee (for buys)
    let adjusted_input = input_amount - protocol_fee;

    // Calculate output using constant product formula
    let output_amount = (adjusted_input * output_reserve) / (input_reserve + adjusted_input);
    if output_amount > output_reserve {
        return Err(StdError::generic_err("Insufficient liquidity in reserves"));
    }

    // For sells, calculate protocol fee after determining output
    let net_output = if is_buy {
        output_amount // For buys, the protocol fee is already deducted from the input
    } else {
        // For sells, calculate the protocol fee on the output amount
        protocol_fee = output_amount * config.taker_fee / Uint128::from(10000u128);
        output_amount.checked_sub(protocol_fee)?
    };

    // Update the reserves
    if is_buy {
        pool_info.state.erth_reserve += adjusted_input; // Add input after fee to ERTH reserve
        pool_info.state.token_b_reserve -= output_amount; // Subtract total output from token B reserve
    } else {
        pool_info.state.token_b_reserve += adjusted_input; // Add input to token B reserve
        pool_info.state.erth_reserve -= output_amount; // Subtract total output from ERTH reserve
    }

    // Return the protocol fee for burning later
    Ok((net_output, protocol_fee, trade_volume_in_erth))
}

pub fn anml_buyback(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, StdError> {
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;
    let input_token = info.sender.clone();

    if input_token != config.erth_token_contract {
        return Err(StdError::generic_err("Invalid input token"));
    }

    let mut pool_info = POOL_INFO
        .get(deps.storage, &config.anml_token_contract)
        .ok_or_else(|| StdError::generic_err("ANML pool not found"))?;

    // Calculate the current price of ANML in terms of ERTH
    let current_price_scaled = pool_info.state.erth_reserve
        .checked_mul(Uint128::from(1_000_000u128))?
        .checked_div(pool_info.state.token_b_reserve)?;

    // Access or initialize the buy orders at the current price
    let buy_orders_by_pool = BUY_ORDERS.add_suffix(&config.anml_token_contract.as_bytes());
    let mut orders_at_price = buy_orders_by_pool
        .get(deps.storage, &current_price_scaled)
        .unwrap_or_else(|| Vec::new()); // Initialize an empty Vec<Order> if none exist

    // Check if there is an existing order with no user
    if let Some(existing_order) = orders_at_price.iter_mut().find(|order| order.user.is_none()) {
        // Add to the existing order's input_remaining
        existing_order.input_remaining += amount;
    } else {
        // Create a new order if no matching order exists
        orders_at_price.push(Order {
            user: None, // No specific user for buyback orders
            erth_price_scaled: current_price_scaled,
            output_remaining: Uint128::zero(),
            input_remaining: amount,
        });
    }

    // Save the updated orders back to storage
    buy_orders_by_pool.insert(deps.storage, &current_price_scaled, &orders_at_price)?;

    // Save the updated state
    STATE.save(deps.storage, &state)?;

    // Add attributes for tracking the action
    Ok(Response::new()
        .add_attribute("action", "buyback_order")
        .add_attribute("input_amount", amount.to_string())
        .add_attribute("current_price_scaled", current_price_scaled.to_string()))
}




fn feeless_amm_swap(
    config: &Config,
    pool_info: &PoolInfo,
    input_amount: Uint128,
    is_buy: bool,
) -> Result<Uint128, StdError> {
    // Extract the reserves immutably before mutating state
    let (input_reserve, output_reserve) = if is_buy {
        (pool_info.state.erth_reserve, pool_info.state.token_b_reserve)
    } else {
        (pool_info.state.token_b_reserve, pool_info.state.erth_reserve)
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

