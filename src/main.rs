use anyhow::Result;
use std::future::{Future, pending};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use rust_sync_proxy::allocator::compiled_allocator_name;
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
    tracing::info!("compiled allocator: {}", compiled_allocator_name());
    run_server(listener, config, shutdown_signal()).await?;
    Ok(())
}

async fn run_server<F>(listener: TcpListener, config: Config, shutdown: F) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, build_router(config))
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!("failed to listen for ctrl_c signal: {error}");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!("failed to listen for SIGTERM: {error}");
                pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!("shutdown signal received, draining server");
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    async fn wait_until_ready(port: u16) {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/does-not-exist");
        for _ in 0..100 {
            if client.get(&url).send().await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("server did not start listening in time");
    }

    #[tokio::test]
    async fn server_stops_after_shutdown_future_resolves() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = rust_sync_proxy::test_config();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let server: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            super::run_server(listener, config, async move {
                let _ = shutdown_rx.await;
            })
            .await
        });

        wait_until_ready(port).await;
        shutdown_tx.send(()).unwrap();

        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("server should stop after shutdown signal")
            .unwrap()
            .unwrap();
    }
}
