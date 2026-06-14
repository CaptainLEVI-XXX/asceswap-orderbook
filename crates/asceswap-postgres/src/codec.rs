use asceswap_engine::EngineEvent;
use asceswap_math::MathError;
use asceswap_state::{OrderState, ReservationLegRole, ReservationStatus};
use asceswap_storage::StorageError;
use asceswap_types::{Address, ClaimSide, MatchKind, OrderError, B256, U256};
use asceswap_validation::ValidationError;
use serde_json::{json, Value};

pub(crate) struct EncodedEvent {
    pub kind: &'static str,
    pub payload: Value,
}

pub(crate) fn encode_event(event: &EngineEvent) -> EncodedEvent {
    let (kind, payload) = match event {
        EngineEvent::OrderReceived {
            order_hash,
            market_id,
        } => (
            "order_received",
            json!({
                "order_hash": encode_b256(*order_hash),
                "market_id": encode_b256(*market_id),
            }),
        ),
        EngineEvent::OrderValidated {
            order_hash,
            remaining_claim_amount,
        } => (
            "order_validated",
            json!({
                "order_hash": encode_b256(*order_hash),
                "remaining_claim_amount": u256_to_string(*remaining_claim_amount),
            }),
        ),
        EngineEvent::OrderRejected { order_hash, reason } => (
            "order_rejected",
            json!({
                "order_hash": encode_b256(*order_hash),
                "reason": validation_error_to_value(reason),
            }),
        ),
        EngineEvent::OrderOpened { order_hash } => (
            "order_opened",
            json!({ "order_hash": encode_b256(*order_hash) }),
        ),
        EngineEvent::OrderInactive { order_hash } => (
            "order_inactive",
            json!({ "order_hash": encode_b256(*order_hash) }),
        ),
        EngineEvent::OrderReserved {
            order_hash,
            reservation_id,
        } => (
            "order_reserved",
            json!({
                "order_hash": encode_b256(*order_hash),
                "reservation_id": encode_b256(*reservation_id),
            }),
        ),
        EngineEvent::OrderSubmitted {
            order_hash,
            reservation_id,
        } => (
            "order_submitted",
            json!({
                "order_hash": encode_b256(*order_hash),
                "reservation_id": encode_b256(*reservation_id),
            }),
        ),
        EngineEvent::OrderStateChanged { order_hash, state } => (
            "order_state_changed",
            json!({
                "order_hash": encode_b256(*order_hash),
                "state": order_state_to_str(*state),
            }),
        ),
        EngineEvent::OrderPartiallyFilled {
            order_hash,
            filled_claim_amount,
            remaining_claim_amount,
        } => (
            "order_partially_filled",
            json!({
                "order_hash": encode_b256(*order_hash),
                "filled_claim_amount": u256_to_string(*filled_claim_amount),
                "remaining_claim_amount": u256_to_string(*remaining_claim_amount),
            }),
        ),
        EngineEvent::OrderFilled { order_hash } => (
            "order_filled",
            json!({ "order_hash": encode_b256(*order_hash) }),
        ),
        EngineEvent::OrderCancelled { order_hash } => (
            "order_cancelled",
            json!({ "order_hash": encode_b256(*order_hash) }),
        ),
        EngineEvent::ReservationCreated {
            reservation_id,
            match_kind,
            maker_count,
        } => (
            "reservation_created",
            json!({
                "reservation_id": encode_b256(*reservation_id),
                "match_kind": match_kind_to_str(*match_kind),
                "maker_count": *maker_count,
            }),
        ),
        EngineEvent::ReservationSubmitted { reservation_id } => (
            "reservation_submitted",
            json!({ "reservation_id": encode_b256(*reservation_id) }),
        ),
        EngineEvent::ReservationReleased { reservation_id } => (
            "reservation_released",
            json!({ "reservation_id": encode_b256(*reservation_id) }),
        ),
        EngineEvent::ReservationExpired { reservation_id } => (
            "reservation_expired",
            json!({ "reservation_id": encode_b256(*reservation_id) }),
        ),
        EngineEvent::ReservationCommitted { reservation_id } => (
            "reservation_committed",
            json!({ "reservation_id": encode_b256(*reservation_id) }),
        ),
    };

    EncodedEvent { kind, payload }
}

