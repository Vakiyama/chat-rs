use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
  pub server: ServerConfig,
  pub auth: AuthConfig,
  pub email: EmailConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
  pub grpc_address: String,
  pub db_connection: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
  pub jwt_key_hex: String,
  pub jwt_access_duration_secs: u64,
  pub jwt_refresh_duration_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmailConfig {
  pub resend_api_key: String,
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
  dotenvy::dotenv().ok();

  Config {
    server: ServerConfig {
      grpc_address: env("SERVER_GRPC_ADDRESS").unwrap_or_else(|| "127.0.0.1:3000".into()),
      db_connection: env("DB_CONNECTION")
        .unwrap_or_else(|| "postgres://postgres@localhost:5432/local".into()),
    },
    auth: AuthConfig {
      jwt_key_hex: env("JWT_KEY").expect("JWT_KEY must be set"),
      jwt_access_duration_secs: env("JWT_ACCESS_DURATION_SECS").unwrap_or(28800),
      jwt_refresh_duration_secs: env("JWT_REFRESH_DURATION_SECS").unwrap_or(2592000),
    },
    email: EmailConfig {
      resend_api_key: env("RESEND_API_KEY").expect("RESEND_API_KEY must be set"),
    },
  }
});

fn env<T: std::str::FromStr>(key: &str) -> Option<T> {
  std::env::var(key).ok().and_then(|v| v.parse().ok())
}
