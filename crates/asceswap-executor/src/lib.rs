use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use asceswap_api::{
    ApiClaimSide, ApiOrder, ApiSide, ListReservationsResponse, ReservationActionResponse,
    SettlementPayloadResponse,
};
use ethers::contract::abigen;
use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, Bytes, TransactionReceipt, H256, U256};
use reqwest::StatusCode;
use serde::Serialize;

abigen!(
    AsceSwapContract,
    r#"[
        {
            "type": "function",
            "name": "matchOrders",
            "stateMutability": "nonpayable",
            "inputs": [
                {
                    "name": "takerOrder",
                    "type": "tuple",
                    "components": [
                        {"name": "salt", "type": "uint256"},
                        {"name": "maker", "type": "address"},
                        {"name": "marketId", "type": "bytes32"},
                        {"name": "claim", "type": "uint8"},
                        {"name": "makerAmount", "type": "uint256"},
                        {"name": "takerAmount", "type": "uint256"},
                        {"name": "side", "type": "uint8"},
                        {"name": "expiration", "type": "uint256"},
                        {"name": "epoch", "type": "uint256"},
                        {"name": "maxFeeRateBps", "type": "uint16"}
                    ]
                },
                {"name": "takerSignature", "type": "bytes"},
                {
                    "name": "makerOrders",
                    "type": "tuple[]",
                    "components": [
                        {"name": "salt", "type": "uint256"},
                        {"name": "maker", "type": "address"},
                        {"name": "marketId", "type": "bytes32"},
                        {"name": "claim", "type": "uint8"},
                        {"name": "makerAmount", "type": "uint256"},
                        {"name": "takerAmount", "type": "uint256"},
                        {"name": "side", "type": "uint8"},
                        {"name": "expiration", "type": "uint256"},
                        {"name": "epoch", "type": "uint256"},
                        {"name": "maxFeeRateBps", "type": "uint16"}
                    ]
                },
                {"name": "makerSignatures", "type": "bytes[]"},
                {"name": "takerClaimFillAmount", "type": "uint256"},
                {"name": "makerClaimFillAmounts", "type": "uint256[]"}
            ],
            "outputs": []
        }
    ]"#
);

type ExecutorClient = SignerMiddleware<Provider<Http>, LocalWallet>;
type ContractOrder = (U256, Address, [u8; 32], u8, U256, U256, u8, U256, U256, u16);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutorConfig {
    pub api_url: String,
    pub rpc_url: String,
    pub exchange_address: Address,
    pub executor_private_key: String,
    pub chain_id: u64,
    pub poll_interval: Duration,
    pub reservation_limit: usize,
    pub confirmations: usize,
    pub dry_run: bool,
    pub release_on_simulation_failure: bool,
}

impl ExecutorConfig {
    pub fn from_env() -> Result<Self, ExecutorError> {
        Self::from_getter(|name| env::var(name).ok())
    }

    fn from_getter(get: impl Fn(&str) -> Option<String>) -> Result<Self, ExecutorError> {
        let api_url = required(&get, "ASCESWAP_API_URL")?
            .trim_end_matches('/')
            .to_string();
        let rpc_url = required(&get, "ASCESWAP_RPC_URL")?;
        let exchange_address = parse_address_env(&required(&get, "ASCESWAP_EXCHANGE_ADDRESS")?)?;
        let executor_private_key = required(&get, "ASCESWAP_EXECUTOR_PRIVATE_KEY")?;
        if !executor_private_key.starts_with("0x") {
            return Err(ExecutorError::Config(
                "ASCESWAP_EXECUTOR_PRIVATE_KEY must start with 0x".to_string(),
            ));
        }

        Ok(Self {
            api_url,
            rpc_url,
            exchange_address,
            executor_private_key,
            chain_id: parse_u64_env(&get, "ASCESWAP_CHAIN_ID", 421_614)?,
            poll_interval: Duration::from_secs(parse_u64_env(
                &get,
                "ASCESWAP_EXECUTOR_POLL_SECS",
                10,
            )?),
            reservation_limit: parse_usize_env(&get, "ASCESWAP_EXECUTOR_RESERVATION_LIMIT", 20)?,
            confirmations: parse_usize_env(&get, "ASCESWAP_EXECUTOR_CONFIRMATIONS", 1)?,
            dry_run: parse_bool_env(&get, "ASCESWAP_EXECUTOR_DRY_RUN", false)?,
            release_on_simulation_failure: parse_bool_env(
                &get,
                "ASCESWAP_EXECUTOR_RELEASE_ON_SIMULATION_FAILURE",
                false,
            )?,
        })
    }
}

