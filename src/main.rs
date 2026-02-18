mod app;
mod attachments;
mod audit;
mod auth;
mod channels;
mod config;
mod errors;
mod observability;
mod rate_limit;
mod reactions;
mod realtime;
mod storage;
mod threads;
mod users;
mod workspaces;

use std::net::SocketAddr;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    trace::{Sampler, SdkTracerProvider},
};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    let config = config::Config::from_env();
    let _telemetry = setup_tracing(&config);
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

struct TelemetryGuard {
    provider: Option<SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            let _ = provider.shutdown();
        }
    }
}

fn setup_tracing(config: &config::Config) -> TelemetryGuard {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "galynx_api=debug,tower_http=info".into());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact();
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(endpoint) = config
        .otel_exporter_otlp_endpoint
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.to_string())
            .build()
            .expect("failed to initialize OTLP exporter");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name(config.otel_service_name.clone())
                    .with_attributes([KeyValue::new(
                        "service.version",
                        env!("CARGO_PKG_VERSION").to_string(),
                    )])
                    .build(),
            )
            .with_sampler(Sampler::TraceIdRatioBased(config.otel_sample_ratio))
            .with_batch_exporter(exporter)
            .build();

        let tracer = provider.tracer(config.otel_service_name.clone());
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        registry.with(otel_layer).init();
        info!("otlp tracing enabled");
        TelemetryGuard {
            provider: Some(provider),
        }
    } else {
        registry.init();
        TelemetryGuard { provider: None }
    }
}
