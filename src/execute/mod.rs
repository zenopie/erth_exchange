// src/execute/mod.rs
pub mod update_config;
pub mod liquidity;
pub mod pool;
pub mod rewards;
pub mod swap;


pub use rewards::{update_user_rewards, update_pool_rewards};
pub use pool::{handle_instantiate_lp_token_reply};

use cosmwasm_std::{DepsMut, Env, MessageInfo, Response, StdResult, StdError, Uint128,
    from_binary, Binary};
use crate::msg::{ExecuteMsg, ReceiveMsg};
use crate::state::{STATE, CONFIG};

pub fn execute_dispatch(
    deps: DepsMut, 
    env: Env, 
    info: MessageInfo, 
    msg: ExecuteMsg
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::UpdateConfig { config } => update_config::update_config(deps, env, info, config),
        ExecuteMsg::ClaimRewards {pools} => rewards::claim_rewards(deps, env, info, pools),
        ExecuteMsg::AddLiquidity { amount_erth, amount_b, pool, stake } =>
            liquidity::add_liquidity(deps, env, info, amount_erth, amount_b, pool, stake),
        ExecuteMsg::WithdrawLpTokens { pool, amount, unbond } => rewards::withdraw_lp_tokens(deps, env, info, pool, amount, unbond),
        ExecuteMsg::AddPool {token, hash, symbol} => pool::add_pool(deps, env, info, token, hash, symbol),
        ExecuteMsg::UpdatePoolConfig { pool, pool_config } => 
            pool::update_pool_config(deps, info, pool, pool_config),
        ExecuteMsg::Receive { sender, from, amount, msg, memo: _ } => 
            recieve_dispatch(deps, env, info, sender, from, amount, msg),
    }
}

pub fn recieve_dispatch(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sender: String,
    from: String,
    amount: Uint128,
    msg: Binary,
) -> Result<Response, StdError> {
    let msg: ReceiveMsg = from_binary(&msg)?;

    let _sender_addr = deps.api.addr_validate(&sender)?;
    let from_addr = deps.api.addr_validate(&from)?;

    match msg {
        ReceiveMsg::DepositLpTokens {pool} => rewards::deposit_lp_tokens(deps, env, info, from_addr, amount, pool),
        ReceiveMsg::UnbondLiquidity {pool} => liquidity::unbond_liquidity(deps, env, info, from_addr, amount, pool),
        ReceiveMsg::Swap {output_token, ..} => swap::swap(deps, env, info, from_addr, amount, output_token,),
        ReceiveMsg::AnmlBuybackSwap {} => swap::anml_buyback_swap(deps, env, info, amount),
        ReceiveMsg::AllocationSend { allocation_id } => recieve_allocation(deps, env, info, amount, allocation_id),
    }
}

fn recieve_allocation(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    amount: Uint128,
    _allocation_id: u32,
) -> Result<Response, StdError> {

    // Load the state
    let mut state = STATE.load(deps.storage)?;
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.erth_token_contract {
        return Err(StdError::generic_err("invalid token"));
    }

    state.pending_reward += amount;
    STATE.save(deps.storage, &state)?;

    Ok(Response::new()
            .add_attribute("action", "claim_allocation")
            .add_attribute("claim_allocation_amount", amount.to_string()))
}