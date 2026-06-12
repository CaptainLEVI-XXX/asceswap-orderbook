#[tokio::main]
async fn main() {
    if let Err(error) = asceswap_executor::run_from_env().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
