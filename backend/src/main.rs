#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rosso_backend::run_server().await
}