pub(crate) fn decode_event(kind: &str, payload: &str) -> Result<EngineEvent, StorageError> {
    let payload = json_payload(payload)?;
    match kind {
        "order_received" => Ok(EngineEvent::OrderReceived {
            order_hash: b256_field(&payload, "order_hash")?,
            market_id: b256_field(&payload, "market_id")?,
        }),
        "order_validated" => Ok(EngineEvent::OrderValidated {
            order_hash: b256_field(&payload, "order_hash")?,
            remaining_claim_amount: u256_field(&payload, "remaining_claim_amount")?,
        }),
        "order_rejected" => Ok(EngineEvent::OrderRejected {
            order_hash: b256_field(&payload, "order_hash")?,
            reason: validation_error_from_value(value_field(&payload, "reason")?)?,
        }),
        "order_opened" => Ok(EngineEvent::OrderOpened {
            order_hash: b256_field(&payload, "order_hash")?,
        }),
        "order_inactive" => Ok(EngineEvent::OrderInactive {
            order_hash: b256_field(&payload, "order_hash")?,
        }),
        "order_reserved" => Ok(EngineEvent::OrderReserved {
            order_hash: b256_field(&payload, "order_hash")?,
            reservation_id: b256_field(&payload, "reservation_id")?,
        }),
        "order_submitted" => Ok(EngineEvent::OrderSubmitted {
            order_hash: b256_field(&payload, "order_hash")?,
            reservation_id: b256_field(&payload, "reservation_id")?,
        }),
        "order_state_changed" => Ok(EngineEvent::OrderStateChanged {
            order_hash: b256_field(&payload, "order_hash")?,
            state: order_state_from_str(string_field(&payload, "state")?)?,
        }),
        "order_partially_filled" => Ok(EngineEvent::OrderPartiallyFilled {
            order_hash: b256_field(&payload, "order_hash")?,
            filled_claim_amount: u256_field(&payload, "filled_claim_amount")?,
            remaining_claim_amount: u256_field(&payload, "remaining_claim_amount")?,
        }),
        "order_filled" => Ok(EngineEvent::OrderFilled {
            order_hash: b256_field(&payload, "order_hash")?,
        }),
        "order_cancelled" => Ok(EngineEvent::OrderCancelled {
            order_hash: b256_field(&payload, "order_hash")?,
        }),
        "reservation_created" => Ok(EngineEvent::ReservationCreated {
            reservation_id: b256_field(&payload, "reservation_id")?,
            match_kind: match_kind_from_str(string_field(&payload, "match_kind")?)?,
            maker_count: usize_field(&payload, "maker_count")?,
        }),
        "reservation_submitted" => Ok(EngineEvent::ReservationSubmitted {
            reservation_id: b256_field(&payload, "reservation_id")?,
        }),
        "reservation_released" => Ok(EngineEvent::ReservationReleased {
            reservation_id: b256_field(&payload, "reservation_id")?,
        }),
        "reservation_expired" => Ok(EngineEvent::ReservationExpired {
            reservation_id: b256_field(&payload, "reservation_id")?,
        }),
        "reservation_committed" => Ok(EngineEvent::ReservationCommitted {
            reservation_id: b256_field(&payload, "reservation_id")?,
        }),
        other => Err(invalid(format!("unknown engine event type {other}"))),
    }
}

pub(crate) fn b256_to_bytes(value: B256) -> Vec<u8> {
    value.as_slice().to_vec()
}

pub(crate) fn b256_from_bytes(field: &str, bytes: Vec<u8>) -> Result<B256, StorageError> {
    if bytes.len() != 32 {
        return Err(invalid(format!(
            "{field} must be 32 bytes, got {} bytes",
            bytes.len()
        )));
    }

    Ok(B256::from_slice(&bytes))
}

pub(crate) fn address_to_bytes(value: Address) -> Vec<u8> {
    value.as_slice().to_vec()
}

pub(crate) fn address_from_bytes(field: &str, bytes: Vec<u8>) -> Result<Address, StorageError> {
    if bytes.len() != 20 {
        return Err(invalid(format!(
            "{field} must be 20 bytes, got {} bytes",
            bytes.len()
        )));
    }

    Ok(Address::from_slice(&bytes))
}

pub(crate) fn u256_to_string(value: U256) -> String {
    value.to_string()
}

pub(crate) fn u256_from_string(field: &str, value: &str) -> Result<U256, StorageError> {
    if value.is_empty() {
        return Err(invalid(format!(
            "{field} is an empty uint256 decimal string"
        )));
    }

    U256::from_str_radix(value, 10)
        .map_err(|_| invalid(format!("{field} is an invalid uint256 decimal string")))
}

pub(crate) fn u64_to_i64(field: &str, value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| invalid(format!("{field} exceeds PostgreSQL BIGINT")))
}

pub(crate) fn i64_to_u64(field: &str, value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| invalid(format!("{field} is negative")))
}

