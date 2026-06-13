use chat_shared::config::{Environment, env};
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct Config {
  pub server_url: String,
  pub keyring_service_name: String,
  pub keyring_user: String,
  pub environment: Environment,
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
  dotenvy::dotenv().ok();
  let environment: Environment = env("ENV").unwrap_or(Environment::Dev);
  Config {
    server_url: env("SERVER_URL").unwrap_or_else(|| {
      option_env!("DEFAULT_SERVER_URL") // baked at compile time, if set
        .unwrap_or("http://127.0.0.1:3000") // dev fallback
        .to_string()
    }),
    keyring_service_name: env("KEYRING_SERVICE_NAME").unwrap_or_else(|| "chat-rs".into()),
    keyring_user: env("KEYRING_USER").unwrap_or_else(|| "chat-rs-user".into()),
    environment,
  }
});
