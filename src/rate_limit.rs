use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::{HeaderMap, header};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::errors::{ApiError, ApiResult};

#[derive(Clone)]
pub struct RateLimitService {
    auth_limiter: Arc<RwLock<FixedWindowLimiter>>,
    ws_connect_limiter: Arc<RwLock<FixedWindowLimiter>>,
    ws_command_limiter: Arc<RwLock<FixedWindowLimiter>>,
}

#[derive(Debug)]
struct FixedWindowLimiter {
    max_requests: u32,
    window: Duration,
    buckets: HashMap<String, WindowBucket>,
}

#[derive(Debug)]
struct WindowBucket {
    count: u32,
    reset_at: Instant,
}

impl RateLimitService {
    pub fn new() -> Self {
        Self {
            auth_limiter: Arc::new(RwLock::new(FixedWindowLimiter::new(
                30,
                Duration::from_secs(60),
            ))),
            ws_connect_limiter: Arc::new(RwLock::new(FixedWindowLimiter::new(
                12,
                Duration::from_secs(60),
            ))),
            ws_command_limiter: Arc::new(RwLock::new(FixedWindowLimiter::new(
                600,
                Duration::from_secs(60),
            ))),
        }
    }

    pub async fn check_auth(&self, client_ip: &str, email: Option<&str>) -> ApiResult<()> {
        let key = format!(
            "ip={}|email={}",
            normalize_key(client_ip),
            email.map(normalize_key).unwrap_or_else(|| "-".to_string())
        );
        self.auth_limiter
            .write()
            .await
            .check(&key, "too many auth requests, retry in a minute")
    }

    pub async fn check_ws_connect(&self, client_ip: &str, user_id: Uuid) -> ApiResult<()> {
        let key = format!("ip={}|user={}", normalize_key(client_ip), user_id);
        self.ws_connect_limiter
            .write()
            .await
            .check(&key, "too many websocket connection attempts")
    }

    pub async fn check_ws_command(&self, user_id: Uuid) -> ApiResult<()> {
        let key = format!("user={}", user_id);
        self.ws_command_limiter
            .write()
            .await
            .check(&key, "too many websocket commands, slow down")
    }
}

impl FixedWindowLimiter {
    fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            buckets: HashMap::new(),
        }
    }

    fn check(&mut self, key: &str, message: &str) -> ApiResult<()> {
        let now = Instant::now();
        let bucket = self.buckets.entry(key.to_string()).or_insert(WindowBucket {
            count: 0,
            reset_at: now + self.window,
        });

        if now >= bucket.reset_at {
            bucket.count = 0;
            bucket.reset_at = now + self.window;
        }

        if bucket.count >= self.max_requests {
            return Err(ApiError::TooManyRequests(message.to_string()));
        }

        bucket.count += 1;
        Ok(())
    }
}

pub fn client_ip_from_headers(headers: &HeaderMap) -> String {
    if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = value.split(',').next().unwrap_or_default().trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    if let Some(value) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = value.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    if let Some(value) = headers
        .get(header::FORWARDED)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_forwarded_for)
    {
        return value;
    }
    "unknown".to_string()
}

fn parse_forwarded_for(value: &str) -> Option<String> {
    for segment in value.split(';') {
        let segment = segment.trim();
        if let Some(for_value) = segment.strip_prefix("for=") {
            let ip = for_value.trim_matches('"').trim();
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }
    None
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fixed_window_limiter_blocks_after_limit() {
        let mut limiter = FixedWindowLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("key", "limit").is_ok());
        assert!(limiter.check("key", "limit").is_ok());
        let result = limiter.check("key", "limit");
        assert!(matches!(result, Err(ApiError::TooManyRequests(_))));
    }
}
