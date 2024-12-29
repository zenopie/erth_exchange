// src/state/mod.rs

pub mod config;
pub mod pool;

pub use config::{Config, CONFIG,};
pub use pool::{PoolInfo, POOL_INFO, UserInfo, USER_INFO, PoolConfig, PENDING_POOL, PoolState};

use cosmwasm_std::{Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit_storage::{Item};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct State {
    pub erth_burned: Uint128,
    pub anml_burned: Uint128,
    pub daily_total_volumes: [Uint128; 8],
    pub daily_total_rewards: [Uint128; 8],
    pub pending_reward: Uint128,
    pub last_updated_day: u64,
}

pub static STATE: Item<State> = Item::new(b"state");

