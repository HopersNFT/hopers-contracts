use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Decimal, Uint128};
use cw20::Cw20ReceiveMsg;

use crate::state::{StakerInfo, UnbondingInfo};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub lp_token_contract: String,
    pub reward_token_contract: String,
    pub distribution_schedule: Vec<(u64, u64, Uint128)>,
    pub lock_duration: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    Receive(Cw20ReceiveMsg),
    Unbond {
        amount: Uint128,
    },
    /// Withdraw pending rewards
    Withdraw {},
    Redeem {},
    /// Owner operation to stop distribution on current staking contract
    /// and send remaining tokens to the new contract
    MigrateStaking {
        new_staking_contract: String,
    },
    UpdateConfig {
        distribution_schedule: Vec<(u64, u64, Uint128)>,
    },
    UpdateAdmin {
        admin: String,
    },
    UpdateTokenContract {
        lp_token_contract: String,
        reward_token_contract: String,
    },
    UpdateLockDuration {
        lock_duration: u64,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    Bond {},
}

/// migrate struct for distribution schedule
/// block-based schedule to a time-based schedule
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    State {
        block_time: Option<u64>,
    },
    StakerInfo {
        staker: String,
    },
    AllStakers {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    UnbondingInfo {
        staker: String,
        start_after: Option<u64>,
        limit: Option<u32>,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub lp_token_contract: String,
    pub reward_token_contract: String,
    pub distribution_schedule: Vec<(u64, u64, Uint128)>,
    pub admin: String,
    pub lock_duration: u64,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StateResponse {
    pub last_distributed: u64,
    pub total_bond_amount: Uint128,
    pub global_reward_index: Decimal,
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StakerInfoResponse {
    pub staker: String,
    pub reward_index: Decimal,
    pub bond_amount: Uint128,
    pub pending_reward: Uint128,
    pub total_earned: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct StakersListResponse {
    pub stakers_list: Vec<StakerInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UnbondingInfoResponse {
    pub unbonding_info: Vec<UnbondingInfo>,
    pub crr_time: u64,
}
