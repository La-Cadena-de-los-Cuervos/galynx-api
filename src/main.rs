mod app;
mod attachments;
mod audit;
mod auth;
mod channels;
mod config;
mod errors;
mod reactions;
mod realtime;
mod rate_limit;
mod storage;
mod threads;

use std::net::SocketAddr;

use tracing::info;

#[tokio::main]
async fn main() {
    setup_tracing();

    let config = config::Config::from_env();
    let app_state = app::build_state(config).await;
    let backend = app_state.storage.backend();
    let port = app_state.config.port;
    let app = app::router(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("starting galynx-api on {}", addr);
    match backend {
        storage::PersistenceBackend::Memory => info!("persistence backend: memory"),
        storage::PersistenceBackend::Mongo => info!("persistence backend: mongo"),
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("server terminated with error");
}

fn setup_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "galynx_api=debug,tower_http=info".into());

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .compact()
        .init();
}
