use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdError, Uint128, Addr, to_binary,
    CosmosMsg, WasmMsg,};
use secret_toolkit::snip20;

use crate::state::{CONFIG, POOL_INFO, };



pub fn add_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount_erth: Uint128,
    amount_b: Uint128,
    pool: String,
    _stake: bool,
) -> Result<Response, StdError> {

    let config = CONFIG.load(deps.storage)?;
    let pool_addr = deps.api.addr_validate(&pool)?;

     // Load the existing PoolInfo
     let mut pool_info = POOL_INFO
     .get(deps.storage, &pool_addr)
     .ok_or_else(|| StdError::generic_err("Pool not found"))?;

    let (shares, adjusted_amount_erth, adjusted_amount_b) = if pool_info.state.total_shares.is_zero() {
        // Initial liquidity: use the provided amounts directly and set the total shares to the sum
        let shares = amount_erth + amount_b;
        (shares, amount_erth, amount_b)
    } else {
        // Subsequent liquidity
        let share_erth = amount_erth * pool_info.state.total_shares / pool_info.state.token_erth_reserve;
        let share_b = amount_b * pool_info.state.total_shares / pool_info.state.token_b_reserve;
        let shares = share_erth.min(share_b);

        // Adjust amounts based on the limiting factor
        let adjusted_amount_erth = (shares * pool_info.state.token_erth_reserve) / pool_info.state.total_shares;
        let adjusted_amount_b = (shares * pool_info.state.token_b_reserve) / pool_info.state.total_shares;

        (shares, adjusted_amount_erth, adjusted_amount_b)
    };

    // Calculate the excess amount of the token that exceeds the required ratio
    let (excess_token, excess_amount) = if amount_erth > adjusted_amount_erth {
        (config.erth_token_contract.clone(), amount_erth - adjusted_amount_erth)
    } else {
        (pool_info.config.token_b_contract.clone(), amount_b - adjusted_amount_b)
    };

    let mut messages = vec![];

    // Create messages for transferring tokens from the user to the contract using allowances
    let transfer_erth_msg = snip20::HandleMsg::TransferFrom {
        owner: info.sender.clone().to_string(),
        recipient: env.contract.address.clone().to_string(),
        amount: adjusted_amount_erth,
        padding: None,
        memo: None,
    };
    let transfer_b_msg = snip20::HandleMsg::TransferFrom {
        owner: info.sender.clone().to_string(),
        recipient: env.contract.address.clone().to_string(),
        amount: adjusted_amount_b,
        padding: None,
        memo: None,
    };

    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.erth_token_contract.to_string(),
        code_hash: config.erth_token_hash.clone(),
        msg: to_binary(&transfer_erth_msg)?,
        funds: vec![],
    }));
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.token_b_contract.to_string(),
        code_hash: pool_info.config.token_b_hash.clone(),
        msg: to_binary(&transfer_b_msg)?,
        funds: vec![],
    }));

    // Refund the excess token if any
    if excess_amount > Uint128::from(2u32) {
        let refund_msg = snip20::HandleMsg::Transfer {
            recipient: info.sender.clone().to_string(),
            amount: excess_amount,
            padding: None,
            memo: None,
        };
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: excess_token.to_string(),
            code_hash: if excess_token == config.erth_token_contract {
                config.erth_token_hash.clone()
            } else {
                pool_info.config.token_b_hash.clone()
            },
            msg: to_binary(&refund_msg)?,
            funds: vec![],
        }));
    }

    // Update reserves
    pool_info.state.token_erth_reserve += adjusted_amount_erth;
    pool_info.state.token_b_reserve += adjusted_amount_b;

    pool_info.state.total_shares += shares;

    // Mint LP tokens
    let mint_lp_tokens_msg = snip20::HandleMsg::Mint {
        recipient: info.sender.clone().to_string(),
        amount: shares,
        memo: None,
        padding: None,
    };
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: pool_info.config.lp_token_contract.to_string(),
        code_hash: pool_info.config.lp_token_hash.clone(),
        msg: to_binary(&mint_lp_tokens_msg)?,
        funds: vec![],
    }));

    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "add_liquidity")
        .add_attribute("from", info.sender)
        .add_attribute("shares", shares.to_string())
        .add_attribute("adjusted_amount_erth", adjusted_amount_erth.to_string())
        .add_attribute("adjusted_amount_b", adjusted_amount_b.to_string()))
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
    let amount_erth = (lp_token_amount * pool_info.state.token_erth_reserve) / pool_info.state.total_shares;
    let amount_b = (lp_token_amount * pool_info.state.token_b_reserve) / pool_info.state.total_shares;

    // Update the state reserves and total shares
    pool_info.state.token_erth_reserve -= amount_erth;
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