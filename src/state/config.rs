use cosmwasm_std::{Addr, Uint128, Deps, StdResult, to_binary, QueryRequest, WasmQuery};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit_storage::Item;


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Config {
    pub contract_manager: Addr,
    pub registry_contract: Addr,
    pub registry_hash: String,
    pub unbonding_seconds: u64,
    pub unbonding_window: u64,
    pub protocol_fee: Uint128,
}

pub static CONFIG: Item<Config> = Item::new(b"config");

// Minimal registry types
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryQueryMsg {
    GetContracts { names: Vec<String> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ContractInfo {
    pub address: Addr,
    pub code_hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct ContractResponseItem {
    pub name: String,
    pub info: ContractInfo,
}

#[derive(Serialize, Deserialize)]
pub struct AllContractsResponse {
    pub contracts: Vec<ContractResponseItem>,
}

/// All contract references resolved from the registry
#[derive(Clone, Debug)]
pub struct ContractAddresses {
    pub erth_token: ContractInfo,
    pub anml_token: ContractInfo,
    pub staking: ContractInfo,
    pub sscrt_token: ContractInfo,
}

pub fn query_registry(
    deps: &Deps,
    registry_addr: &Addr,
    registry_hash: &str,
    names: Vec<&str>,
) -> StdResult<Vec<ContractInfo>> {
    let query_msg = RegistryQueryMsg::GetContracts {
        names: names.iter().map(|n| n.to_string()).collect(),
    };
    let response: AllContractsResponse = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: registry_addr.to_string(),
        code_hash: registry_hash.to_string(),
        msg: to_binary(&query_msg)?,
    }))?;
    Ok(response.contracts.into_iter().map(|c| c.info).collect())
}

/// Load all contract addresses from registry in one query
pub fn load_contracts(deps: &Deps, config: &Config) -> StdResult<ContractAddresses> {
    let contracts = query_registry(
        deps,
        &config.registry_contract,
        &config.registry_hash,
        vec!["erth_token", "anml_token", "staking", "sscrt_token"],
    )?;
    Ok(ContractAddresses {
        erth_token: contracts[0].clone(),
        anml_token: contracts[1].clone(),
        staking: contracts[2].clone(),
        sscrt_token: contracts[3].clone(),
    })
}