pub(crate) fn usize_to_i32(field: &str, value: usize) -> Result<i32, StorageError> {
    i32::try_from(value).map_err(|_| invalid(format!("{field} exceeds PostgreSQL INTEGER")))
}

pub(crate) fn i32_to_u16(field: &str, value: i32) -> Result<u16, StorageError> {
    u16::try_from(value).map_err(|_| invalid(format!("{field} is outside u16 range")))
}

pub(crate) fn claim_side_to_i16(value: ClaimSide) -> i16 {
    match value {
        ClaimSide::Residual => 0,
        ClaimSide::Payoff => 1,
    }
}

pub(crate) fn claim_side_from_i16(field: &str, value: i16) -> Result<ClaimSide, StorageError> {
    match value {
        0 => Ok(ClaimSide::Residual),
        1 => Ok(ClaimSide::Payoff),
        _ => Err(invalid(format!("{field} has invalid claim side {value}"))),
    }
}

pub(crate) fn side_to_i16(value: asceswap_types::Side) -> i16 {
    match value {
        asceswap_types::Side::Buy => 0,
        asceswap_types::Side::Sell => 1,
    }
}

pub(crate) fn side_from_i16(field: &str, value: i16) -> Result<asceswap_types::Side, StorageError> {
    match value {
        0 => Ok(asceswap_types::Side::Buy),
        1 => Ok(asceswap_types::Side::Sell),
        _ => Err(invalid(format!("{field} has invalid side {value}"))),
    }
}

pub(crate) fn reservation_leg_role_to_i16(value: ReservationLegRole) -> i16 {
    match value {
        ReservationLegRole::Taker => 0,
        ReservationLegRole::Maker => 1,
    }
}

pub(crate) fn reservation_leg_role_from_i16(
    field: &str,
    value: i16,
) -> Result<ReservationLegRole, StorageError> {
    match value {
        0 => Ok(ReservationLegRole::Taker),
        1 => Ok(ReservationLegRole::Maker),
        _ => Err(invalid(format!(
            "{field} has invalid reservation leg role {value}"
        ))),
    }
}

pub(crate) fn order_state_to_str(value: OrderState) -> &'static str {
    match value {
        OrderState::Received => "received",
        OrderState::Validating => "validating",
        OrderState::Rejected => "rejected",
        OrderState::Open => "open",
        OrderState::PartiallyFilled => "partially_filled",
        OrderState::Reserved => "reserved",
        OrderState::Submitted => "submitted",
        OrderState::Filled => "filled",
        OrderState::Expired => "expired",
        OrderState::SoftCancelled => "soft_cancelled",
        OrderState::CancelPending => "cancel_pending",
        OrderState::Cancelled => "cancelled",
        OrderState::EpochInvalidated => "epoch_invalidated",
        OrderState::Inactive => "inactive",
    }
}

pub(crate) fn order_state_from_str(value: &str) -> Result<OrderState, StorageError> {
    match value {
        "received" => Ok(OrderState::Received),
        "validating" => Ok(OrderState::Validating),
        "rejected" => Ok(OrderState::Rejected),
        "open" => Ok(OrderState::Open),
        "partially_filled" => Ok(OrderState::PartiallyFilled),
        "reserved" => Ok(OrderState::Reserved),
        "submitted" => Ok(OrderState::Submitted),
        "filled" => Ok(OrderState::Filled),
        "expired" => Ok(OrderState::Expired),
        "soft_cancelled" => Ok(OrderState::SoftCancelled),
        "cancel_pending" => Ok(OrderState::CancelPending),
        "cancelled" => Ok(OrderState::Cancelled),
        "epoch_invalidated" => Ok(OrderState::EpochInvalidated),
        "inactive" => Ok(OrderState::Inactive),
        _ => Err(invalid(format!("unknown order state {value}"))),
    }
}

pub(crate) fn reservation_status_to_str(value: ReservationStatus) -> &'static str {
    match value {
        ReservationStatus::Reserved => "reserved",
        ReservationStatus::Submitted => "submitted",
        ReservationStatus::Released => "released",
        ReservationStatus::Expired => "expired",
        ReservationStatus::Committed => "committed",
    }
}

pub(crate) fn reservation_status_from_str(value: &str) -> Result<ReservationStatus, StorageError> {
    match value {
        "reserved" => Ok(ReservationStatus::Reserved),
        "submitted" => Ok(ReservationStatus::Submitted),
        "released" => Ok(ReservationStatus::Released),
        "expired" => Ok(ReservationStatus::Expired),
        "committed" => Ok(ReservationStatus::Committed),
        _ => Err(invalid(format!("unknown reservation status {value}"))),
    }
}

