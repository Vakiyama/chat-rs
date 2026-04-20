use std::{
  collections::{HashMap, HashSet},
  fmt::Debug,
  sync::{Arc, Mutex},
  time::Duration,
};

use axum::{Json, response::IntoResponse};
use bytes::Bytes;
use futures_util::{TryFutureExt, future::BoxFuture};
use http::{Request, Response, StatusCode};
use jwt_simple::{
  claims::{Claims, DEFAULT_TIME_TOLERANCE_SECS, NoCustomClaims},
  prelude::{HS256Key, MACLike},
  reexports::serde_json,
};
use serde::Deserialize;
use tower_http::auth::AsyncAuthorizeRequest;

use uuid::Uuid;

#[derive(Default, Clone)]
pub struct EmailCodePair {
  pub code: String,
  pub email: String,
}

pub type Token = String;
pub type UserId = Uuid;

#[derive(Clone)]
pub struct JWTKey {
  pub key: HS256Key,
}

impl Default for JWTKey {
  fn default() -> Self {
    let key_bytes: Bytes = hex::decode(std::env::var("JWT_KEY").expect("Missing JWT_KEY env var"))
      .expect("Invalid key, decode failed")
      .into();

    let key: HS256Key = HS256Key::from_bytes(&key_bytes);

    Self { key }
  }
}

impl JWTKey {
  pub fn get(&self) -> &HS256Key {
    &self.key
  }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct LoginResponse {
  pub identifier: Uuid,
}

impl IntoResponse for LoginResponse {
  fn into_response(self) -> axum::response::Response {
    Json(serde_json::json!(self)).into_response()
  }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct LoginBody {
  pub email: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct VerifyBody {
  pub identifier: Uuid,
  pub email: String,
  pub code: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct VerifyResponse {
  pub access_token: String,
  pub refresh_token: String,
  pub duration_milliseconds: u128,
}

impl IntoResponse for VerifyResponse {
  fn into_response(self) -> axum::response::Response {
    Json(self).into_response()
  }
}

#[derive(Debug, Deserialize, Clone)]
pub enum VerifyError {
  InvalidCode,
  UnknownIdentifier,
  Internal,
}

impl IntoResponse for VerifyError {
  fn into_response(self) -> axum::response::Response {
    match self {
      VerifyError::InvalidCode => (StatusCode::UNAUTHORIZED, "Invalid code").into_response(),
      VerifyError::UnknownIdentifier => {
        (StatusCode::BAD_REQUEST, "No such identifier").into_response()
      }
      VerifyError::Internal => {
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
      }
    }
  }
}

pub struct TokenPair {
  pub access_token: String,
  pub refresh_token: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct RefreshBody {
  pub refresh_token: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct RefreshResponse {
  pub access_token: String,
  pub refresh_token: String,
}

impl IntoResponse for RefreshResponse {
  fn into_response(self) -> axum::response::Response {
    Json(self).into_response()
  }
}

#[derive(Deserialize, Debug)]
pub enum RefreshError {
  Unauthorized,
  UnknownIdentifier,
  Expired,
  Internal,
}

impl IntoResponse for RefreshError {
  fn into_response(self) -> axum::response::Response {
    match self {
      RefreshError::Unauthorized => (StatusCode::UNAUTHORIZED, "Not authorized").into_response(),
      RefreshError::UnknownIdentifier => {
        (StatusCode::BAD_REQUEST, "No such identifier").into_response()
      }
      RefreshError::Expired => (StatusCode::UNAUTHORIZED, "Refresh token expired").into_response(),
      RefreshError::Internal => {
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
      }
    }
  }
}