pub async fn run_from_env() -> Result<(), ExecutorError> {
    let config = ExecutorConfig::from_env()?;
    let executor = Executor::connect(config).await?;
    executor.run_forever().await
}

#[derive(Clone)]
pub struct Executor {
    config: ExecutorConfig,
    backend: BackendClient,
    contract: AsceSwapContract<ExecutorClient>,
}

impl Executor {
    pub async fn connect(config: ExecutorConfig) -> Result<Self, ExecutorError> {
        let provider = Provider::<Http>::try_from(config.rpc_url.as_str())
            .map_err(|error| ExecutorError::Rpc(error.to_string()))?;
        let wallet = config
            .executor_private_key
            .parse::<LocalWallet>()
            .map_err(|error| {
                ExecutorError::Config(format!("invalid executor private key: {error}"))
            })?
            .with_chain_id(config.chain_id);
        let client = Arc::new(SignerMiddleware::new(provider, wallet));
        let contract = AsceSwapContract::new(config.exchange_address, client);
        let backend = BackendClient::new(config.api_url.clone());

        Ok(Self {
            config,
            backend,
            contract,
        })
    }

    pub async fn run_forever(&self) -> Result<(), ExecutorError> {
        let mut interval = tokio::time::interval(self.config.poll_interval);
        loop {
            interval.tick().await;
            if let Err(error) = self.run_once().await {
                eprintln!("executor poll failed: {error}");
            }
        }
    }

    pub async fn run_once(&self) -> Result<usize, ExecutorError> {
        let reservations = self
            .backend
            .list_reserved_reservations(self.config.reservation_limit)
            .await?;
        let mut attempted = 0;

        for reservation in reservations.reservations {
            attempted += 1;
            if let Err(error) = self.execute_reservation(&reservation.reservation_id).await {
                eprintln!(
                    "reservation {} execution failed: {error}",
                    reservation.reservation_id
                );
            }
        }

        Ok(attempted)
    }

    async fn execute_reservation(&self, reservation_id: &str) -> Result<(), ExecutorError> {
        let payload = self.backend.settlement_payload(reservation_id).await?;
        let args = settlement_to_contract_args(payload)?;
        let call = self.contract.match_orders(
            args.taker_order,
            args.taker_signature,
            args.maker_orders,
            args.maker_signatures,
            args.taker_claim_fill_amount,
            args.maker_claim_fill_amounts,
        );

        if let Err(error) = call.call().await {
            if self.config.release_on_simulation_failure {
                self.backend.release_reservation(reservation_id).await?;
            }
            return Err(ExecutorError::Simulation(error.to_string()));
        }

        if self.config.dry_run {
            println!("reservation {reservation_id} simulated successfully");
            return Ok(());
        }

        self.backend.mark_submitted(reservation_id).await?;
        let pending = call
            .send()
            .await
            .map_err(|error| ExecutorError::Transaction(error.to_string()))?;
        let tx_hash = *pending;
        println!("submitted reservation {reservation_id}: tx={tx_hash:?}");

        let receipt = pending
            .confirmations(self.config.confirmations)
            .await
            .map_err(|error| ExecutorError::Transaction(error.to_string()))?
            .ok_or_else(|| {
                ExecutorError::Transaction("transaction dropped before receipt".to_string())
            })?;

        if receipt_succeeded(&receipt) {
            self.backend.commit_reservation(reservation_id).await?;
            println!("committed reservation {reservation_id}: tx={tx_hash:?}");
        } else {
            self.backend.release_reservation(reservation_id).await?;
            return Err(ExecutorError::Transaction(format!(
                "transaction reverted: tx={tx_hash:?}"
            )));
        }

        Ok(())
    }
}

