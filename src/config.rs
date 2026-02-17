use crate::storage::PersistenceBackend;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub jwt_secret: String,
    pub access_ttl_minutes: i64,
    pub refresh_ttl_days: i64,
    pub bootstrap_email: String,
    pub bootstrap_password: String,
    pub persistence_backend: PersistenceBackend,
    pub mongo_uri: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: read_env("PORT")
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(3000),
            jwt_secret: read_env("JWT_SECRET")
                .unwrap_or_else(|| "dev-only-change-me-in-prod".to_string()),
            access_ttl_minutes: read_env("ACCESS_TTL_MINUTES")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(15),
            refresh_ttl_days: read_env("REFRESH_TTL_DAYS")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(30),
            bootstrap_email: read_env("BOOTSTRAP_EMAIL")
                .unwrap_or_else(|| "owner@galynx.local".to_string()),
            bootstrap_password: read_env("BOOTSTRAP_PASSWORD")
                .unwrap_or_else(|| "ChangeMe123!".to_string()),
            persistence_backend: read_env("PERSISTENCE_BACKEND")
                .as_deref()
                .map(PersistenceBackend::from_env_value)
                .unwrap_or(PersistenceBackend::Memory),
            mongo_uri: read_env("MONGO_URI"),
        }
    }
}

fn read_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl PersistenceBackend {
    fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "mongo" | "mongodb" | "documentdb" => Self::Mongo,
            _ => Self::Memory,
        }
    }
}
