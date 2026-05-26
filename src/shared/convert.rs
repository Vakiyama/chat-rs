use prost_types::Duration as ProtoDuration;
use std::time::Duration;
use tonic::Status;
use uuid::Uuid;

use crate::shared::domain::auth::*;
use proto::auth::*;

pub mod proto {
  pub mod auth {
    include!(concat!(env!("OUT_DIR"), "/auth.v1.rs"));
  }
}

// --- Into proto (domain → wire) ---

impl From<UserId> for LoginResponse {
  fn from(id: UserId) -> Self {
    LoginResponse {
      identifier: id.to_string(),
    }
  }
}

impl From<TokenPair> for VerifyResponse {
  fn from(pair: TokenPair) -> Self {
    VerifyResponse {
      access_token: pair.access_token,
      refresh_token: pair.refresh_token,
      token_duration: Some(duration_to_proto(pair.duration)),
    }
  }
}

impl From<TokenPair> for RefreshResponse {
  fn from(pair: TokenPair) -> Self {
    RefreshResponse {
      access_token: pair.access_token,
      refresh_token: pair.refresh_token,
    }
  }
}

// --- TryFrom proto (wire → domain) ---

impl TryFrom<LoginRequest> for LoginCommand {
  type Error = Status;
  fn try_from(req: LoginRequest) -> Result<Self, Status> {
    if req.email.is_empty() {
      return Err(Status::invalid_argument("email is required"));
    }
    Ok(LoginCommand { email: req.email })
  }
}

impl TryFrom<VerifyRequest> for VerifyCommand {
  type Error = Status;
  fn try_from(req: VerifyRequest) -> Result<Self, Status> {
    let identifier = Uuid::parse_str(&req.identifier)
      .map_err(|_| Status::invalid_argument("invalid identifier format"))?;
    if req.email.is_empty() || req.code.is_empty() {
      return Err(Status::invalid_argument("email and code are required"));
    }
    Ok(VerifyCommand {
      identifier,
      email: req.email,
      code: req.code,
    })
  }
}

impl TryFrom<RefreshRequest> for RefreshCommand {
  type Error = Status;
  fn try_from(req: RefreshRequest) -> Result<Self, Status> {
    if req.refresh_token.is_empty() {
      return Err(Status::invalid_argument("refresh_token is required"));
    }
    Ok(RefreshCommand {
      refresh_token: req.refresh_token,
    })
  }
}

// impl TryFrom<RefreshResponse> for Refresh

// --- Error → Status ---

impl From<VerifyError> for Status {
  fn from(e: VerifyError) -> Self {
    match e {
      VerifyError::InvalidCode => Status::unauthenticated("invalid code"),
      VerifyError::UnknownIdentifier => Status::not_found("unknown identifier"),
      VerifyError::Internal => Status::internal("internal error"),
    }
  }
}

impl From<RefreshError> for Status {
  fn from(e: RefreshError) -> Self {
    match e {
      RefreshError::Unauthorized => Status::unauthenticated("not authorized"),
      RefreshError::UnknownIdentifier => Status::not_found("unknown identifier"),
      RefreshError::Expired => Status::unauthenticated("refresh token expired"),
      RefreshError::Internal => Status::internal("internal error"),
    }
  }
}

fn duration_to_proto(d: Duration) -> ProtoDuration {
  ProtoDuration {
    seconds: d.as_secs() as i64,
    nanos: d.subsec_nanos() as i32,
  }
}
