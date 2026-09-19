use backend::{config::Config, server::router, storage::Store};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let store = Arc::new(Store::open(&config.data_dir)?);
    let app = router(store, config.api_token, config.nebula)?;
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    println!("BENCHMARK_BACKEND_PORT={}", listener.local_addr()?.port());
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
