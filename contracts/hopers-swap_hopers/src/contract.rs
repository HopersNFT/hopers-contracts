use cosmwasm_std::{
    attr, entry_point, to_binary, Addr, Binary, BlockInfo, Coin, CosmosMsg, Decimal, Deps, DepsMut,
    Env, MessageInfo, Reply, Response, StdError, StdResult, SubMsg, Uint128,
    WasmMsg,
};
use cw0::parse_reply_instantiate_data;
use cw2::{get_contract_version, set_contract_version};
use cw20::Denom::Cw20;
use cw20::{Cw20ExecuteMsg, Denom, Expiration, MinterResponse};
use cw20_base::contract::query_balance;
use std::str::FromStr;

use crate::error::ContractError;
use crate::msg::{
    ExecuteMsg, FeeResponse, InfoResponse, InstantiateMsg, MigrateMsg, QueryMsg,
    Token1ForToken2PriceResponse, Token2ForToken1PriceResponse, TokenSelect, WalletInfo,
};
use crate::state::{Fees, Token, BURN_FEE_INFO, FEES, LP_TOKEN, OWNER, TOKEN1, TOKEN2};

// Version info for migration info
pub const CONTRACT_NAME: &str = "crates.io:wasmswap";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const INSTANTIATE_LP_TOKEN_REPLY_ID: u64 = 0;

//const FEE_SCALE_FACTOR: Uint128 = Uint128::new(10_000);
const MAX_FEE_PERCENT: &str = "1";
//const FEE_DECIMAL_PRECISION: Uint128 = Uint128::new(10u128.pow(20));

// Note, you can use StdResult in some functions where you do not
// make use of the custom errors
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let token1 = Token {
        reserve: Uint128::zero(),
        denom: msg.token1_denom.clone(),
    };
    TOKEN1.save(deps.storage, &token1)?;

    let token2 = Token {
        denom: msg.token2_denom.clone(),
        reserve: Uint128::zero(),
    };
    TOKEN2.save(deps.storage, &token2)?;

    let owner = msg.owner.map(|h| deps.api.addr_validate(&h)).transpose()?;
    OWNER.save(deps.storage, &owner)?;

    let mut total_ratio = Decimal::zero();
    for dev_wallet in msg.dev_wallet_lists.clone() {
        deps.api.addr_validate(&dev_wallet.address)?;
        total_ratio = total_ratio + dev_wallet.ratio;
    }

    if total_ratio != Decimal::one() {
        return Err(ContractError::WrongRatio {});
    }

    let num = msg.fee_percent_numerator.u128();
    let den = msg.fee_percent_denominator.u128();
    let total_fee_percent = Decimal::from_ratio(num, den);
    let max_fee_percent = Decimal::from_str(MAX_FEE_PERCENT)?;

    if total_fee_percent > max_fee_percent {
        return Err(ContractError::FeesTooHigh {
            max_fee_percent,
            total_fee_percent,
        });
    }

    let fees = Fees {
        fee_percent_numerator: msg.fee_percent_numerator,
        fee_percent_denominator: msg.fee_percent_denominator,
        dev_wallet_lists: msg.dev_wallet_lists,
    };
    FEES.save(deps.storage, &fees)?;

    BURN_FEE_INFO.save(deps.storage, &msg.burn_fee_percent_numerator)?;

    let instantiate_lp_token_msg = WasmMsg::Instantiate {
        code_id: msg.lp_token_code_id,
        funds: vec![],
        admin: Some(info.sender.to_string()),
        label: msg.lp_token_name.clone(),
        msg: to_binary(&cw20_base::msg::InstantiateMsg {
            name: msg.lp_token_name,
            symbol: msg.lp_token_symbol,
            decimals: 6,
            initial_balances: vec![],
            mint: Some(MinterResponse {
                minter: env.contract.address.into(),
                cap: None,
            }),
            marketing: None,
        })?,
    };

    let reply_msg =
        SubMsg::reply_on_success(instantiate_lp_token_msg, INSTANTIATE_LP_TOKEN_REPLY_ID);

    Ok(Response::new().add_submessage(reply_msg))
}

// And declare a custom Error variant for the ones where you will want to make use of it
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::AddLiquidity {
            token1_amount,
            min_liquidity,
            max_token2,
            expiration,
        } => execute_add_liquidity(
            deps,
            &info,
            env,
            min_liquidity,
            token1_amount,
            max_token2,
            expiration,
        ),
        ExecuteMsg::RemoveLiquidity {
            amount,
            min_token1,
            min_token2,
            expiration,
        } => execute_remove_liquidity(deps, info, env, amount, min_token1, min_token2, expiration),
        ExecuteMsg::Swap {
            input_token,
            input_amount,
            min_output,
            expiration,
            ..
        } => execute_swap(
            deps,
            &info,
            input_amount,
            env,
            input_token,
            info.sender.to_string(),
            min_output,
            expiration,
        ),
        ExecuteMsg::PassThroughSwap {
            output_amm_address,
            input_token,
            input_token_amount,
            output_min_token,
            expiration,
        } => execute_pass_through_swap(
            deps,
            info,
            env,
            output_amm_address,
            input_token,
            input_token_amount,
            output_min_token,
            expiration,
        ),
        ExecuteMsg::SwapAndSendTo {
            input_token,
            input_amount,
            recipient,
            min_token,
            expiration,
        } => execute_swap(
            deps,
            &info,
            input_amount,
            env,
            input_token,
            recipient,
            min_token,
            expiration,
        ),
        ExecuteMsg::UpdateConfig {
            owner,
            dev_wallet_lists,
            fee_percent_numerator,
            burn_fee_percent_numerator,
            fee_percent_denominator,
        } => execute_update_config(
            deps,
            info,
            owner,
            fee_percent_numerator,
            burn_fee_percent_numerator,
            fee_percent_denominator,
            dev_wallet_lists,
        ),
    }
}

