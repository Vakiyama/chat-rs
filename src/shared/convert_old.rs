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

impl From<LoginReturn> for LoginResponse {
  fn from(result: LoginReturn) -> Self {
    LoginResponse {
      identifier: result.identifier.to_string(),
    }
  }
}

impl From<VerifyCommand> for VerifyRequest {
  fn from(command: VerifyCommand) -> Self {
    VerifyRequest {
      identifier: command.identifier.to_string(),
      email: command.email,
      code: command.code,
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

impl TryFrom<RefreshResponse> for RefreshReturn {
  type Error = Status;

  fn try_from(res: RefreshResponse) -> Result<Self, Self::Error> {
    if res.refresh_token.is_empty() {
      return Err(Status::invalid_argument(
        "response doesn't include refresh token",
      ));
    }

    if res.access_token.is_empty() {
      return Err(Status::invalid_argument(
        "response doesn't include access_token token",
      ));
    }

    Ok(RefreshReturn {
      access_token: res.access_token,
      refresh_token: res.refresh_token,
    })
  }
}

impl TryFrom<VerifyResponse> for VerifyReturn {
  type Error = Status;

  fn try_from(res: VerifyResponse) -> Result<Self, Self::Error> {
    if res.refresh_token.is_empty() {
      return Err(Status::invalid_argument(
        "response doesn't include refresh token",
      ));
    }

    if res.access_token.is_empty() {
      return Err(Status::invalid_argument(
        "response doesn't include access_token token",
      ));
    }

    let Some(token_duration) = res.token_duration else {
      return Err(Status::invalid_argument("duration is empty"));
    };
    let seconds: u64 = token_duration.seconds.try_into().map_err(|_| {
      Status::invalid_argument("Incoming duration failed to parse into u64 from i64")
    })?;

    let nanos: u32 = token_duration.nanos.try_into().map_err(|_| {
      Status::invalid_argument("Incoming duration failed to parse into u32 from i32")
    })?;

    Ok(VerifyReturn {
      access_token: res.access_token,
      refresh_token: res.refresh_token,
      token_duration: Duration::new(seconds, nanos),
    })
  }
}

impl TryFrom<LoginResponse> for LoginReturn {
  type Error = Status;

  fn try_from(res: LoginResponse) -> Result<Self, Self::Error> {
    if res.identifier.is_empty() {
      return Err(Status::invalid_argument(
        "response doesn't include identifier",
      ));
    }

    let Ok(identifier) = res.identifier.try_into() else {
      return Err(Status::invalid_argument("Invalid identifier"));
    };

    Ok(LoginReturn { identifier })
  }
}

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
