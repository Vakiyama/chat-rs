use std::time::Duration;
use uuid::Uuid;

pub type Token = String;
pub type UserId = Uuid;

pub struct TokenPair {
  pub access_token: Token,
  pub refresh_token: Token,
  pub duration: Duration,
}

pub struct LoginCommand {
  pub email: String,
}

pub struct VerifyCommand {
  pub identifier: UserId,
  pub email: String,
  pub code: String,
}

pub struct RefreshCommand {
  pub refresh_token: Token,
}

#[derive(serde::Deserialize)]
pub struct RefreshBody {
  pub access_token: String,
  pub refresh_token: String,
}
#[derive(Debug)]
pub enum VerifyError {
  InvalidCode,
  UnknownIdentifier,
  Internal,
}

#[derive(Debug)]
pub enum RefreshError {
  Unauthorized,
  UnknownIdentifier,
  Expired,
  Internal,
}