fn check_expiration(
    expiration: &Option<Expiration>,
    block: &BlockInfo,
) -> Result<(), ContractError> {
    match expiration {
        Some(e) => {
            if e.is_expired(block) {
                return Err(ContractError::MsgExpirationError {});
            }
            Ok(())
        }
        None => Ok(()),
    }
}

fn get_lp_token_amount_to_mint(
    token1_amount: Uint128,
    liquidity_supply: Uint128,
    token1_reserve: Uint128,
) -> Result<Uint128, ContractError> {
    if liquidity_supply == Uint128::zero() {
        Ok(token1_amount)
    } else {
        Ok(token1_amount
            .checked_mul(liquidity_supply)
            .map_err(StdError::overflow)?
            .checked_div(token1_reserve)
            .map_err(StdError::divide_by_zero)?)
    }
}

fn get_token2_amount_required(
    max_token: Uint128,
    token1_amount: Uint128,
    liquidity_supply: Uint128,
    token2_reserve: Uint128,
    token1_reserve: Uint128,
) -> Result<Uint128, StdError> {
    if liquidity_supply == Uint128::zero() {
        Ok(max_token)
    } else {
        Ok(token1_amount
            .checked_mul(token2_reserve)
            .map_err(StdError::overflow)?
            .checked_div(token1_reserve)
            .map_err(StdError::divide_by_zero)?
            .checked_add(Uint128::new(1))
            .map_err(StdError::overflow)?)
    }
}

pub fn execute_add_liquidity(
    deps: DepsMut,
    info: &MessageInfo,
    env: Env,
    min_liquidity: Uint128,
    token1_amount: Uint128,
    max_token2: Uint128,
    expiration: Option<Expiration>,
) -> Result<Response, ContractError> {
    let action = "add_liquidity".to_string();
    check_expiration(&expiration, &env.block)?;

    let token1 = TOKEN1.load(deps.storage)?;
    let token2 = TOKEN2.load(deps.storage)?;
    let lp_token_addr = LP_TOKEN.load(deps.storage)?;

    // validate funds
    validate_input_amount(&info.funds, token1_amount, &token1.denom)?;
    validate_input_amount(&info.funds, max_token2, &token2.denom)?;

    let lp_token_supply = get_lp_token_supply(deps.as_ref(), &lp_token_addr)?;
    let liquidity_amount =
        get_lp_token_amount_to_mint(token1_amount, lp_token_supply, token1.reserve)?;

    let token2_amount = get_token2_amount_required(
        max_token2,
        token1_amount,
        lp_token_supply,
        token2.reserve,
        token1.reserve,
    )?;

    if liquidity_amount < min_liquidity {
        return Err(ContractError::MinLiquidityError {
            min_liquidity,
            liquidity_available: liquidity_amount,
        });
    }

    if token2_amount > max_token2 {
        return Err(ContractError::MaxTokenError {
            max_token: max_token2,
            tokens_required: token2_amount,
        });
    }

    // Generate cw20 transfer messages if necessary
    let mut transfer_msgs: Vec<CosmosMsg> = vec![];
    if let Cw20(addr) = token1.denom {
        transfer_msgs.push(get_cw20_transfer_from_msg(
            &info.sender,
            &env.contract.address,
            &addr,
            token1_amount,
        )?)
    }
    if let Cw20(addr) = token2.denom.clone() {
        transfer_msgs.push(get_cw20_transfer_from_msg(
            &info.sender,
            &env.contract.address,
            &addr,
            token2_amount,
        )?)
    }

    // Refund token 2 if is a native token and not all is spent
    if let Denom::Native(denom) = token2.denom {
        if token2_amount < max_token2 {
            transfer_msgs.push(get_bank_transfer_to_msg(
                &info.sender,
                &denom,
                max_token2 - token2_amount,
            ))
        }
    }

    TOKEN1.update(deps.storage, |mut token1| -> Result<_, ContractError> {
        token1.reserve += token1_amount;
        Ok(token1)
    })?;
    TOKEN2.update(deps.storage, |mut token2| -> Result<_, ContractError> {
        token2.reserve += token2_amount;
        Ok(token2)
    })?;

    let mint_msg = mint_lp_tokens(&info.sender, liquidity_amount, &lp_token_addr)?;
    Ok(Response::new()
        .add_messages(transfer_msgs)
        .add_message(mint_msg)
        .add_attributes(vec![
            attr("action", action),
            attr("token1_amount", token1_amount),
            attr("token2_amount", token2_amount),
            attr("liquidity_received", liquidity_amount),
        ]))
}

fn get_lp_token_supply(deps: Deps, lp_token_addr: &Addr) -> StdResult<Uint128> {
    let resp: cw20::TokenInfoResponse = deps
        .querier
        .query_wasm_smart(lp_token_addr, &cw20_base::msg::QueryMsg::TokenInfo {})?;
    Ok(resp.total_supply)
}

