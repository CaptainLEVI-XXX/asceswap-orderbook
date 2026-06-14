#[tokio::main]
async fn main() {
    if let Err(error) = run_worker_from_env().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run_worker_from_env() -> Result<(), String> {
    if !keeper_enabled() {
        return asceswap_executor::run_from_env()
            .await
            .map_err(|error| error.to_string());
    }

    println!("starting executor and adapter keeper workers");
    tokio::select! {
        result = asceswap_executor::run_from_env() => {
            result.map_err(|error| error.to_string())
        }
        result = asceswap_adapter_keeper::run_from_env() => {
            result.map_err(|error| error.to_string())
        }
    }
}

fn keeper_enabled() -> bool {
    std::env::var("ASCESWAP_KEEPER_PRIVATE_KEY")
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}
