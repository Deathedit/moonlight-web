mod api;
mod boxart;
mod config;
mod discovery;
mod hosts;
mod identity;
mod nvhttp;
mod pairing;
mod stream;
mod util;

use std::sync::Arc;

use axum::Router;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn".into()))
        .init();

    let config = Arc::new(config::Config::from_env()?);
    tokio::fs::create_dir_all(&config.data_dir).await?;
    let identity = Arc::new(identity::Identity::load_or_create(&config.data_dir).await?);
    let state = api::AppState {
        config: config.clone(),
        hosts: Arc::new(hosts::HostStore::load(&config.data_dir).await?),
        nv: nvhttp::NvClient::new(identity),
        pairing: Default::default(),
        boxart: Arc::new(boxart::BoxArtCache::new(&config.data_dir)),
        streams: Default::default(),
        discovery: discovery::Discovery::start(),
    };

    // SPA: unknown paths fall back to index.html so client-side routes work on reload.
    let spa = ServeDir::new(&config.web_dir)
        .not_found_service(ServeFile::new(config.web_dir.join("index.html")));

    let app = Router::new()
        .nest("/api", api::router(state))
        .fallback_service(spa)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!("Moonlight Web listening on http://{}", config.bind);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