fn mint_lp_tokens(
    recipient: &Addr,
    liquidity_amount: Uint128,
    lp_token_address: &Addr,
) -> StdResult<CosmosMsg> {
    let mint_msg = cw20_base::msg::ExecuteMsg::Mint {
        recipient: recipient.into(),
        amount: liquidity_amount,
    };
    Ok(WasmMsg::Execute {
        contract_addr: lp_token_address.to_string(),
        msg: to_binary(&mint_msg)?,
        funds: vec![],
    }
    .into())
}

fn get_token_balance(deps: Deps, contract: &Addr, addr: &Addr) -> StdResult<Uint128> {
    let resp: cw20::BalanceResponse = deps.querier.query_wasm_smart(
        contract,
        &cw20_base::msg::QueryMsg::Balance {
            address: addr.to_string(),
        },
    )?;
    Ok(resp.balance)
}

fn validate_input_amount(
    actual_funds: &[Coin],
    given_amount: Uint128,
    given_denom: &Denom,
) -> Result<(), ContractError> {
    match given_denom {
        Denom::Cw20(_) => Ok(()),
        Denom::Native(denom) => {
            let actual = get_amount_for_denom(actual_funds, denom);
            if actual.amount != given_amount {
                return Err(ContractError::InsufficientFunds {});
            }
            if &actual.denom != denom {
                return Err(ContractError::IncorrectNativeDenom {
                    provided: actual.denom,
                    required: denom.to_string(),
                });
            };
            Ok(())
        }
    }
}

fn get_cw20_transfer_from_msg(
    owner: &Addr,
    recipient: &Addr,
    token_addr: &Addr,
    token_amount: Uint128,
) -> StdResult<CosmosMsg> {
    // create transfer cw20 msg
    let transfer_cw20_msg = Cw20ExecuteMsg::TransferFrom {
        owner: owner.into(),
        recipient: recipient.into(),
        amount: token_amount,
    };
    let exec_cw20_transfer = WasmMsg::Execute {
        contract_addr: token_addr.into(),
        msg: to_binary(&transfer_cw20_msg)?,
        funds: vec![],
    };
    let cw20_transfer_cosmos_msg: CosmosMsg = exec_cw20_transfer.into();
    Ok(cw20_transfer_cosmos_msg)
}

fn get_cw20_burn_from_msg(
    owner: &Addr,
    token_addr: &Addr,
    token_amount: Uint128,
) -> StdResult<CosmosMsg> {
    // create transfer cw20 msg
    let transfer_cw20_msg = Cw20ExecuteMsg::BurnFrom {
        owner: owner.into(),
        amount: token_amount,
    };
    let exec_cw20_transfer = WasmMsg::Execute {
        contract_addr: token_addr.into(),
        msg: to_binary(&transfer_cw20_msg)?,
        funds: vec![],
    };
    let cw20_transfer_cosmos_msg: CosmosMsg = exec_cw20_transfer.into();
    Ok(cw20_transfer_cosmos_msg)
}

fn get_cw20_burn_msg(token_addr: &Addr, token_amount: Uint128) -> StdResult<CosmosMsg> {
    // create transfer cw20 msg
    let transfer_cw20_msg = Cw20ExecuteMsg::Burn {
        amount: token_amount,
    };
    let exec_cw20_transfer = WasmMsg::Execute {
        contract_addr: token_addr.into(),
        msg: to_binary(&transfer_cw20_msg)?,
        funds: vec![],
    };
    let cw20_transfer_cosmos_msg: CosmosMsg = exec_cw20_transfer.into();
    Ok(cw20_transfer_cosmos_msg)
}

fn get_cw20_increase_allowance_msg(
    token_addr: &Addr,
    spender: &Addr,
    amount: Uint128,
    expires: Option<Expiration>,
) -> StdResult<CosmosMsg> {
    // create transfer cw20 msg
    let increase_allowance_msg = Cw20ExecuteMsg::IncreaseAllowance {
        spender: spender.to_string(),
        amount,
        expires,
    };
    let exec_allowance = WasmMsg::Execute {
        contract_addr: token_addr.into(),
        msg: to_binary(&increase_allowance_msg)?,
        funds: vec![],
    };
    Ok(exec_allowance.into())
}

pub fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    new_owner: Option<String>,
    fee_percent_numerator: Uint128,
    burn_fee_percent_numerator: Uint128,
    fee_percent_denominator: Uint128,
    dev_wallet_lists: Vec<WalletInfo>,
) -> Result<Response, ContractError> {
    let owner = OWNER.load(deps.storage)?;
    if Some(info.sender) != owner {
        return Err(ContractError::Unauthorized {});
    }

    let new_owner_addr = new_owner
        .as_ref()
        .map(|h| deps.api.addr_validate(h))
        .transpose()?;
    OWNER.save(deps.storage, &new_owner_addr)?;

    let num = fee_percent_numerator.u128();
    let den = fee_percent_denominator.u128();

    let total_fee_percent = Decimal::from_ratio(num, den);
    let max_fee_percent = Decimal::from_str(MAX_FEE_PERCENT)?;
    if total_fee_percent > max_fee_percent {
        return Err(ContractError::FeesTooHigh {
            max_fee_percent,
            total_fee_percent,
        });
    }

    let mut total_ratio = Decimal::zero();
    for dev_wallet in dev_wallet_lists.clone() {
        deps.api.addr_validate(&dev_wallet.address)?;
        total_ratio = total_ratio + dev_wallet.ratio;
    }

    if total_ratio != Decimal::one() {
        return Err(ContractError::WrongRatio {});
    }

    let updated_fees = Fees {
        dev_wallet_lists,
        fee_percent_numerator,
        fee_percent_denominator,
    };
    FEES.save(deps.storage, &updated_fees)?;

    BURN_FEE_INFO.save(deps.storage, &burn_fee_percent_numerator)?;

    let new_owner = new_owner.unwrap_or_else(|| "".to_string());
    Ok(Response::new().add_attributes(vec![
        attr("new_owner", new_owner),
        attr("fee_percent", total_fee_percent.to_string()),
    ]))
}

