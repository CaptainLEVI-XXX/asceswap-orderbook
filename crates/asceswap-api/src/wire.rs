use serde::{Deserialize, Serialize};

use asceswap_state::{OrderState, ReservationLegRole, ReservationStatus};
use asceswap_types::{Address, ClaimSide, MatchKind, Side, B256, U256};
use asceswap_validation::SignatureCheck;

use crate::ApiError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiClaimSide {
    Residual,
    Payoff,
}

impl From<ApiClaimSide> for ClaimSide {
    fn from(value: ApiClaimSide) -> Self {
        match value {
            ApiClaimSide::Residual => Self::Residual,
            ApiClaimSide::Payoff => Self::Payoff,
        }
    }
}

impl From<ClaimSide> for ApiClaimSide {
    fn from(value: ClaimSide) -> Self {
        match value {
            ClaimSide::Residual => Self::Residual,
            ClaimSide::Payoff => Self::Payoff,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiSide {
    Buy,
    Sell,
}

impl From<ApiSide> for Side {
    fn from(value: ApiSide) -> Self {
        match value {
            ApiSide::Buy => Self::Buy,
            ApiSide::Sell => Self::Sell,
        }
    }
}

impl From<Side> for ApiSide {
    fn from(value: Side) -> Self {
        match value {
            Side::Buy => Self::Buy,
            Side::Sell => Self::Sell,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiSignatureCheck {
    Unchecked,
    Valid,
    Invalid,
}

impl From<ApiSignatureCheck> for SignatureCheck {
    fn from(value: ApiSignatureCheck) -> Self {
        match value {
            ApiSignatureCheck::Unchecked => Self::Unchecked,
            ApiSignatureCheck::Valid => Self::Valid,
            ApiSignatureCheck::Invalid => Self::Invalid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiOrderState {
    Received,
    Validating,
    Rejected,
    Open,
    PartiallyFilled,
    Reserved,
    Submitted,
    Filled,
    Expired,
    SoftCancelled,
    CancelPending,
    Cancelled,
    EpochInvalidated,
    Inactive,
}

impl From<OrderState> for ApiOrderState {
    fn from(value: OrderState) -> Self {
        match value {
            OrderState::Received => Self::Received,
            OrderState::Validating => Self::Validating,
            OrderState::Rejected => Self::Rejected,
            OrderState::Open => Self::Open,
            OrderState::PartiallyFilled => Self::PartiallyFilled,
            OrderState::Reserved => Self::Reserved,
            OrderState::Submitted => Self::Submitted,
            OrderState::Filled => Self::Filled,
            OrderState::Expired => Self::Expired,
            OrderState::SoftCancelled => Self::SoftCancelled,
            OrderState::CancelPending => Self::CancelPending,
            OrderState::Cancelled => Self::Cancelled,
            OrderState::EpochInvalidated => Self::EpochInvalidated,
            OrderState::Inactive => Self::Inactive,
        }
    }
}

impl From<ApiOrderState> for OrderState {
    fn from(value: ApiOrderState) -> Self {
        match value {
            ApiOrderState::Received => Self::Received,
            ApiOrderState::Validating => Self::Validating,
            ApiOrderState::Rejected => Self::Rejected,
            ApiOrderState::Open => Self::Open,
            ApiOrderState::PartiallyFilled => Self::PartiallyFilled,
            ApiOrderState::Reserved => Self::Reserved,
            ApiOrderState::Submitted => Self::Submitted,
            ApiOrderState::Filled => Self::Filled,
            ApiOrderState::Expired => Self::Expired,
            ApiOrderState::SoftCancelled => Self::SoftCancelled,
            ApiOrderState::CancelPending => Self::CancelPending,
            ApiOrderState::Cancelled => Self::Cancelled,
            ApiOrderState::EpochInvalidated => Self::EpochInvalidated,
            ApiOrderState::Inactive => Self::Inactive,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiReservationStatus {
    Reserved,
    Submitted,
    Released,
    Expired,
    Committed,
}

impl From<ReservationStatus> for ApiReservationStatus {
    fn from(value: ReservationStatus) -> Self {
        match value {
            ReservationStatus::Reserved => Self::Reserved,
            ReservationStatus::Submitted => Self::Submitted,
            ReservationStatus::Released => Self::Released,
            ReservationStatus::Expired => Self::Expired,
            ReservationStatus::Committed => Self::Committed,
        }
    }
}

impl From<ApiReservationStatus> for ReservationStatus {
    fn from(value: ApiReservationStatus) -> Self {
        match value {
            ApiReservationStatus::Reserved => Self::Reserved,
            ApiReservationStatus::Submitted => Self::Submitted,
            ApiReservationStatus::Released => Self::Released,
            ApiReservationStatus::Expired => Self::Expired,
            ApiReservationStatus::Committed => Self::Committed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiReservationLegRole {
    Taker,
    Maker,
}

impl From<ReservationLegRole> for ApiReservationLegRole {
    fn from(value: ReservationLegRole) -> Self {
        match value {
            ReservationLegRole::Taker => Self::Taker,
            ReservationLegRole::Maker => Self::Maker,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiMatchKind {
    Direct,
    MintAssisted,
    MergeAssisted,
}

impl From<MatchKind> for ApiMatchKind {
    fn from(value: MatchKind) -> Self {
        match value {
            MatchKind::Direct => Self::Direct,
            MatchKind::MintAssisted => Self::MintAssisted,
            MatchKind::MergeAssisted => Self::MergeAssisted,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiOrder {
    pub salt: String,
    pub maker: String,
    pub market_id: String,
    pub claim: ApiClaimSide,
    pub maker_amount: String,
    pub taker_amount: String,
    pub side: ApiSide,
    pub expiration: String,
    pub epoch: String,
    pub max_fee_rate_bps: u16,
}

impl From<&asceswap_types::Order> for ApiOrder {
    fn from(order: &asceswap_types::Order) -> Self {
        Self {
            salt: encode_u256(order.salt),
            maker: encode_address(order.maker),
            market_id: encode_b256(order.market_id),
            claim: order.claim.into(),
            maker_amount: encode_u256(order.maker_amount),
            taker_amount: encode_u256(order.taker_amount),
            side: order.side.into(),
            expiration: encode_u256(order.expiration),
            epoch: encode_u256(order.epoch),
            max_fee_rate_bps: order.max_fee_rate_bps,
        }
    }
}

impl ApiOrder {
    pub fn to_order(&self) -> Result<asceswap_types::Order, ApiError> {
        Ok(asceswap_types::Order {
            salt: parse_u256("salt", &self.salt)?,
            maker: parse_address("maker", &self.maker)?,
            market_id: parse_b256("market_id", &self.market_id)?,
            claim: self.claim.into(),
            maker_amount: parse_u256("maker_amount", &self.maker_amount)?,
            taker_amount: parse_u256("taker_amount", &self.taker_amount)?,
            side: self.side.into(),
            expiration: parse_u256("expiration", &self.expiration)?,
            epoch: parse_u256("epoch", &self.epoch)?,
            max_fee_rate_bps: self.max_fee_rate_bps,
        })
    }
}

pub fn encode_b256(value: B256) -> String {
    encode_hex(value.as_slice())
}

pub fn encode_address(value: Address) -> String {
    encode_hex(value.as_slice())
}

pub fn encode_bytes(value: &[u8]) -> String {
    encode_hex(value)
}

pub fn encode_u256(value: U256) -> String {
    value.to_string()
}

pub fn parse_b256(field: &'static str, value: &str) -> Result<B256, ApiError> {
    let bytes = parse_hex_fixed(field, value, 32)?;
    Ok(B256::from_slice(&bytes))
}

pub fn parse_address(field: &'static str, value: &str) -> Result<Address, ApiError> {
    let bytes = parse_hex_fixed(field, value, 20)?;
    Ok(Address::from_slice(&bytes))
}

pub fn parse_hex_bytes(field: &'static str, value: &str) -> Result<Vec<u8>, ApiError> {
    let raw = value.strip_prefix("0x").ok_or(ApiError::InvalidField {
        field,
        reason: "missing 0x prefix",
    })?;
    if raw.len() % 2 != 0 {
        return Err(ApiError::InvalidField {
            field,
            reason: "odd hex length",
        });
    }

    parse_hex_raw(field, raw)
}

pub fn parse_u256(field: &'static str, value: &str) -> Result<U256, ApiError> {
    if value.is_empty() {
        return Err(ApiError::InvalidField {
            field,
            reason: "empty decimal string",
        });
    }

    U256::from_str_radix(value, 10).map_err(|_| ApiError::InvalidField {
        field,
        reason: "invalid uint256 decimal string",
    })
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn parse_hex_fixed(field: &'static str, value: &str, byte_len: usize) -> Result<Vec<u8>, ApiError> {
    let raw = value.strip_prefix("0x").ok_or(ApiError::InvalidField {
        field,
        reason: "missing 0x prefix",
    })?;
    if raw.len() != byte_len * 2 {
        return Err(ApiError::InvalidField {
            field,
            reason: "incorrect hex length",
        });
    }

    parse_hex_raw(field, raw)
}

fn parse_hex_raw(field: &'static str, raw: &str) -> Result<Vec<u8>, ApiError> {
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    let raw = raw.as_bytes();
    for index in (0..raw.len()).step_by(2) {
        let high = hex_nibble(raw[index]).ok_or(ApiError::InvalidField {
            field,
            reason: "invalid hex character",
        })?;
        let low = hex_nibble(raw[index + 1]).ok_or(ApiError::InvalidField {
            field,
            reason: "invalid hex character",
        })?;
        bytes.push((high << 4) | low);
    }

    Ok(bytes)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