#[derive(Clone)]
struct BackendClient {
    base_url: String,
    client: reqwest::Client,
}

impl BackendClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    async fn list_reserved_reservations(
        &self,
        limit: usize,
    ) -> Result<ListReservationsResponse, ExecutorError> {
        self.get_json(&format!("/reservations?status=reserved&limit={limit}"))
            .await
    }

    async fn settlement_payload(
        &self,
        reservation_id: &str,
    ) -> Result<SettlementPayloadResponse, ExecutorError> {
        self.get_json(&format!("/reservations/{reservation_id}/settlement"))
            .await
    }

    async fn mark_submitted(&self, reservation_id: &str) -> Result<(), ExecutorError> {
        self.post_reservation_action(&format!("/reservations/{reservation_id}/submitted"))
            .await
    }

    async fn commit_reservation(&self, reservation_id: &str) -> Result<(), ExecutorError> {
        self.post_reservation_action(&format!("/reservations/{reservation_id}/commit"))
            .await
    }

    async fn release_reservation(&self, reservation_id: &str) -> Result<(), ExecutorError> {
        self.post_reservation_action(&format!("/reservations/{reservation_id}/release"))
            .await
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ExecutorError> {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|error| ExecutorError::Backend(error.to_string()))?;
        decode_backend_response(response).await
    }

    async fn post_reservation_action(&self, path: &str) -> Result<(), ExecutorError> {
        let body = ReservationActionBody { now: unix_now()? };
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .json(&body)
            .send()
            .await
            .map_err(|error| ExecutorError::Backend(error.to_string()))?;
        let _: ReservationActionResponse = decode_backend_response(response).await?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct ReservationActionBody {
    now: u64,
}

async fn decode_backend_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ExecutorError> {
    let status = response.status();
    if status != StatusCode::OK {
        let body = response.text().await.unwrap_or_default();
        return Err(ExecutorError::Backend(format!(
            "backend returned {status}: {body}"
        )));
    }

    response
        .json::<T>()
        .await
        .map_err(|error| ExecutorError::Backend(error.to_string()))
}

pub struct MatchOrdersArgs {
    pub taker_order: ContractOrder,
    pub taker_signature: Bytes,
    pub maker_orders: Vec<ContractOrder>,
    pub maker_signatures: Vec<Bytes>,
    pub taker_claim_fill_amount: U256,
    pub maker_claim_fill_amounts: Vec<U256>,
}

pub fn settlement_to_contract_args(
    payload: SettlementPayloadResponse,
) -> Result<MatchOrdersArgs, ExecutorError> {
    Ok(MatchOrdersArgs {
        taker_order: api_order_to_contract(payload.taker_order)?,
        taker_signature: parse_bytes(&payload.taker_signature, "taker_signature")?,
        maker_orders: payload
            .maker_orders
            .into_iter()
            .map(api_order_to_contract)
            .collect::<Result<Vec<_>, _>>()?,
        maker_signatures: payload
            .maker_signatures
            .iter()
            .map(|signature| parse_bytes(signature, "maker_signature"))
            .collect::<Result<Vec<_>, _>>()?,
        taker_claim_fill_amount: parse_u256(
            &payload.taker_claim_fill_amount,
            "taker_claim_fill_amount",
        )?,
        maker_claim_fill_amounts: payload
            .maker_claim_fill_amounts
            .iter()
            .map(|amount| parse_u256(amount, "maker_claim_fill_amount"))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn api_order_to_contract(order: ApiOrder) -> Result<ContractOrder, ExecutorError> {
    Ok((
        parse_u256(&order.salt, "order.salt")?,
        parse_address(&order.maker, "order.maker")?,
        parse_h256(&order.market_id, "order.market_id")?.to_fixed_bytes(),
        claim_to_u8(order.claim),
        parse_u256(&order.maker_amount, "order.maker_amount")?,
        parse_u256(&order.taker_amount, "order.taker_amount")?,
        side_to_u8(order.side),
        parse_u256(&order.expiration, "order.expiration")?,
        parse_u256(&order.epoch, "order.epoch")?,
        order.max_fee_rate_bps,
    ))
}

fn claim_to_u8(claim: ApiClaimSide) -> u8 {
    match claim {
        ApiClaimSide::Residual => 0,
        ApiClaimSide::Payoff => 1,
    }
}

fn side_to_u8(side: ApiSide) -> u8 {
    match side {
        ApiSide::Buy => 0,
        ApiSide::Sell => 1,
    }
}

fn parse_u256(value: &str, field: &'static str) -> Result<U256, ExecutorError> {
    U256::from_dec_str(value)
        .map_err(|_| ExecutorError::Config(format!("{field} is not a valid uint256 decimal")))
}

fn parse_address(value: &str, field: &'static str) -> Result<Address, ExecutorError> {
    value
        .parse::<Address>()
        .map_err(|_| ExecutorError::Config(format!("{field} is not a valid address")))
}

fn parse_address_env(value: &str) -> Result<Address, ExecutorError> {
    parse_address(value, "ASCESWAP_EXCHANGE_ADDRESS")
}

fn parse_h256(value: &str, field: &'static str) -> Result<H256, ExecutorError> {
    value
        .parse::<H256>()
        .map_err(|_| ExecutorError::Config(format!("{field} is not a valid bytes32")))
}

fn parse_bytes(value: &str, field: &'static str) -> Result<Bytes, ExecutorError> {
    let raw = value
        .strip_prefix("0x")
        .ok_or_else(|| ExecutorError::Config(format!("{field} must start with 0x")))?;
    if raw.len() % 2 != 0 {
        return Err(ExecutorError::Config(format!("{field} has odd hex length")));
    }

    let mut bytes = Vec::with_capacity(raw.len() / 2);
    for pair in raw.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0])
            .ok_or_else(|| ExecutorError::Config(format!("{field} has invalid hex")))?;
        let low = hex_nibble(pair[1])
            .ok_or_else(|| ExecutorError::Config(format!("{field} has invalid hex")))?;
        bytes.push((high << 4) | low);
    }

    Ok(Bytes::from(bytes))
}

fn required(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<String, ExecutorError> {
    get(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ExecutorError::Config(format!("{name} is required")))
}

fn parse_u64_env(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<u64, ExecutorError> {
    match get(name) {
        Some(value) if !value.is_empty() => value
            .parse::<u64>()
            .map_err(|_| ExecutorError::Config(format!("{name} must be a u64"))),
        Some(_) => Err(ExecutorError::Config(format!("{name} cannot be empty"))),
        None => Ok(default),
    }
}

fn parse_usize_env(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: usize,
) -> Result<usize, ExecutorError> {
    let value = match get(name) {
        Some(value) if !value.is_empty() => value
            .parse::<usize>()
            .map_err(|_| ExecutorError::Config(format!("{name} must be a usize")))?,
        Some(_) => return Err(ExecutorError::Config(format!("{name} cannot be empty"))),
        None => default,
    };
    if value == 0 {
        return Err(ExecutorError::Config(format!(
            "{name} must be greater than zero"
        )));
    }
    Ok(value)
}

fn parse_bool_env(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: bool,
) -> Result<bool, ExecutorError> {
    match get(name).as_deref() {
        Some("true") | Some("1") | Some("yes") => Ok(true),
        Some("false") | Some("0") | Some("no") => Ok(false),
        Some("") => Err(ExecutorError::Config(format!("{name} cannot be empty"))),
        Some(_) => Err(ExecutorError::Config(format!("{name} must be boolean"))),
        None => Ok(default),
    }
}

fn unix_now() -> Result<u64, ExecutorError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| ExecutorError::Clock(error.to_string()))
}

fn receipt_succeeded(receipt: &TransactionReceipt) -> bool {
    receipt.status == Some(1_u64.into())
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug)]
pub enum ExecutorError {
    Backend(String),
    Clock(String),
    Config(String),
    Rpc(String),
    Simulation(String),
    Transaction(String),
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(message) => write!(formatter, "backend error: {message}"),
            Self::Clock(message) => write!(formatter, "clock error: {message}"),
            Self::Config(message) => write!(formatter, "config error: {message}"),
            Self::Rpc(message) => write!(formatter, "rpc error: {message}"),
            Self::Simulation(message) => write!(formatter, "simulation error: {message}"),
            Self::Transaction(message) => write!(formatter, "transaction error: {message}"),
        }
    }
}