pub fn execute_remove_liquidity(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    amount: Uint128,
    min_token1: Uint128,
    min_token2: Uint128,
    expiration: Option<Expiration>,
) -> Result<Response, ContractError> {
    let action = "remove_liquidity".to_string();
    check_expiration(&expiration, &env.block)?;

    let lp_token_addr = LP_TOKEN.load(deps.storage)?;
    let balance = get_token_balance(deps.as_ref(), &lp_token_addr, &info.sender)?;
    let lp_token_supply = get_lp_token_supply(deps.as_ref(), &lp_token_addr)?;
    let token1 = TOKEN1.load(deps.storage)?;
    let token2 = TOKEN2.load(deps.storage)?;

    if amount > balance {
        return Err(ContractError::InsufficientLiquidityError {
            requested: amount,
            available: balance,
        });
    }

    let token1_amount = amount
        .checked_mul(token1.reserve)
        .map_err(StdError::overflow)?
        .checked_div(lp_token_supply)
        .map_err(StdError::divide_by_zero)?;
    if token1_amount < min_token1 {
        return Err(ContractError::MinToken1Error {
            requested: min_token1,
            available: token1_amount,
        });
    }

    let token2_amount = amount
        .checked_mul(token2.reserve)
        .map_err(StdError::overflow)?
        .checked_div(lp_token_supply)
        .map_err(StdError::divide_by_zero)?;
    if token2_amount < min_token2 {
        return Err(ContractError::MinToken2Error {
            requested: min_token2,
            available: token2_amount,
        });
    }

    TOKEN1.update(deps.storage, |mut token1| -> Result<_, ContractError> {
        token1.reserve = token1
            .reserve
            .checked_sub(token1_amount)
            .map_err(StdError::overflow)?;
        Ok(token1)
    })?;

    TOKEN2.update(deps.storage, |mut token2| -> Result<_, ContractError> {
        token2.reserve = token2
            .reserve
            .checked_sub(token2_amount)
            .map_err(StdError::overflow)?;
        Ok(token2)
    })?;

    let token1_transfer_msg = match token1.denom {
        Denom::Cw20(addr) => get_cw20_transfer_to_msg(&info.sender, &addr, token1_amount)?,
        Denom::Native(denom) => get_bank_transfer_to_msg(&info.sender, &denom, token1_amount),
    };
    let token2_transfer_msg = match token2.denom {
        Denom::Cw20(addr) => get_cw20_transfer_to_msg(&info.sender, &addr, token2_amount)?,
        Denom::Native(denom) => get_bank_transfer_to_msg(&info.sender, &denom, token2_amount),
    };

    let lp_token_burn_msg = get_burn_msg(&lp_token_addr, &info.sender, amount)?;

    Ok(Response::new()
        .add_messages(vec![
            token1_transfer_msg,
            token2_transfer_msg,
            lp_token_burn_msg,
        ])
        .add_attributes(vec![
            attr("action", action),
            attr("liquidity_burned", amount),
            attr("token1_returned", token1_amount),
            attr("token2_returned", token2_amount),
        ]))
}

fn get_burn_msg(contract: &Addr, owner: &Addr, amount: Uint128) -> StdResult<CosmosMsg> {
    let msg = cw20_base::msg::ExecuteMsg::BurnFrom {
        owner: owner.to_string(),
        amount,
    };
    Ok(WasmMsg::Execute {
        contract_addr: contract.to_string(),
        msg: to_binary(&msg)?,
        funds: vec![],
    }
    .into())
}

fn get_cw20_transfer_to_msg(
    recipient: &Addr,
    token_addr: &Addr,
    token_amount: Uint128,
) -> StdResult<CosmosMsg> {
    // create transfer cw20 msg
    let transfer_cw20_msg = Cw20ExecuteMsg::Transfer {
        recipient: recipient.into(),
        amount: token_amount,
    };
    let exec_cw20_transfer = WasmMsg::Execute {
        contract_addr: token_addr.into(),
        msg: to_binary(&transfer_cw20_msg)?,
        funds: vec![],
    };
    let cw20_transfer_cosmos_msg: CosmosMsg = exec_cw20_transfer.into();
    Ok(cw20_transfer_cosmos_msg)
}

fn get_bank_transfer_to_msg(recipient: &Addr, denom: &str, native_amount: Uint128) -> CosmosMsg {
    let transfer_bank_msg = cosmwasm_std::BankMsg::Send {
        to_address: recipient.into(),
        amount: vec![Coin {
            denom: denom.to_string(),
            amount: native_amount,
        }],
    };

    let transfer_bank_cosmos_msg: CosmosMsg = transfer_bank_msg.into();
    transfer_bank_cosmos_msg
}

