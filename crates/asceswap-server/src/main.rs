use std::env;
use std::net::SocketAddr;

use asceswap_api::{spawn_actor_orderbook_api_service, ActorOrderbookApiService, DemoMarketMaker};
use asceswap_matcher::MatchConfig;
use asceswap_postgres::PostgresEngineStore;
use asceswap_server::actor_router;
use asceswap_types::{Address, U256};
use asceswap_validation::SignatureDomain;

const MARKET_ACTOR_INBOX_CAPACITY: usize = 1_024;
const DEFAULT_CHAIN_ID: u64 = 421_614;
const DEFAULT_EXCHANGE_ADDRESS: &str = "0x346457c948EaA86Afa9392B9E790bE2E42c6ebD6";

#[tokio::main]
async fn main() {
    if let Err(error) = run_from_env().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run_from_env() -> Result<(), String> {
    let postgres_url = env::var("ASCESWAP_POSTGRES_URL")
        .map_err(|_| "ASCESWAP_POSTGRES_URL is required".to_string())?;
    let listen_addr = listen_addr_from_env()?;
    let bootstrap_schema = env_bool("ASCESWAP_BOOTSTRAP_SCHEMA", true)?;
    let signature_domain = SignatureDomain::new(
        env_u256_default("ASCESWAP_CHAIN_ID", U256::from(DEFAULT_CHAIN_ID))?,
        env_address_default("ASCESWAP_EXCHANGE_ADDRESS", DEFAULT_EXCHANGE_ADDRESS)?,
    );

    let mut store =
        PostgresEngineStore::connect(&postgres_url).map_err(|error| format!("{error:?}"))?;
    if bootstrap_schema {
        store.run_schema().map_err(|error| format!("{error:?}"))?;
    }

    let mut service = ActorOrderbookApiService::recover_from_store(
        store,
        MatchConfig::default(),
        MARKET_ACTOR_INBOX_CAPACITY,
    )
    .map_err(|error| format!("{error:?}"))?
    .with_signature_domain(signature_domain);
    if let Some(private_key) = env_private_key("ASCESWAP_DEMO_MM_PRIVATE_KEY")? {
        let demo_market_maker = DemoMarketMaker::new(
            private_key,
            signature_domain,
            env_u256_default("ASCESWAP_DEMO_MM_EPOCH", U256::from(1))?,
            env_u16_default("ASCESWAP_DEMO_MM_MAX_FEE_RATE_BPS", 100)?,
            env_optional_u64("ASCESWAP_DEMO_MM_RESERVATION_TTL_SECS")?.or(Some(30)),
            env_bool("ASCESWAP_DEMO_MM_AUTO_COMMIT", false)?,
        )
        .map_err(|error| format!("{error:?}"))?;
        service = service.with_demo_market_maker(demo_market_maker);
    }
    let service = spawn_actor_orderbook_api_service(service);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .map_err(|error| format!("failed to bind {listen_addr}: {error}"))?;
    axum::serve(listener, actor_router(service))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("server failed: {error}"))
}

fn listen_addr_from_env() -> Result<SocketAddr, String> {
    if let Ok(value) = env::var("ASCESWAP_LISTEN_ADDR") {
        if value.is_empty() {
            return Err("ASCESWAP_LISTEN_ADDR cannot be empty".to_string());
        }
        return value
            .parse::<SocketAddr>()
            .map_err(|error| format!("invalid ASCESWAP_LISTEN_ADDR: {error}"));
    }

    if let Ok(port) = env::var("PORT") {
        if port.is_empty() {
            return Err("PORT cannot be empty".to_string());
        }
        return format!("0.0.0.0:{port}")
            .parse::<SocketAddr>()
            .map_err(|error| format!("invalid PORT: {error}"));
    }

    "127.0.0.1:8080"
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid default listen address: {error}"))
}

fn env_u256_default(name: &str, default: U256) -> Result<U256, String> {
    match env::var(name) {
        Ok(value) => {
            if value.is_empty() {
                return Err(format!("{name} cannot be empty"));
            }
            U256::from_str_radix(&value, 10)
                .map_err(|_| format!("invalid {name}: expected decimal uint256"))
        }
        Err(_) => Ok(default),
    }
}

fn env_u16_default(name: &str, default: u16) -> Result<u16, String> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u16>()
            .map_err(|_| format!("invalid {name}: expected u16")),
        Err(_) => Ok(default),
    }
}

fn env_optional_u64(name: &str) -> Result<Option<u64>, String> {
    match env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} cannot be empty")),
        Ok(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("invalid {name}: expected u64")),
        Err(_) => Ok(None),
    }
}

fn env_private_key(name: &str) -> Result<Option<[u8; 32]>, String> {
    let value = match env::var(name) {
        Ok(value) if value.is_empty() => return Err(format!("{name} cannot be empty")),
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let raw = value
        .strip_prefix("0x")
        .ok_or_else(|| format!("invalid {name}: missing 0x prefix"))?;
    if raw.len() != 64 {
        return Err(format!("invalid {name}: expected 32-byte hex private key"));
    }

    let mut bytes = [0_u8; 32];
    for (index, chunk) in raw.as_bytes().chunks_exact(2).enumerate() {
        let high =
            hex_nibble(chunk[0]).ok_or_else(|| format!("invalid {name}: invalid hex character"))?;
        let low =
            hex_nibble(chunk[1]).ok_or_else(|| format!("invalid {name}: invalid hex character"))?;
        bytes[index] = (high << 4) | low;
    }

    Ok(Some(bytes))
}

fn env_address_default(name: &str, default: &str) -> Result<Address, String> {
    match env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} cannot be empty")),
        Ok(value) => parse_address_value(name, &value),
        Err(_) => parse_address_value(name, default),
    }
}

fn parse_address_value(name: &str, value: &str) -> Result<Address, String> {
    let raw = value
        .strip_prefix("0x")
        .ok_or_else(|| format!("invalid {name}: missing 0x prefix"))?;
    if raw.len() != 40 {
        return Err(format!("invalid {name}: expected 20-byte hex address"));
    }

    let mut bytes = [0_u8; 20];
    for (index, chunk) in raw.as_bytes().chunks_exact(2).enumerate() {
        let high =
            hex_nibble(chunk[0]).ok_or_else(|| format!("invalid {name}: invalid hex character"))?;
        let low =
            hex_nibble(chunk[1]).ok_or_else(|| format!("invalid {name}: invalid hex character"))?;
        bytes[index] = (high << 4) | low;
    }

    Ok(Address::from(bytes))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn env_bool(name: &str, default: bool) -> Result<bool, String> {
    match env::var(name) {
        Ok(value) if matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES") => Ok(true),
        Ok(value) if matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO") => Ok(false),
        Ok(value) => Err(format!("invalid {name}: {value}")),
        Err(_) => Ok(default),
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
