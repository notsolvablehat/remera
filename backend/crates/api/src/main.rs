use std::fs::OpenOptions;

use std::net::SocketAddr;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{config::AppConfig, router::build_router, state::AppState};

mod config;
mod extractors;
mod middleware;
mod router;
mod routes;
mod state;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = AppConfig::from_env();
    let port = config.port;

    let state = AppState::new(config).await;
    let app = build_router(state);

    let addr: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .expect("Invalid bind address");

    tracing::info!(%addr, "starting server");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind to port");

    tracing::info!("SERVER RUNNING AT http://{addr}");
    // `with_connect_info` (not plain `into_make_service()`) is required —
    // the rate limiter's PeerIpKeyExtractor reads the peer's SocketAddr
    // from request extensions, which only this variant populates.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("Failed to start the server.");
}

fn init_tracing() {
    let env = std::env::var("APP_ENV").unwrap_or_else(|_| "production".to_string());

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(filter);

    if env == "development" {
        registry
            .with(tracing_subscriber::fmt::layer().with_ansi(true))
            .init();
    } else {
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open("app.log")
            .expect("Failed to open log file");

        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(log_file),
            )
            .init();
    }
}
