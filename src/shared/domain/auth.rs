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

#[derive(Clone, Debug)]
pub struct LoginReturn {
  pub identifier: Uuid,
}

pub struct VerifyCommand {
  pub identifier: UserId,
  pub email: String,
  pub code: String,
}

#[derive(Clone, Debug)]
pub struct VerifyReturn {
  pub access_token: String,
  pub refresh_token: String,
  pub token_duration: Duration,
}

pub struct RefreshCommand {
  pub refresh_token: Token,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct RefreshReturn {
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