pub(crate) fn match_kind_to_str(value: MatchKind) -> &'static str {
    match value {
        MatchKind::Direct => "direct",
        MatchKind::MintAssisted => "mint_assisted",
        MatchKind::MergeAssisted => "merge_assisted",
    }
}

pub(crate) fn match_kind_from_str(value: &str) -> Result<MatchKind, StorageError> {
    match value {
        "direct" => Ok(MatchKind::Direct),
        "mint_assisted" => Ok(MatchKind::MintAssisted),
        "merge_assisted" => Ok(MatchKind::MergeAssisted),
        _ => Err(invalid(format!("unknown match kind {value}"))),
    }
}

fn validation_error_to_value(error: &ValidationError) -> Value {
    match *error {
        ValidationError::BasicOrder(error) => json!({
            "kind": "basic_order",
            "error": order_error_to_str(error),
        }),
        ValidationError::OrderHashMismatch { expected, actual } => json!({
            "kind": "order_hash_mismatch",
            "expected": encode_b256(expected),
            "actual": encode_b256(actual),
        }),
        ValidationError::InvalidSignature => json!({ "kind": "invalid_signature" }),
        ValidationError::Expired { expiration, now } => json!({
            "kind": "expired",
            "expiration": u256_to_string(expiration),
            "now": now,
        }),
        ValidationError::Cancelled => json!({ "kind": "cancelled" }),
        ValidationError::EpochMismatch {
            order_epoch,
            maker_epoch,
        } => json!({
            "kind": "epoch_mismatch",
            "order_epoch": u256_to_string(order_epoch),
            "maker_epoch": u256_to_string(maker_epoch),
        }),
        ValidationError::FeeRateTooHigh {
            fee_rate_bps,
            max_fee_rate_bps,
        } => json!({
            "kind": "fee_rate_too_high",
            "fee_rate_bps": fee_rate_bps,
            "max_fee_rate_bps": max_fee_rate_bps,
        }),
        ValidationError::InvalidExchangeFeeRate {
            fee_rate_bps,
            max_fee_rate_bps,
        } => json!({
            "kind": "invalid_exchange_fee_rate",
            "fee_rate_bps": fee_rate_bps,
            "max_fee_rate_bps": max_fee_rate_bps,
        }),
        ValidationError::MissingSignatureVerification => {
            json!({ "kind": "missing_signature_verification" })
        }
        ValidationError::Fill(error) => json!({
            "kind": "fill",
            "error": math_error_to_value(&error),
        }),
        ValidationError::NoRemainingClaim => json!({ "kind": "no_remaining_claim" }),
    }
}

fn validation_error_from_value(value: &Value) -> Result<ValidationError, StorageError> {
    match string_field(value, "kind")? {
        "basic_order" => Ok(ValidationError::BasicOrder(order_error_from_str(
            string_field(value, "error")?,
        )?)),
        "order_hash_mismatch" => Ok(ValidationError::OrderHashMismatch {
            expected: b256_field(value, "expected")?,
            actual: b256_field(value, "actual")?,
        }),
        "invalid_signature" => Ok(ValidationError::InvalidSignature),
        "expired" => Ok(ValidationError::Expired {
            expiration: u256_field(value, "expiration")?,
            now: u64_field(value, "now")?,
        }),
        "cancelled" => Ok(ValidationError::Cancelled),
        "epoch_mismatch" => Ok(ValidationError::EpochMismatch {
            order_epoch: u256_field(value, "order_epoch")?,
            maker_epoch: u256_field(value, "maker_epoch")?,
        }),
        "fee_rate_too_high" => Ok(ValidationError::FeeRateTooHigh {
            fee_rate_bps: u16_field(value, "fee_rate_bps")?,
            max_fee_rate_bps: u16_field(value, "max_fee_rate_bps")?,
        }),
        "invalid_exchange_fee_rate" => Ok(ValidationError::InvalidExchangeFeeRate {
            fee_rate_bps: u16_field(value, "fee_rate_bps")?,
            max_fee_rate_bps: u16_field(value, "max_fee_rate_bps")?,
        }),
        "missing_signature_verification" => Ok(ValidationError::MissingSignatureVerification),
        "fill" => Ok(ValidationError::Fill(math_error_from_value(value_field(
            value, "error",
        )?)?)),
        "no_remaining_claim" => Ok(ValidationError::NoRemainingClaim),
        other => Err(invalid(format!("unknown validation error kind {other}"))),
    }
}