fn get_fee_transfer_msg(
    sender: &Addr,
    recipient: &Addr,
    fee_denom: &Denom,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    match fee_denom {
        Denom::Cw20(addr) => get_cw20_transfer_from_msg(sender, recipient, addr, amount),
        Denom::Native(denom) => Ok(get_bank_transfer_to_msg(recipient, denom, amount)),
    }
}

/* 
fn fee_decimal_to_uint128(decimal: Decimal) -> StdResult<Uint128> {
    let result: Uint128 = decimal
        .atomics()
        .checked_mul(FEE_SCALE_FACTOR)
        .map_err(StdError::overflow)?;

    Ok(result / FEE_DECIMAL_PRECISION)
} */

pub fn get_input_price(
    input_amount: Uint128,
    input_reserve: Uint128,
    output_reserve: Uint128,
    fee_percent_numerator: Uint128,
    fee_percent_denominator: Uint128,
    burn_fee_percent_numerator: Uint128,
    token_type: TokenSelect,
) -> StdResult<Uint128> {
    if input_reserve == Uint128::zero() || output_reserve == Uint128::zero() {
        return Err(StdError::generic_err("No liquidity"));
    };

    let input_amount_with_fee: Uint128;
    match token_type {
        TokenSelect::Token1 => {
            input_amount_with_fee = input_amount
                .checked_mul(
                    fee_percent_denominator - fee_percent_numerator - burn_fee_percent_numerator,
                )
                .map_err(StdError::overflow)?;
        }
        TokenSelect::Token2 => {
            input_amount_with_fee = input_amount
                .checked_mul(fee_percent_denominator - fee_percent_numerator)
                .map_err(StdError::overflow)?;
        }
    }

    let numerator = input_amount_with_fee
        .checked_mul(output_reserve)
        .map_err(StdError::overflow)?;
    let denominator = input_reserve
        .checked_mul(fee_percent_denominator)
        .map_err(StdError::overflow)?
        .checked_add(input_amount_with_fee)
        .map_err(StdError::overflow)?;

    Ok(numerator
        .checked_div(denominator)
        .map_err(StdError::divide_by_zero)?)
}

pub fn get_percent_amount(
    input_amount: Uint128,
    percent_numerator: Uint128,
    percent_denominator: Uint128,
) -> StdResult<Uint128> {
    if percent_numerator.is_zero() {
        return Ok(Uint128::zero());
    }
    Ok(input_amount
        .checked_mul(percent_numerator)
        .map_err(StdError::overflow)?
        .checked_div(percent_denominator)
        .map_err(StdError::divide_by_zero)?)
}

