use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use axum::{
    extract::State,
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::app::AppState;

#[derive(Debug)]
pub struct AppMetrics {
    in_flight: AtomicU64,
    requests_total: AtomicU64,
    requests_2xx: AtomicU64,
    requests_4xx: AtomicU64,
    requests_5xx: AtomicU64,
    latency_ms_le_50: AtomicU64,
    latency_ms_le_100: AtomicU64,
    latency_ms_le_250: AtomicU64,
    latency_ms_le_500: AtomicU64,
    latency_ms_le_1000: AtomicU64,
    latency_ms_le_2500: AtomicU64,
    latency_ms_le_5000: AtomicU64,
    latency_ms_inf: AtomicU64,
}

impl Default for AppMetrics {
    fn default() -> Self {
        Self {
            in_flight: AtomicU64::new(0),
            requests_total: AtomicU64::new(0),
            requests_2xx: AtomicU64::new(0),
            requests_4xx: AtomicU64::new(0),
            requests_5xx: AtomicU64::new(0),
            latency_ms_le_50: AtomicU64::new(0),
            latency_ms_le_100: AtomicU64::new(0),
            latency_ms_le_250: AtomicU64::new(0),
            latency_ms_le_500: AtomicU64::new(0),
            latency_ms_le_1000: AtomicU64::new(0),
            latency_ms_le_2500: AtomicU64::new(0),
            latency_ms_le_5000: AtomicU64::new(0),
            latency_ms_inf: AtomicU64::new(0),
        }
    }
}

impl AppMetrics {
    pub fn on_request_start(&self) {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    pub fn on_request_end(&self, status: u16, duration: Duration) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
        self.requests_total.fetch_add(1, Ordering::Relaxed);

        if (200..300).contains(&status) {
            self.requests_2xx.fetch_add(1, Ordering::Relaxed);
        } else if (400..500).contains(&status) {
            self.requests_4xx.fetch_add(1, Ordering::Relaxed);
        } else if status >= 500 {
            self.requests_5xx.fetch_add(1, Ordering::Relaxed);
        }

        let ms = duration.as_millis() as u64;
        if ms <= 50 {
            self.latency_ms_le_50.fetch_add(1, Ordering::Relaxed);
        } else if ms <= 100 {
            self.latency_ms_le_100.fetch_add(1, Ordering::Relaxed);
        } else if ms <= 250 {
            self.latency_ms_le_250.fetch_add(1, Ordering::Relaxed);
        } else if ms <= 500 {
            self.latency_ms_le_500.fetch_add(1, Ordering::Relaxed);
        } else if ms <= 1000 {
            self.latency_ms_le_1000.fetch_add(1, Ordering::Relaxed);
        } else if ms <= 2500 {
            self.latency_ms_le_2500.fetch_add(1, Ordering::Relaxed);
        } else if ms <= 5000 {
            self.latency_ms_le_5000.fetch_add(1, Ordering::Relaxed);
        } else {
            self.latency_ms_inf.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn render_prometheus(&self) -> String {
        let le_50 = self.latency_ms_le_50.load(Ordering::Relaxed);
        let le_100 = le_50 + self.latency_ms_le_100.load(Ordering::Relaxed);
        let le_250 = le_100 + self.latency_ms_le_250.load(Ordering::Relaxed);
        let le_500 = le_250 + self.latency_ms_le_500.load(Ordering::Relaxed);
        let le_1000 = le_500 + self.latency_ms_le_1000.load(Ordering::Relaxed);
        let le_2500 = le_1000 + self.latency_ms_le_2500.load(Ordering::Relaxed);
        let le_5000 = le_2500 + self.latency_ms_le_5000.load(Ordering::Relaxed);
        let total = le_5000 + self.latency_ms_inf.load(Ordering::Relaxed);

        format!(
            concat!(
                "# TYPE galynx_http_in_flight gauge\n",
                "galynx_http_in_flight {}\n",
                "# TYPE galynx_http_requests_total counter\n",
                "galynx_http_requests_total {}\n",
                "galynx_http_requests_total{{status_class=\"2xx\"}} {}\n",
                "galynx_http_requests_total{{status_class=\"4xx\"}} {}\n",
                "galynx_http_requests_total{{status_class=\"5xx\"}} {}\n",
                "# TYPE galynx_http_request_duration_ms histogram\n",
                "galynx_http_request_duration_ms_bucket{{le=\"50\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"100\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"250\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"500\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"1000\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"2500\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"5000\"}} {}\n",
                "galynx_http_request_duration_ms_bucket{{le=\"+Inf\"}} {}\n",
                "galynx_http_request_duration_ms_count {}\n"
            ),
            self.in_flight.load(Ordering::Relaxed),
            self.requests_total.load(Ordering::Relaxed),
            self.requests_2xx.load(Ordering::Relaxed),
            self.requests_4xx.load(Ordering::Relaxed),
            self.requests_5xx.load(Ordering::Relaxed),
            le_50,
            le_100,
            le_250,
            le_500,
            le_1000,
            le_2500,
            le_5000,
            total,
            total,
        )
    }
}

pub async fn metrics_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let started_at = std::time::Instant::now();
    state.metrics.on_request_start();
    let response = next.run(request).await;
    state
        .metrics
        .on_request_end(response.status().as_u16(), started_at.elapsed());
    response
}

#[utoipa::path(
    get,
    path = "/api/v1/metrics",
    responses(
        (status = 200, description = "Prometheus metrics", body = String)
    )
)]
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    state.metrics.render_prometheus()
}
