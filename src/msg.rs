use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Binary, Uint128, Addr,};

use crate::state::{UserInfo, PoolInfo, PoolConfig, Config,};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct InstantiateMsg {
    pub contract_manager: String,
    pub erth_token_contract: String,
    pub erth_token_hash: String,
    pub anml_token_contract: String,
    pub anml_token_hash: String,
    pub allocation_contract: String,
    pub allocation_hash: String,
    pub lp_token_code_id: u64,
    pub lp_token_hash: String,
    pub unbonding_seconds: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    AddLiquidity {
        amount_erth: Uint128,
        amount_b: Uint128,
        pool: String,
        stake: bool,
    },
    WithdrawLpTokens {
        pool: String,
        amount: Uint128,
        unbond: bool,
    },
    ClaimUnbondLiquidity {
        pool: String,
    },
    ClaimRewards {
        pools: Vec<String>,
    },
    UpdateConfig {
        config: Config,
    },
    AddPool { 
        token: String,
        hash: String,
        symbol: String,
    },
    UpdatePoolConfig { 
        pool: String, 
        pool_config: PoolConfig,
    },
    UpdatePoolRewards {},
    Receive {
        sender: String,
        from: String,
        amount: Uint128,
        memo: Option<String>,
        msg: Binary,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    DepositLpTokens {
        pool: String,
    },
    AllocationSend {
        allocation_id: u32,
    },
    UnbondLiquidity {
        pool: String,
    },
    Swap {
        output_token: String,
        min_received: Option<Uint128>,
        forwarding: Option<Addr>,
    },
    AnmlBuybackSwap {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendMsg {
    ClaimAllocation {
        allocation_id: u32,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrateMsg {
    Migrate {
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    QueryState {},
    QueryConfig {},
    QueryPoolInfo {
        pools: Vec<String>, 
    },
    QueryUserInfo { 
        pools: Vec<String>, 
        user: String,
    },
    QueryUnbondingRequests { 
        pool: String, 
        user: String,
    },
    SimulateSwap {
        input_token: String,
        amount: Uint128,
        output_token: String,
    },
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UserInfoResponse {
    pub pool_info: PoolInfo,
    pub user_info: UserInfo,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SimulateSwapResponse {
    pub output_amount: Uint128,
    pub intermediate_amount: Uint128,   // if double swap, can return it
    pub total_fee: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Snip20InstantiateMsg {
    pub name: String,
    pub admin: Option<String>,
    pub symbol: String,
    pub decimals: u8,
    pub initial_balances: Option<Vec<InitialBalance>>,
    pub prng_seed: Binary,
    pub config: Option<InitConfig>,
    pub supported_denoms: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct InitConfig {
    pub public_total_supply: Option<bool>,
    pub enable_deposit: Option<bool>,
    pub enable_redeem: Option<bool>,
    pub enable_mint: Option<bool>,
    pub enable_burn: Option<bool>,
    pub can_modify_denoms: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct InitialBalance {
    pub address: String,
    pub amount: Uint128,
}