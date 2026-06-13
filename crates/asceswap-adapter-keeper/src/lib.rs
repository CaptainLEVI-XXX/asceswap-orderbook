use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use ethers::contract::abigen;
use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, TransactionReceipt, H256};

abigen!(
    TwaAdapterContract,
    r#"[
        {
            "type": "function",
            "name": "poke",
            "stateMutability": "nonpayable",
            "inputs": [{"name": "marketId", "type": "bytes32"}],
            "outputs": [{"name": "valueWad", "type": "int256"}]
        }
    ]"#
);

type KeeperClient = SignerMiddleware<Provider<Http>, LocalWallet>;

const DEFAULT_RPC_URL: &str = "https://sepolia-rollup.arbitrum.io/rpc";
const DEFAULT_CHAIN_ID: u64 = 421_614;
const DEFAULT_TARGETS: &str = concat!(
    "aave-usdc-borrow=0x3B9D6fF6d0C798317f3B51681e335f5b07cbD70F:",
    "0x2f56d7c26e665a04dd24404cdd841d6fcd7fd402a3b127760e2598c64d2df369;",
    "arbitrum-gas=0x81aA57736801E33f8ef059F79B8F4332416D4DB8:",
    "0xfe77931a0aa6baee55370819b38cb10feb3f03e2c0053a9a37e3213a471b7f28"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeeperConfig {
    pub rpc_url: String,
    pub private_key: String,
    pub chain_id: u64,
    pub targets: Vec<KeeperTarget>,
    pub interval: Duration,
    pub retry_interval: Duration,
    pub confirmations: usize,
    pub dry_run: bool,
}

impl KeeperConfig {
    pub fn from_env() -> Result<Self, KeeperError> {
        Self::from_getter(|name| env::var(name).ok())
    }

    fn from_getter(get: impl Fn(&str) -> Option<String>) -> Result<Self, KeeperError> {
        let rpc_url = match optional_string(&get, "ASCESWAP_KEEPER_RPC_URL")? {
            Some(value) => value,
            None => optional_string(&get, "ASCESWAP_RPC_URL")?
                .unwrap_or_else(|| DEFAULT_RPC_URL.to_string()),
        };
        let private_key = required(&get, "ASCESWAP_KEEPER_PRIVATE_KEY")?;
        if !private_key.starts_with("0x") {
            return Err(KeeperError::Config(
                "ASCESWAP_KEEPER_PRIVATE_KEY must start with 0x".to_string(),
            ));
        }

        Ok(Self {
            rpc_url,
            private_key,
            chain_id: parse_u64_env(&get, "ASCESWAP_KEEPER_CHAIN_ID", DEFAULT_CHAIN_ID)?,
            targets: parse_targets(
                &optional_string(&get, "ASCESWAP_KEEPER_TARGETS")?
                    .unwrap_or_else(|| DEFAULT_TARGETS.to_string()),
            )?,
            interval: Duration::from_secs(parse_u64_env(
                &get,
                "ASCESWAP_KEEPER_INTERVAL_SECS",
                300,
            )?),
            retry_interval: Duration::from_secs(parse_u64_env(
                &get,
                "ASCESWAP_KEEPER_RETRY_SECS",
                60,
            )?),
            confirmations: parse_usize_env(&get, "ASCESWAP_KEEPER_CONFIRMATIONS", 1)?,
            dry_run: parse_bool_env(&get, "ASCESWAP_KEEPER_DRY_RUN", false)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeeperTarget {
    pub label: Option<String>,
    pub adapter: Address,
    pub market_id: H256,
}

impl KeeperTarget {
    fn display_name(&self) -> String {
        self.label
            .clone()
            .unwrap_or_else(|| format!("{:?}: {:?}", self.adapter, self.market_id))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeeperCycleReport {
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
}

pub async fn run_from_env() -> Result<(), KeeperError> {
    let config = KeeperConfig::from_env()?;
    let keeper = AdapterKeeper::connect(config).await?;
    keeper.run_forever().await
}

#[derive(Clone)]
pub struct AdapterKeeper {
    config: KeeperConfig,
    client: Arc<KeeperClient>,
}

impl AdapterKeeper {
    pub async fn connect(config: KeeperConfig) -> Result<Self, KeeperError> {
        let provider = Provider::<Http>::try_from(config.rpc_url.as_str())
            .map_err(|error| KeeperError::Rpc(error.to_string()))?;
        let wallet = config
            .private_key
            .parse::<LocalWallet>()
            .map_err(|error| KeeperError::Config(format!("invalid keeper private key: {error}")))?
            .with_chain_id(config.chain_id);
        let client = Arc::new(SignerMiddleware::new(provider, wallet));

        Ok(Self { config, client })
    }

    pub async fn run_forever(&self) -> Result<(), KeeperError> {
        loop {
            let report = self.run_once().await?;
            let delay = if report.failed == 0 {
                self.config.interval
            } else {
                self.config.retry_interval
            };

            println!(
                "adapter keeper cycle complete: attempted={} succeeded={} failed={} next_tick_secs={}",
                report.attempted,
                report.succeeded,
                report.failed,
                delay.as_secs()
            );
            tokio::time::sleep(delay).await;
        }
    }

    pub async fn run_once(&self) -> Result<KeeperCycleReport, KeeperError> {
        let mut report = KeeperCycleReport::default();

        for target in &self.config.targets {
            report.attempted += 1;
            match self.poke_target(target).await {
                Ok(()) => report.succeeded += 1,
                Err(error) => {
                    report.failed += 1;
                    eprintln!("adapter poke failed for {}: {error}", target.display_name());
                }
            }
        }

        Ok(report)
    }

    async fn poke_target(&self, target: &KeeperTarget) -> Result<(), KeeperError> {
        let contract = TwaAdapterContract::new(target.adapter, self.client.clone());
        let call = contract.poke(target.market_id.to_fixed_bytes());
        let value_wad = call
            .call()
            .await
            .map_err(|error| KeeperError::Simulation(error.to_string()))?;

        if self.config.dry_run {
            println!(
                "adapter poke simulated: target={} value_wad={value_wad:?}",
                target.display_name()
            );
            return Ok(());
        }

        let pending = call
            .send()
            .await
            .map_err(|error| KeeperError::Transaction(error.to_string()))?;
        let tx_hash = *pending;
        println!(
            "adapter poke submitted: target={} tx={tx_hash:?}",
            target.display_name()
        );

        let receipt = pending
            .confirmations(self.config.confirmations)
            .await
            .map_err(|error| KeeperError::Transaction(error.to_string()))?
            .ok_or_else(|| {
                KeeperError::Transaction("transaction dropped before receipt".to_string())
            })?;

        if !receipt_succeeded(&receipt) {
            return Err(KeeperError::Transaction(format!(
                "transaction reverted: tx={tx_hash:?}"
            )));
        }

        println!(
            "adapter poke confirmed: target={} tx={tx_hash:?}",
            target.display_name()
        );
        Ok(())
    }
}

pub fn parse_targets(value: &str) -> Result<Vec<KeeperTarget>, KeeperError> {
    let mut targets = Vec::new();

    for raw_entry in value.split([',', ';', '\n']) {
        let raw_entry = raw_entry.trim();
        if raw_entry.is_empty() {
            continue;
        }

        let (label, target) = match raw_entry.split_once('=') {
            Some((label, target)) => {
                let label = label.trim();
                if label.is_empty() {
                    return Err(KeeperError::Config(
                        "keeper target label cannot be empty".to_string(),
                    ));
                }
                (Some(label.to_string()), target.trim())
            }
            None => (None, raw_entry),
        };
        let (adapter, market_id) = target.split_once(':').ok_or_else(|| {
            KeeperError::Config(
                "keeper targets must use label=adapter:marketId or adapter:marketId".to_string(),
            )
        })?;

        targets.push(KeeperTarget {
            label,
            adapter: parse_address(adapter.trim(), "keeper target adapter")?,
            market_id: parse_h256(market_id.trim(), "keeper target marketId")?,
        });
    }

    if targets.is_empty() {
        return Err(KeeperError::Config(
            "ASCESWAP_KEEPER_TARGETS must contain at least one target".to_string(),
        ));
    }

    Ok(targets)
}

fn parse_address(value: &str, field: &'static str) -> Result<Address, KeeperError> {
    value
        .parse::<Address>()
        .map_err(|_| KeeperError::Config(format!("{field} is not a valid address")))
}

fn parse_h256(value: &str, field: &'static str) -> Result<H256, KeeperError> {
    value
        .parse::<H256>()
        .map_err(|_| KeeperError::Config(format!("{field} is not a valid bytes32")))
}

fn required(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<String, KeeperError> {
    get(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| KeeperError::Config(format!("{name} is required")))
}

fn optional_string(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<String>, KeeperError> {
    match get(name) {
        Some(value) if !value.is_empty() => Ok(Some(value)),
        Some(_) => Err(KeeperError::Config(format!("{name} cannot be empty"))),
        None => Ok(None),
    }
}

fn parse_u64_env(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<u64, KeeperError> {
    match get(name) {
        Some(value) if !value.is_empty() => value
            .parse::<u64>()
            .map_err(|_| KeeperError::Config(format!("{name} must be a u64"))),
        Some(_) => Err(KeeperError::Config(format!("{name} cannot be empty"))),
        None => Ok(default),
    }
}

fn parse_usize_env(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: usize,
) -> Result<usize, KeeperError> {
    let value = match get(name) {
        Some(value) if !value.is_empty() => value
            .parse::<usize>()
            .map_err(|_| KeeperError::Config(format!("{name} must be a usize")))?,
        Some(_) => return Err(KeeperError::Config(format!("{name} cannot be empty"))),
        None => default,
    };
    if value == 0 {
        return Err(KeeperError::Config(format!(
            "{name} must be greater than zero"
        )));
    }
    Ok(value)
}

fn parse_bool_env(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: bool,
) -> Result<bool, KeeperError> {
    match get(name).as_deref() {
        Some("true") | Some("1") | Some("yes") => Ok(true),
        Some("false") | Some("0") | Some("no") => Ok(false),
        Some("") => Err(KeeperError::Config(format!("{name} cannot be empty"))),
        Some(_) => Err(KeeperError::Config(format!("{name} must be boolean"))),
        None => Ok(default),
    }
}

fn receipt_succeeded(receipt: &TransactionReceipt) -> bool {
    receipt.status == Some(1_u64.into())
}

#[derive(Debug)]
pub enum KeeperError {
    Config(String),
    Rpc(String),
    Simulation(String),
    Transaction(String),
}

impl fmt::Display for KeeperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(formatter, "config error: {message}"),
            Self::Rpc(message) => write!(formatter, "rpc error: {message}"),
            Self::Simulation(message) => write!(formatter, "simulation error: {message}"),
            Self::Transaction(message) => write!(formatter, "transaction error: {message}"),
        }
    }
}

impl std::error::Error for KeeperError {}

#[cfg(test)]
mod tests {
    use super::*;

    const AAVE_ADAPTER: &str = "0x3B9D6fF6d0C798317f3B51681e335f5b07cbD70F";
    const AAVE_MARKET_ID: &str =
        "0x2f56d7c26e665a04dd24404cdd841d6fcd7fd402a3b127760e2598c64d2df369";
    const GAS_ADAPTER: &str = "0x81aA57736801E33f8ef059F79B8F4332416D4DB8";
    const GAS_MARKET_ID: &str =
        "0xfe77931a0aa6baee55370819b38cb10feb3f03e2c0053a9a37e3213a471b7f28";

    fn config_getter(name: &str) -> Option<String> {
        match name {
            "ASCESWAP_KEEPER_PRIVATE_KEY" => Some(format!("0x{}", "11".repeat(32))),
            _ => None,
        }
    }

    #[test]
    fn config_uses_safe_defaults() {
        let config = KeeperConfig::from_getter(config_getter).unwrap();

        assert_eq!(config.chain_id, 421_614);
        assert_eq!(config.interval, Duration::from_secs(300));
        assert_eq!(config.retry_interval, Duration::from_secs(60));
        assert_eq!(config.confirmations, 1);
        assert!(!config.dry_run);
        assert_eq!(config.targets.len(), 2);
        assert_eq!(config.rpc_url, DEFAULT_RPC_URL);
    }

    #[test]
    fn config_can_fallback_to_shared_rpc_url() {
        let config = KeeperConfig::from_getter(|name| match name {
            "ASCESWAP_KEEPER_RPC_URL" => None,
            "ASCESWAP_RPC_URL" => Some("https://sepolia-rollup.arbitrum.io/rpc".to_string()),
            _ => config_getter(name),
        })
        .unwrap();

        assert_eq!(
            config.rpc_url,
            "https://sepolia-rollup.arbitrum.io/rpc".to_string()
        );
    }

    #[test]
    fn config_rejects_private_key_without_0x_prefix() {
        let error = KeeperConfig::from_getter(|name| {
            if name == "ASCESWAP_KEEPER_PRIVATE_KEY" {
                Some("11".repeat(32))
            } else {
                config_getter(name)
            }
        })
        .unwrap_err();

        assert!(error.to_string().contains("must start with 0x"));
    }

    #[test]
    fn parses_labelled_and_unlabelled_targets() {
        let targets = parse_targets(&format!(
            "aave={AAVE_ADAPTER}:{AAVE_MARKET_ID}, {GAS_ADAPTER}:{GAS_MARKET_ID}"
        ))
        .unwrap();

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].label.as_deref(), Some("aave"));
        assert_eq!(targets[0].adapter, AAVE_ADAPTER.parse::<Address>().unwrap());
        assert_eq!(
            targets[0].market_id,
            AAVE_MARKET_ID.parse::<H256>().unwrap()
        );
        assert_eq!(targets[1].label, None);
        assert_eq!(targets[1].adapter, GAS_ADAPTER.parse::<Address>().unwrap());
        assert_eq!(targets[1].market_id, GAS_MARKET_ID.parse::<H256>().unwrap());
    }

    #[test]
    fn rejects_empty_targets() {
        let error = parse_targets(" , ; \n ").unwrap_err();

        assert!(error.to_string().contains("at least one target"));
    }
}