fn get_amount_for_denom(coins: &[Coin], denom: &str) -> Coin {
    let amount: Uint128 = coins
        .iter()
        .filter(|c| c.denom == denom)
        .map(|c| c.amount)
        .sum();
    Coin {
        amount,
        denom: denom.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn execute_swap(
    deps: DepsMut,
    info: &MessageInfo,
    input_amount: Uint128,
    _env: Env,
    input_token_enum: TokenSelect,
    recipient: String,
    min_token: Uint128,
    expiration: Option<Expiration>,
) -> Result<Response, ContractError> {
    let action = "swap".to_string();
    check_expiration(&expiration, &_env.block)?;

    let input_token_item = match input_token_enum {
        TokenSelect::Token1 => TOKEN1,
        TokenSelect::Token2 => TOKEN2,
    };
    let input_token = input_token_item.load(deps.storage)?;
    let output_token_item = match input_token_enum {
        TokenSelect::Token1 => TOKEN2,
        TokenSelect::Token2 => TOKEN1,
    };
    let output_token = output_token_item.load(deps.storage)?;

    let main_token_denom = match input_token_enum {
        TokenSelect::Token1 => input_token.denom.clone(),
        TokenSelect::Token2 => output_token.denom.clone(),
    };

    // validate input_amount if native input token
    validate_input_amount(&info.funds, input_amount, &input_token.denom)?;

    let fees = FEES.load(deps.storage)?;
    let burn_fee_percent_numerator = BURN_FEE_INFO.load(deps.storage)?;

    let mut token_bought: Uint128;
    let burn_fee_amount: Uint128;

    // Calculate fees
    let protocol_fee_amount = get_percent_amount(
        input_amount,
        fees.fee_percent_numerator,
        fees.fee_percent_denominator,
    )?;
    let input_amount_minus_protocol_burn_fee: Uint128;

    //Token1 is always hopers token so if the input is Token1, we should burn some percent of input token
    //else we should burn after swap
    match input_token_enum {
        TokenSelect::Token1 => {
            token_bought = get_input_price(
                input_amount,
                input_token.reserve,
                output_token.reserve,
                fees.fee_percent_numerator,
                fees.fee_percent_denominator,
                burn_fee_percent_numerator,
                TokenSelect::Token1,
            )?;
            // burn fee  = 2/1000 * input_amount(the input token in our main token hopers)
            // out_put token amount would not get affected
            burn_fee_amount = get_percent_amount(
                input_amount,
                burn_fee_percent_numerator,
                fees.fee_percent_denominator,
            )?;
            input_amount_minus_protocol_burn_fee =
                input_amount - protocol_fee_amount - burn_fee_amount;
        }
        TokenSelect::Token2 => {
            token_bought = get_input_price(
                input_amount,
                input_token.reserve,
                output_token.reserve,
                fees.fee_percent_numerator,
                fees.fee_percent_denominator,
                burn_fee_percent_numerator,
                TokenSelect::Token2,
            )?;
            // we get burn_fee_amount after swap the other token as hopers
            burn_fee_amount = get_percent_amount(
                token_bought,
                burn_fee_percent_numerator,
                fees.fee_percent_denominator,
            )?;
            // token_bought amount must be decreased by burn_fee_percent
            token_bought = get_percent_amount(
                token_bought,
                fees.fee_percent_denominator - burn_fee_percent_numerator,
                fees.fee_percent_denominator,
            )?;
            input_amount_minus_protocol_burn_fee = input_amount - protocol_fee_amount;
        }
    };

    if min_token > token_bought {
        return Err(ContractError::SwapMinError {
            min: min_token,
            available: token_bought,
        });
    }

    let mut msgs = match input_token.denom.clone() {
        Denom::Cw20(addr) => vec![get_cw20_transfer_from_msg(
            &info.sender,
            &_env.contract.address,
            &addr,
            input_amount_minus_protocol_burn_fee,
        )?],
        Denom::Native(_) => vec![],
    };

    match main_token_denom {
        Denom::Cw20(addr) => match input_token_enum {
            TokenSelect::Token1 => {
                if burn_fee_amount > Uint128::zero() {
                    msgs.push(get_cw20_burn_from_msg(
                        &info.sender,
                        &addr,
                        burn_fee_amount,
                    )?)
                }
            }
            TokenSelect::Token2 => {
                if burn_fee_amount > Uint128::zero() {
                    msgs.push(get_cw20_burn_msg(&addr, burn_fee_amount)?)
                }
            }
        },
        Denom::Native(_) => {}
    };

    // Send protocol fee to protocol fee recipient

    for dev_wallet in fees.dev_wallet_lists {
        let fee_amount = protocol_fee_amount * dev_wallet.ratio;
        if fee_amount > Uint128::zero() {
            msgs.push(get_fee_transfer_msg(
                &info.sender,
                &deps.api.addr_validate(&dev_wallet.address)?,
                &input_token.denom,
                fee_amount,
            )?)
        }
    }

    let recipient = deps.api.addr_validate(&recipient)?;
    // Create transfer to message
    msgs.push(match output_token.denom {
        Denom::Cw20(addr) => get_cw20_transfer_to_msg(&recipient, &addr, token_bought)?,
        Denom::Native(denom) => get_bank_transfer_to_msg(&recipient, &denom, token_bought),
    });

    input_token_item.update(
        deps.storage,
        |mut input_token| -> Result<_, ContractError> {
            input_token.reserve = input_token
                .reserve
                .checked_add(input_amount_minus_protocol_burn_fee)
                .map_err(StdError::overflow)?;
            Ok(input_token)
        },
    )?;

    output_token_item.update(
        deps.storage,
        |mut output_token| -> Result<_, ContractError> {
            output_token.reserve = output_token
                .reserve
                .checked_sub(token_bought)
                .map_err(StdError::overflow)?;
            Ok(output_token)
        },
    )?;

    let input_denom: String;
    match input_token_enum {
        TokenSelect::Token1 => input_denom = "Token1".to_string(),
        TokenSelect::Token2 => input_denom = "Token2".to_string(),
    }

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", action),
        attr("native_sold", input_amount),
        attr("token_bought", token_bought),
        attr("protocol_fee_amount", protocol_fee_amount),
        attr("input_token", input_denom),
    ]))
}

