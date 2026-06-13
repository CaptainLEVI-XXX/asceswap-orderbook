#[tokio::main]
async fn main() {
    if let Err(error) = asceswap_adapter_keeper::run_from_env().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
