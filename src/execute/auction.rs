use cosmwasm_std::{
    Addr, CosmosMsg, DepsMut, Env, MessageInfo, Reply, Response, StdError, StdResult, SubMsg,
    SubMsgResponse, Uint128, WasmMsg, to_binary,
};
use secret_toolkit::snip20;

use crate::state::{AUCTION, CONTRIBUTIONS, PendingAuction, AuctionInfo};
use crate::msg::Snip20InstantiateMsg;

// Constants for token supply limits.
const MIN_SUPPLY: Uint128 = Uint128::new(1_000);
const MAX_SUPPLY: Uint128 = Uint128::new(1_000_000);

// Create an auction by instantiating a new token.
pub fn create_auction(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    token_supply: Uint128,
    min_liquidity: Uint128,
    auction_duration: u64,
) -> StdResult<Response> {
    if token_supply < MIN_SUPPLY || token_supply > MAX_SUPPLY {
        return Err(StdError::generic_err("Token supply out of range"));
    }

    let auction_end = env.block.time.seconds() + auction_duration;

    let token_name = "Auction Token".to_string();
    let token_symbol = "AUCT".to_string();

    let instantiate_msg = Snip20InstantiateMsg {
        name: token_name.clone(),
        admin: Some(env.contract.address.to_string()),
        symbol: token_symbol,
        decimals: 6,
        initial_balances: None,
        prng_seed: to_binary(&env.block.time.seconds())?,
        config: None,
        supported_denoms: None,
    };

    let token_inst_msg = WasmMsg::Instantiate {
        admin: Some(info.sender.to_string()),
        code_id: 123, // placeholder
        code_hash: "token_code_hash".to_string(), // placeholder
        msg: to_binary(&instantiate_msg)?,
        funds: vec![],
        label: token_name,
    };

    let sub_msg = SubMsg::reply_on_success(CosmosMsg::Wasm(token_inst_msg), 1);

    let auction = AuctionInfo {
        token_supply,
        min_liquidity,
        auction_end,
        total_funds: Uint128::zero(),
        token_contract: None,
        is_resolved: false,
    };
    AUCTION.save(deps.storage, &auction)?;
    PendingAuction.save(deps.storage, &env.contract.address)?;

    Ok(Response::new()
        .add_submessage(sub_msg)
        .add_attribute("action", "create_auction"))
}

// Handle the reply from token instantiation to record the token address.
pub fn handle_instantiate_token_reply(
    deps: DepsMut,
    _env: Env,
    msg: Reply,
) -> StdResult<Response> {
    let mut auction = AUCTION.load(deps.storage)?;
    let res: SubMsgResponse = msg.result.unwrap();

    let token_event = res
        .events
        .iter()
        .find(|e| e.ty == "instantiate")
        .ok_or_else(|| StdError::generic_err("Instantiate event not found"))?;
    let token_addr_attr = token_event
        .attributes
        .iter()
        .find(|a| a.key == "contract_address")
        .ok_or_else(|| StdError::generic_err("Token contract address not found"))?;
    let token_addr = deps.api.addr_validate(&token_addr_attr.value)?;

    auction.token_contract = Some(token_addr.clone());
    AUCTION.save(deps.storage, &auction)?;

    Ok(Response::new()
        .add_attribute("action", "handle_instantiate_token_reply")
        .add_attribute("token_contract", token_addr.to_string()))
}

// Allow users to buy tokens during the auction.
pub fn buy_tokens(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    funds: Uint128,
) -> StdResult<Response> {
    let mut auction = AUCTION.load(deps.storage)?;
    if env.block.time.seconds() > auction.auction_end || auction.is_resolved {
        return Err(StdError::generic_err("Auction inactive"));
    }

    auction.total_funds += funds;
    AUCTION.save(deps.storage, &auction)?;

    let prev = CONTRIBUTIONS.may_load(deps.storage, &info.sender)?.unwrap_or_default();
    CONTRIBUTIONS.save(deps.storage, &info.sender, &(prev + funds))?;

    Ok(Response::new()
        .add_attribute("action", "buy_tokens")
        .add_attribute("buyer", info.sender.to_string())
        .add_attribute("amount", funds.to_string()))
}

// Allow users to refund their contributions if the auction hasn't resolved.
pub fn refund(deps: DepsMut, _env: Env, info: MessageInfo) -> StdResult<Response> {
    let auction = AUCTION.load(deps.storage)?;
    if auction.is_resolved {
        return Err(StdError::generic_err("Auction resolved"));
    }

    let contribution = CONTRIBUTIONS.may_load(deps.storage, &info.sender)?.unwrap_or_default();
    if contribution.is_zero() {
        return Err(StdError::generic_err("No contribution"));
    }
    CONTRIBUTIONS.remove(deps.storage, &info.sender);

    // In practice, trigger a bank refund here.
    Ok(Response::new()
        .add_attribute("action", "refund")
        .add_attribute("buyer", info.sender.to_string())
        .add_attribute("amount", contribution.to_string()))
}

