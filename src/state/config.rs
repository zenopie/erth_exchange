use cosmwasm_std::{Addr, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit_storage::{Item};


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Config {
    pub contract_manager: Addr,
    pub erth_token_contract: Addr,
    pub erth_token_hash: String,
    pub anml_token_contract: Addr,
    pub anml_token_hash: String,
    pub allocation_contract: Addr,
    pub allocation_hash: String,
    pub lp_token_code_id: u64,
    pub lp_token_hash: String,
    pub unbonding_seconds: u64,
    pub protocol_fee: Uint128,
}

pub static CONFIG: Item<Config> = Item::new(b"config");