#[allow(clippy::too_many_arguments)]
pub fn execute_pass_through_swap(
    deps: DepsMut,
    info: MessageInfo,
    _env: Env,
    output_amm_address: String,
    input_token_enum: TokenSelect,
    input_token_amount: Uint128,
    output_min_token: Uint128,
    expiration: Option<Expiration>,
) -> Result<Response, ContractError> {
    let action = "pass_swap".to_string();
    check_expiration(&expiration, &_env.block)?;

    let input_token_state = match input_token_enum {
        TokenSelect::Token1 => TOKEN1,
        TokenSelect::Token2 => TOKEN2,
    };
    let input_token = input_token_state.load(deps.storage)?;
    let transfer_token_state = match input_token_enum {
        TokenSelect::Token1 => TOKEN2,
        TokenSelect::Token2 => TOKEN1,
    };
    let transfer_token = transfer_token_state.load(deps.storage)?;

    let main_token_denom = match input_token_enum {
        TokenSelect::Token1 => input_token.denom.clone(),
        TokenSelect::Token2 => transfer_token.denom.clone(),
    };

    validate_input_amount(&info.funds, input_token_amount, &input_token.denom)?;

    let fees = FEES.load(deps.storage)?;
    let burn_fee_percent_numerator = BURN_FEE_INFO.load(deps.storage)?;

    let mut amount_to_transfer: Uint128;
    let burn_fee_amount: Uint128;

    // Calculate fees
    let protocol_fee_amount = get_percent_amount(
        input_token_amount,
        fees.fee_percent_numerator,
        fees.fee_percent_denominator,
    )?;
    let input_amount_minus_protocol_burn_fee: Uint128;

    match input_token_enum {
        TokenSelect::Token1 => {
            amount_to_transfer = get_input_price(
                input_token_amount,
                input_token.reserve,
                transfer_token.reserve,
                fees.fee_percent_numerator,
                fees.fee_percent_denominator,
                burn_fee_percent_numerator,
                TokenSelect::Token1,
            )?;
            // burn fee  = 2/1000 * input_amount(the input token in our main token hopers)
            // out_put token amount would not get affected
            burn_fee_amount = get_percent_amount(
                input_token_amount,
                burn_fee_percent_numerator,
                fees.fee_percent_denominator,
            )?;
            input_amount_minus_protocol_burn_fee =
                input_token_amount - protocol_fee_amount - burn_fee_amount;
        }
        TokenSelect::Token2 => {
            amount_to_transfer = get_input_price(
                input_token_amount,
                input_token.reserve,
                transfer_token.reserve,
                fees.fee_percent_numerator,
                fees.fee_percent_denominator,
                burn_fee_percent_numerator,
                TokenSelect::Token2,
            )?;
            // we get burn_fee_amount after swap the other token as hopers
            burn_fee_amount = get_percent_amount(
                amount_to_transfer,
                burn_fee_percent_numerator,
                fees.fee_percent_denominator,
            )?;
            // amount_to_transfer amount must be decreased by burn_fee_percent
            amount_to_transfer = get_percent_amount(
                amount_to_transfer,
                fees.fee_percent_denominator - burn_fee_percent_numerator,
                fees.fee_percent_denominator,
            )?;
            input_amount_minus_protocol_burn_fee = input_token_amount - protocol_fee_amount;
        }
    };

    // Transfer input amount - protocol fee to contract
    let mut msgs: Vec<CosmosMsg> = vec![];
    if let Denom::Cw20(addr) = &input_token.denom {
        msgs.push(get_cw20_transfer_from_msg(
            &info.sender,
            &_env.contract.address,
            addr,
            input_amount_minus_protocol_burn_fee,
        )?)
    };

    match main_token_denom {
        Denom::Cw20(addr) => match input_token_enum {
            TokenSelect::Token1 => {
                if burn_fee_amount > Uint128::zero() {
                    msgs.push(get_cw20_burn_from_msg(
                        &info.sender,
                        &addr,
                        burn_fee_amount,
                    )?)
                }
            }
            TokenSelect::Token2 => {
                if burn_fee_amount > Uint128::zero() {
                    msgs.push(get_cw20_burn_msg(&addr, burn_fee_amount)?)
                }
            }
        },
        Denom::Native(_) => {}
    };

    // Send protocol fee to protocol fee recipient
    for dev_wallet in fees.dev_wallet_lists {
        let fee_amount = protocol_fee_amount * dev_wallet.ratio;
        if fee_amount > Uint128::zero() {
            msgs.push(get_fee_transfer_msg(
                &info.sender,
                &deps.api.addr_validate(&dev_wallet.address)?,
                &input_token.denom,
                fee_amount,
            )?)
        }
    }

    let output_amm_address = deps.api.addr_validate(&output_amm_address)?;

    // Increase allowance of output contract is transfer token is cw20
    if let Denom::Cw20(addr) = &transfer_token.denom {
        msgs.push(get_cw20_increase_allowance_msg(
            addr,
            &output_amm_address,
            amount_to_transfer,
            Some(Expiration::AtHeight(_env.block.height + 1)),
        )?)
    };

    let resp: InfoResponse = deps
        .querier
        .query_wasm_smart(&output_amm_address, &QueryMsg::Info {})?;

    let transfer_input_token_enum = if transfer_token.denom == resp.token1_denom {
        Ok(TokenSelect::Token1)
    } else if transfer_token.denom == resp.token2_denom {
        Ok(TokenSelect::Token2)
    } else {
        Err(ContractError::InvalidOutputPool {})
    }?;

    let swap_msg = ExecuteMsg::SwapAndSendTo {
        input_token: transfer_input_token_enum,
        input_amount: amount_to_transfer,
        recipient: info.sender.to_string(),
        min_token: output_min_token,
        expiration,
    };

    msgs.push(
        WasmMsg::Execute {
            contract_addr: output_amm_address.into(),
            msg: to_binary(&swap_msg)?,
            funds: match transfer_token.denom {
                Denom::Cw20(_) => vec![],
                Denom::Native(denom) => vec![Coin {
                    denom,
                    amount: amount_to_transfer,
                }],
            },
        }
        .into(),
    );

    input_token_state.update(deps.storage, |mut token| -> Result<_, ContractError> {
        // Add input amount - protocol fee to input token reserve
        token.reserve = token
            .reserve
            .checked_add(input_amount_minus_protocol_burn_fee)
            .map_err(StdError::overflow)?;
        Ok(token)
    })?;

    transfer_token_state.update(deps.storage, |mut token| -> Result<_, ContractError> {
        token.reserve = token
            .reserve
            .checked_sub(amount_to_transfer)
            .map_err(StdError::overflow)?;
        Ok(token)
    })?;

    Ok(Response::new().add_messages(msgs).add_attributes(vec![
        attr("action", action),
        attr("input_token_amount", input_token_amount),
        attr("native_transferred", amount_to_transfer),
    ]))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Balance { address } => to_binary(&query_balance(deps, address)?),
        QueryMsg::Info {} => to_binary(&query_info(deps)?),
        QueryMsg::Token1ForToken2Price { token1_amount } => {
            to_binary(&query_token1_for_token2_price(deps, token1_amount)?)
        }
        QueryMsg::Token2ForToken1Price { token2_amount } => {
            to_binary(&query_token2_for_token1_price(deps, token2_amount)?)
        }
        QueryMsg::Fee {} => to_binary(&query_fee(deps)?),
    }
}