pub fn resolve_auction(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
) -> StdResult<Response> {
    // Validate auction state.
    let mut auction = AUCTION.load(deps.storage)?;
    if env.block.time.seconds() < auction.auction_end {
        return Err(StdError::generic_err("Auction not ended"));
    }
    if auction.is_resolved {
        return Err(StdError::generic_err("Auction already resolved"));
    }
    if auction.total_funds < auction.min_liquidity {
        return Err(StdError::generic_err("Minimum liquidity not reached"));
    }
    auction.is_resolved = true;
    AUCTION.save(deps.storage, &auction)?;

    // Calculate token splits.
    let liquidity_tokens = auction.token_supply.multiply_ratio(50u128, 100u128);
    let distribution_tokens = liquidity_tokens;
    let total_shares = liquidity_tokens + auction.total_funds;

    // Initialize pool state with auction ERTH and pool tokens.
    let pool_state = PoolState {
        total_shares,
        total_staked: Uint128::zero(),
        reward_per_token_scaled: Uint128::zero(),
        pending_volume: Uint128::zero(),
        erth_reserve: auction.total_funds,
        token_b_reserve: liquidity_tokens,
        daily_rewards: [Uint128::zero(); 7],
        last_updated_day: 0,
        unbonding_shares: Uint128::zero(),
    };

    let config = CONFIG.load(deps.storage)?;
    let token_addr = auction.token_contract.clone()
        .ok_or_else(|| StdError::generic_err("Token contract not set"))?;
    let pool_config = PoolConfig {
        token_b_contract: token_addr.clone(),
        token_b_hash: auction.token_hash.clone(),
        token_b_symbol: auction.token_symbol.clone(),
        lp_token_contract: Addr::unchecked(""), // To be updated on reply.
        lp_token_hash: config.lp_token_hash.clone(),
    };
    let pool_info = PoolInfo { state: pool_state, config: pool_config };
    POOL_INFO.insert(deps.storage, &token_addr, &pool_info)?;
    PENDING_POOL.save(deps.storage, &token_addr)?;

    // Instantiate LP token.
    let lp_token_name = format!("ERTH-{} LP", auction.token_symbol);
    let lp_token_symbol = format!("{}LP", auction.token_symbol);
    let init_config = InitConfig {
        public_total_supply: Some(true),
        enable_deposit: Some(false),
        enable_redeem: Some(false),
        enable_mint: Some(true),
        enable_burn: Some(true),
        can_modify_denoms: Some(false),
    };
    let lp_token_instantiate_msg = Snip20InstantiateMsg {
        name: lp_token_name.clone(),
        admin: Some(env.contract.address.to_string()),
        symbol: lp_token_symbol,
        decimals: 6,
        initial_balances: None,
        prng_seed: to_binary(&env.block.time.seconds())?,
        config: Some(init_config),
        supported_denoms: None,
    };
    let lp_token_msg = WasmMsg::Instantiate {
        admin: Some(config.contract_manager.to_string()),
        code_id: config.lp_token_code_id,
        code_hash: config.lp_token_hash.clone(),
        msg: to_binary(&lp_token_instantiate_msg)?,
        funds: vec![],
        label: lp_token_name,
    };
    let sub_msg_lp =
        SubMsg::reply_on_success(CosmosMsg::Wasm(lp_token_msg), INSTANTIATE_LP_TOKEN_REPLY_ID);

    // Register this contract as a receiver for the auction token.
    let register_pool_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: token_addr.to_string(),
        code_hash: auction.token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::RegisterReceive {
            code_hash: env.contract.code_hash.clone(),
            padding: None,
        })?,
        funds: vec![],
    });

    // Batch distribute distribution_tokens based on each contributor's ERTH share.
    let mut transfers = vec![];
    for item in CONTRIBUTIONS.range(deps.storage, None, None, cosmwasm_std::Order::Ascending) {
        let (addr, contribution) = item?;
        let allocation = contribution.multiply_ratio(distribution_tokens, auction.total_funds);
        transfers.push(snip20::BatchRecipient {
            recipient: addr.to_string(),
            amount: allocation,
        });
        CONTRIBUTIONS.remove(deps.storage, &addr);
    }
    let batch_send_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: token_addr.to_string(),
        code_hash: auction.token_hash.clone(),
        msg: to_binary(&snip20::HandleMsg::BatchSend { transfers })?,
        funds: vec![],
    });

    Ok(Response::new()
        .add_submessage(sub_msg_lp)
        .add_message(register_pool_msg)
        .add_message(batch_send_msg)
        .add_attribute("action", "resolve_auction")
        .add_attribute("erth_liquidity", auction.total_funds.to_string())
        .add_attribute("pool_token", liquidity_tokens.to_string())
        .add_attribute("total_shares", total_shares.to_string())
        .add_attribute("distributed_tokens", distribution_tokens.to_string()))
}

