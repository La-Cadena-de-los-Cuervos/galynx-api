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
    pub redis_url: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: String,
    pub s3_endpoint: Option<String>,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub s3_force_path_style: bool,
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
            redis_url: read_env("REDIS_URL"),
            s3_bucket: read_env("S3_BUCKET"),
            s3_region: read_env("S3_REGION").unwrap_or_else(|| "us-east-1".to_string()),
            s3_endpoint: read_env("S3_ENDPOINT"),
            s3_access_key_id: read_env("S3_ACCESS_KEY_ID"),
            s3_secret_access_key: read_env("S3_SECRET_ACCESS_KEY"),
            s3_force_path_style: read_env("S3_FORCE_PATH_STYLE")
                .map(|value| parse_bool(&value))
                .unwrap_or(true),
        }
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on"
    )
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
