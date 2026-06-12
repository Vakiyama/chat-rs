#[derive(Debug, Clone, PartialEq)]
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
    match s.to_uppercase().as_str() {
      "DEV" => Ok(Environment::Dev),
      "STAGING" => Ok(Environment::Staging),
      "PROD" => Ok(Environment::Prod),
      _ => Err(format!(
        "Unknown environment {s}. Valid envs: DEV, STAGING, PROD (capital insensitive)"
      )),
    }
  }
}

pub fn env<T: std::str::FromStr>(key: &str) -> Option<T> {
  std::env::var(key).ok().and_then(|v| v.parse().ok())
}
