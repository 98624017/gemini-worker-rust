use anyhow::Result;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use rust_sync_proxy::build_router;
use rust_sync_proxy::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_process_env()?;
    let address = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&address).await?;

    tracing::info!("starting rust sync proxy on {}", address);
    axum::serve(listener, build_router(config)).await?;
    Ok(())
}
