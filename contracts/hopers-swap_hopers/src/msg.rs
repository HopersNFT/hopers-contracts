use cosmwasm_schema::cw_serde;
use cosmwasm_schema::QueryResponses;

use cosmwasm_std::{Decimal, Uint128};

use cw20::{Denom, Expiration};

#[cw_serde]
pub struct InstantiateMsg {
    pub token1_denom: Denom,
    pub token2_denom: Denom,
    pub lp_token_code_id: u64,
    pub owner: Option<String>,
    // NOTE: Fees percents are out of 100 e.g., 1 = 1%
    pub fee_percent_numerator: Uint128,
    pub burn_fee_percent_numerator: Uint128,
    pub fee_percent_denominator: Uint128,
    pub lp_token_name: String,
    pub lp_token_symbol: String,
    pub dev_wallet_lists: Vec<WalletInfo>,
}

#[cw_serde]
pub enum TokenSelect {
    Token1,
    Token2,
}

#[cw_serde]
pub enum ExecuteMsg {
    AddLiquidity {
        token1_amount: Uint128,
        min_liquidity: Uint128,
        max_token2: Uint128,
        expiration: Option<Expiration>,
    },
    RemoveLiquidity {
        amount: Uint128,
        min_token1: Uint128,
        min_token2: Uint128,
        expiration: Option<Expiration>,
    },
    Swap {
        input_token: TokenSelect,
        input_amount: Uint128,
        min_output: Uint128,
        expiration: Option<Expiration>,
    },
    /// Chained swap converting A -> B and B -> C by leveraging two swap contracts
    PassThroughSwap {
        output_amm_address: String,
        input_token: TokenSelect,
        input_token_amount: Uint128,
        output_min_token: Uint128,
        expiration: Option<Expiration>,
    },
    SwapAndSendTo {
        input_token: TokenSelect,
        input_amount: Uint128,
        recipient: String,
        min_token: Uint128,
        expiration: Option<Expiration>,
    },
    UpdateConfig {
        owner: Option<String>,
        fee_percent_numerator: Uint128,
        burn_fee_percent_numerator: Uint128,
        fee_percent_denominator: Uint128,
        dev_wallet_lists: Vec<WalletInfo>,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Implements CW20. Returns the current balance of the given address, 0 if unset.
    #[returns(BalanceResponse)]
    Balance {
        address: String,
    },
    #[returns(InfoResponse)]
    Info {},
    #[returns(Token1ForToken2PriceResponse)]
    Token1ForToken2Price {
        token1_amount: Uint128,
    },
    #[returns(Token2ForToken1PriceResponse)]
    Token2ForToken1Price {
        token2_amount: Uint128,
    },
    #[returns(FeeResponse)]
    Fee {},
}
#[cw_serde]
pub struct MigrateMsg {
    pub burn_fee_percent_numerator: Uint128,
}


#[cw_serde]
pub struct BalanceResponse {
    pub balance: Uint128,
}

#[cw_serde]
pub struct InfoResponse {
    pub token1_reserve: Uint128,
    pub token1_denom: Denom,
    pub token2_reserve: Uint128,
    pub token2_denom: Denom,
    pub lp_token_supply: Uint128,
    pub lp_token_address: String,
}

#[cw_serde]
pub struct FeeResponse {
    pub owner: Option<String>,
    pub total_fee_percent: Decimal,
    pub dev_wallet_lists: Vec<WalletInfo>,
}

#[cw_serde]
pub struct Token1ForToken2PriceResponse {
    pub token2_amount: Uint128,
}

#[cw_serde]
pub struct Token2ForToken1PriceResponse {
    pub token1_amount: Uint128,
}

#[cw_serde]
#[derive(Eq)]
pub struct WalletInfo {
    pub address: String,
    pub ratio: Decimal,
}
