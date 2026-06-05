use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub enum Environment {
  Dev,
  Staging,
  Prod,
}

impl std::fmt::Display for Environment {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let s = match self {
      Environment::Dev => "DEV",
      Environment::Staging => "STAGING",
      Environment::Prod => "PROD",
    };
    f.write_str(s)
  }
}

impl std::str::FromStr for Environment {
  type Err = String;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    if s.to_uppercase() == "DEV" {
      return Ok(Environment::Dev);
    }
    if s.to_uppercase() == "STAGING" {
      return Ok(Environment::Staging);
    }
    if s.to_uppercase() == "PROD" {
      return Ok(Environment::Prod);
    }

    let err_msg =
      format!("Unknown environment {s}. Valid envs: DEV, STAGING, PROD (capital insensitive)");

    eprintln!("{err_msg}");

    Err(err_msg)
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
  pub server: ServerConfig,
  pub auth: AuthConfig,
  pub email: EmailConfig,
  pub environment: Environment,
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
  pub max_verify_code_attempts: i8,
  pub keyring_service_name: String,
  pub keyring_user: String,
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
      jwt_access_duration_secs: env("JWT_ACCESS_DURATION_SECS").unwrap_or(900),
      jwt_refresh_duration_secs: env("JWT_REFRESH_DURATION_SECS").unwrap_or(604800),
      max_verify_code_attempts: env("MAX_VERIFY_CODE_ATTEMPTS").unwrap_or(3),
      keyring_service_name: env("KEYRING_SERVICE_NAME").unwrap_or("chat-rs".into()),
      keyring_user: env("KEYRING_USER").unwrap_or("chat-rs-user".into()),
    },
    email: EmailConfig {
      resend_api_key: env("RESEND_API_KEY").expect("RESEND_API_KEY must be set"),
    },
    environment: env("ENV").expect("ENV must be set."),
  }
});

fn env<T: std::str::FromStr>(key: &str) -> Option<T> {
  std::env::var(key).ok().and_then(|v| v.parse().ok())
}
