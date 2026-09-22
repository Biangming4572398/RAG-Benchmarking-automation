use backend::{
    config::{Catalog, Config},
    server::{Store, router_with_initialization},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("rag-benchmark-backend {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let config = Config::from_env()?;
    let catalog = Catalog::load(&config.catalog_path)?;
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    let store = Arc::new(Store::open(&config.data_dir)?);
    let app = router_with_initialization(store, config.nebula, catalog, config.nebula_corpus)?;
    println!("BENCHMARK_BACKEND_PORT={}", listener.local_addr()?.port());
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