pub fn query_info(deps: Deps) -> StdResult<InfoResponse> {
    let token1 = TOKEN1.load(deps.storage)?;
    let token2 = TOKEN2.load(deps.storage)?;
    let lp_token_address = LP_TOKEN.load(deps.storage)?;

    // TODO get total supply
    Ok(InfoResponse {
        token1_reserve: token1.reserve,
        token1_denom: token1.denom,
        token2_reserve: token2.reserve,
        token2_denom: token2.denom,
        lp_token_supply: get_lp_token_supply(deps, &lp_token_address)?,
        lp_token_address: lp_token_address.into_string(),
    })
}

pub fn query_token1_for_token2_price(
    deps: Deps,
    token1_amount: Uint128,
) -> StdResult<Token1ForToken2PriceResponse> {
    let token1 = TOKEN1.load(deps.storage)?;
    let token2 = TOKEN2.load(deps.storage)?;

    let fees = FEES.load(deps.storage)?;
    let burn_fee_percent_numerator = BURN_FEE_INFO.load(deps.storage)?;

    let token2_amount = get_input_price(
        token1_amount,
        token1.reserve,
        token2.reserve,
        fees.fee_percent_numerator,
        fees.fee_percent_denominator,
        burn_fee_percent_numerator,
        TokenSelect::Token1,
    )?;
    Ok(Token1ForToken2PriceResponse { token2_amount })
}

pub fn query_token2_for_token1_price(
    deps: Deps,
    token2_amount: Uint128,
) -> StdResult<Token2ForToken1PriceResponse> {
    let token1 = TOKEN1.load(deps.storage)?;
    let token2 = TOKEN2.load(deps.storage)?;

    let fees = FEES.load(deps.storage)?;
    let burn_fee_percent_numerator = BURN_FEE_INFO.load(deps.storage)?;

    let mut token1_amount = get_input_price(
        token2_amount,
        token2.reserve,
        token1.reserve,
        fees.fee_percent_numerator,
        fees.fee_percent_denominator,
        burn_fee_percent_numerator,
        TokenSelect::Token2,
    )?;

    token1_amount = get_percent_amount(
        token1_amount,
        fees.fee_percent_denominator - burn_fee_percent_numerator,
        fees.fee_percent_denominator,
    )?;

    Ok(Token2ForToken1PriceResponse { token1_amount })
}

pub fn query_fee(deps: Deps) -> StdResult<FeeResponse> {
    let fees = FEES.load(deps.storage)?;
    let owner = OWNER.load(deps.storage)?.map(|o| o.into_string());

    let num = fees.fee_percent_numerator.u128();
    let den = fees.fee_percent_denominator.u128();
    let total_fee_percent = Decimal::from_ratio(num, den);

    Ok(FeeResponse {
        owner,
        total_fee_percent,
        dev_wallet_lists: fees.dev_wallet_lists,
    })
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    if msg.id != INSTANTIATE_LP_TOKEN_REPLY_ID {
        return Err(ContractError::UnknownReplyId { id: msg.id });
    };
    let res = parse_reply_instantiate_data(msg);
    match res {
        Ok(res) => {
            // Validate contract address
            let cw20_addr = deps.api.addr_validate(&res.contract_address)?;

            // Save gov token
            LP_TOKEN.save(deps.storage, &cw20_addr)?;

            Ok(Response::new())
        }
        Err(_) => Err(ContractError::InstantiateLpTokenError {}),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, msg: MigrateMsg) -> Result<Response, ContractError> {
    let version = get_contract_version(deps.storage)?;
    if version.contract != CONTRACT_NAME {
        return Err(ContractError::CannotMigrate {
            previous_contract: version.contract,
        });
    }

    if version.version != CONTRACT_VERSION {
        return Err(ContractError::CannotMigrate {
            previous_contract: version.version,
        });
    }

    BURN_FEE_INFO.save(deps.storage, &msg.burn_fee_percent_numerator)?;

    Ok(Response::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_liquidity_amount() {
        let liquidity =
            get_lp_token_amount_to_mint(Uint128::new(100), Uint128::zero(), Uint128::zero())
                .unwrap();
        assert_eq!(liquidity, Uint128::new(100));

        let liquidity =
            get_lp_token_amount_to_mint(Uint128::new(100), Uint128::new(50), Uint128::new(25))
                .unwrap();
        assert_eq!(liquidity, Uint128::new(200));
    }

    #[test]
    fn test_get_token_amount() {
        let liquidity = get_token2_amount_required(
            Uint128::new(100),
            Uint128::new(50),
            Uint128::zero(),
            Uint128::zero(),
            Uint128::zero(),
        )
        .unwrap();
        assert_eq!(liquidity, Uint128::new(100));

        let liquidity = get_token2_amount_required(
            Uint128::new(200),
            Uint128::new(50),
            Uint128::new(50),
            Uint128::new(100),
            Uint128::new(25),
        )
        .unwrap();
        assert_eq!(liquidity, Uint128::new(201));
    }
}