impl std::error::Error for ExecutorError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_getter(name: &str) -> Option<String> {
        match name {
            "ASCESWAP_API_URL" => Some("http://localhost:8080/".to_string()),
            "ASCESWAP_RPC_URL" => Some("https://sepolia-rollup.arbitrum.io/rpc".to_string()),
            "ASCESWAP_EXCHANGE_ADDRESS" => {
                Some("0x346457c948EaA86Afa9392B9E790bE2E42c6ebD6".to_string())
            }
            "ASCESWAP_EXECUTOR_PRIVATE_KEY" => Some(format!("0x{}", "11".repeat(32))),
            _ => None,
        }
    }

    #[test]
    fn config_uses_safe_defaults_and_normalizes_api_url() {
        let config = ExecutorConfig::from_getter(config_getter).unwrap();

        assert_eq!(config.api_url, "http://localhost:8080");
        assert_eq!(config.chain_id, 421_614);
        assert_eq!(config.poll_interval, Duration::from_secs(10));
        assert_eq!(config.reservation_limit, 20);
        assert_eq!(config.confirmations, 1);
        assert!(!config.dry_run);
        assert!(!config.release_on_simulation_failure);
    }

    #[test]
    fn config_rejects_private_key_without_0x_prefix() {
        let error = ExecutorConfig::from_getter(|name| {
            if name == "ASCESWAP_EXECUTOR_PRIVATE_KEY" {
                Some("11".repeat(32))
            } else {
                config_getter(name)
            }
        })
        .unwrap_err();

        assert!(error.to_string().contains("must start with 0x"));
    }

    #[test]
    fn converts_settlement_payload_to_contract_args() {
        let order = ApiOrder {
            salt: "1".to_string(),
            maker: "0x1111111111111111111111111111111111111111".to_string(),
            market_id: format!("0x{}", "22".repeat(32)),
            claim: ApiClaimSide::Payoff,
            maker_amount: "100".to_string(),
            taker_amount: "40".to_string(),
            side: ApiSide::Sell,
            expiration: "0".to_string(),
            epoch: "1".to_string(),
            max_fee_rate_bps: 100,
        };
        let payload = SettlementPayloadResponse {
            taker_order: order.clone(),
            taker_signature: format!("0x{}", "aa".repeat(65)),
            maker_orders: vec![order],
            maker_signatures: vec![format!("0x{}", "bb".repeat(65))],
            taker_claim_fill_amount: "100".to_string(),
            maker_claim_fill_amounts: vec!["100".to_string()],
        };

        let args = settlement_to_contract_args(payload).unwrap();

        assert_eq!(args.taker_order.3, 1);
        assert_eq!(args.taker_order.6, 1);
        assert_eq!(args.taker_signature.len(), 65);
        assert_eq!(args.maker_orders.len(), 1);
        assert_eq!(args.maker_signatures[0].len(), 65);
        assert_eq!(args.taker_claim_fill_amount, U256::from(100_u64));
        assert_eq!(args.maker_claim_fill_amounts, vec![U256::from(100_u64)]);
    }
}