fn math_error_to_value(error: &MathError) -> Value {
    match *error {
        MathError::Order(error) => json!({
            "kind": "order",
            "error": order_error_to_str(error),
        }),
        MathError::DivisionByZero => json!({ "kind": "division_by_zero" }),
        MathError::ZeroFill => json!({ "kind": "zero_fill" }),
        MathError::Overfill => json!({ "kind": "overfill" }),
        MathError::ArithmeticOverflow => json!({ "kind": "arithmetic_overflow" }),
        MathError::InvalidFeeConfig => json!({ "kind": "invalid_fee_config" }),
    }
}

fn math_error_from_value(value: &Value) -> Result<MathError, StorageError> {
    match string_field(value, "kind")? {
        "order" => Ok(MathError::Order(order_error_from_str(string_field(
            value, "error",
        )?)?)),
        "division_by_zero" => Ok(MathError::DivisionByZero),
        "zero_fill" => Ok(MathError::ZeroFill),
        "overfill" => Ok(MathError::Overfill),
        "arithmetic_overflow" => Ok(MathError::ArithmeticOverflow),
        "invalid_fee_config" => Ok(MathError::InvalidFeeConfig),
        other => Err(invalid(format!("unknown math error kind {other}"))),
    }
}

fn order_error_to_str(error: OrderError) -> &'static str {
    match error {
        OrderError::ZeroMaker => "zero_maker",
        OrderError::ZeroMarket => "zero_market",
        OrderError::ZeroAmount => "zero_amount",
        OrderError::ImpossiblePrice => "impossible_price",
    }
}

fn order_error_from_str(value: &str) -> Result<OrderError, StorageError> {
    match value {
        "zero_maker" => Ok(OrderError::ZeroMaker),
        "zero_market" => Ok(OrderError::ZeroMarket),
        "zero_amount" => Ok(OrderError::ZeroAmount),
        "impossible_price" => Ok(OrderError::ImpossiblePrice),
        _ => Err(invalid(format!("unknown order error {value}"))),
    }
}

fn json_payload(payload: &str) -> Result<Value, StorageError> {
    serde_json::from_str(payload)
        .map_err(|error| invalid(format!("invalid event payload: {error}")))
}

fn value_field<'a>(value: &'a Value, field: &str) -> Result<&'a Value, StorageError> {
    value
        .get(field)
        .ok_or_else(|| invalid(format!("missing JSON field {field}")))
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, StorageError> {
    value_field(value, field)?
        .as_str()
        .ok_or_else(|| invalid(format!("JSON field {field} must be a string")))
}

fn u64_field(value: &Value, field: &str) -> Result<u64, StorageError> {
    value_field(value, field)?
        .as_u64()
        .ok_or_else(|| invalid(format!("JSON field {field} must be a u64")))
}

fn usize_field(value: &Value, field: &str) -> Result<usize, StorageError> {
    usize::try_from(u64_field(value, field)?)
        .map_err(|_| invalid(format!("JSON field {field} exceeds usize")))
}

fn u16_field(value: &Value, field: &str) -> Result<u16, StorageError> {
    u16::try_from(u64_field(value, field)?)
        .map_err(|_| invalid(format!("JSON field {field} exceeds u16")))
}

fn u256_field(value: &Value, field: &str) -> Result<U256, StorageError> {
    u256_from_string(field, string_field(value, field)?)
}

fn b256_field(value: &Value, field: &str) -> Result<B256, StorageError> {
    let bytes = parse_hex_fixed(field, string_field(value, field)?, 32)?;
    Ok(B256::from_slice(&bytes))
}

fn encode_b256(value: B256) -> String {
    encode_hex(value.as_slice())
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

fn parse_hex_fixed(field: &str, value: &str, byte_len: usize) -> Result<Vec<u8>, StorageError> {
    let raw = value
        .strip_prefix("0x")
        .ok_or_else(|| invalid(format!("{field} is missing 0x prefix")))?;
    if raw.len() != byte_len * 2 {
        return Err(invalid(format!(
            "{field} has incorrect hex length {}",
            raw.len()
        )));
    }

    let mut bytes = Vec::with_capacity(byte_len);
    let raw = raw.as_bytes();
    for index in (0..raw.len()).step_by(2) {
        let high = hex_nibble(raw[index])
            .ok_or_else(|| invalid(format!("{field} has invalid hex character")))?;
        let low = hex_nibble(raw[index + 1])
            .ok_or_else(|| invalid(format!("{field} has invalid hex character")))?;
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

fn invalid(message: impl Into<String>) -> StorageError {
    StorageError::Backend(message.into())
}
