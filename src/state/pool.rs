use cosmwasm_std::{Addr, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit_storage::{Item, Keymap};


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct PoolState {
    pub total_shares: Uint128,
    pub total_staked: Uint128,
    pub reward_per_token_scaled: Uint128,
    pub daily_volumes: [Uint128; 8],
    pub daily_rewards: [Uint128; 8],
    pub last_updated_day: u64,
    pub erth_reserve: Uint128,
    pub token_b_reserve: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct PoolConfig {
    pub token_b_contract: Addr,
    pub token_b_hash: String,
    pub token_b_symbol: String,
    pub lp_token_contract: Addr,
    pub lp_token_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct PoolInfo {
    pub state: PoolState,
    pub config: PoolConfig,
}

pub static POOL_INFO: Keymap<Addr, PoolInfo> = Keymap::new(b"pool_info");


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct UserInfo {
    pub amount_staked: Uint128,
    pub reward_debt: Uint128,
    pub pending_rewards: Uint128,
}

pub static USER_INFO: Keymap<Addr, UserInfo> = Keymap::new(b"user_info");

pub static PENDING_POOL: Item<Addr> = Item::new(b"pending_pool");

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Order {
    pub user: Option<Addr>,
    pub input_remaining: Uint128,
}

pub static BUY_ORDERS: Keymap<Uint128, Vec<Order>> = Keymap::new(b"buy_orders");

pub static SELL_ORDERS: Keymap<Uint128, Vec<Order>> = Keymap::new(b"sell_orders");