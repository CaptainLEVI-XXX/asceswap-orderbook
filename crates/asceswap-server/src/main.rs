use std::env;
use std::net::SocketAddr;

use asceswap_api::{spawn_actor_orderbook_api_service, ActorOrderbookApiService};
use asceswap_matcher::MatchConfig;
use asceswap_postgres::PostgresEngineStore;
use asceswap_server::actor_router;
use asceswap_types::{Address, U256};
use asceswap_validation::SignatureDomain;

const MARKET_ACTOR_INBOX_CAPACITY: usize = 1_024;

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
    let listen_addr = env::var("ASCESWAP_LISTEN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid ASCESWAP_LISTEN_ADDR: {error}"))?;
    let bootstrap_schema = env_bool("ASCESWAP_BOOTSTRAP_SCHEMA", true)?;
    let signature_domain = SignatureDomain::new(
        env_u256("ASCESWAP_CHAIN_ID")?,
        env_address("ASCESWAP_EXCHANGE_ADDRESS")?,
    );

    let mut store =
        PostgresEngineStore::connect(&postgres_url).map_err(|error| format!("{error:?}"))?;
    if bootstrap_schema {
        store.run_schema().map_err(|error| format!("{error:?}"))?;
    }

    let service = ActorOrderbookApiService::recover_from_store(
        store,
        MatchConfig::default(),
        MARKET_ACTOR_INBOX_CAPACITY,
    )
    .map_err(|error| format!("{error:?}"))?
    .with_signature_domain(signature_domain);
    let service = spawn_actor_orderbook_api_service(service);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .map_err(|error| format!("failed to bind {listen_addr}: {error}"))?;
    axum::serve(listener, actor_router(service))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("server failed: {error}"))
}

fn env_u256(name: &str) -> Result<U256, String> {
    let value = env::var(name).map_err(|_| format!("{name} is required"))?;
    if value.is_empty() {
        return Err(format!("{name} cannot be empty"));
    }

    U256::from_str_radix(&value, 10)
        .map_err(|_| format!("invalid {name}: expected decimal uint256"))
}

fn env_address(name: &str) -> Result<Address, String> {
    let value = env::var(name).map_err(|_| format!("{name} is required"))?;
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
