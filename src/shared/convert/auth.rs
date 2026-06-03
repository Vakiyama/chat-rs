use std::time::Duration;

use tonic::Status;
use uuid::Uuid;

pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/auth.v1.rs"));
}

use crate::shared::convert::auth::proto::*;
use crate::shared::convert::{IntoProto, IntoStatus, TryFromProto};
use crate::shared::domain::auth::*;

// impl TryFromProto<LoginResponse> for LoginReturn {
//   type Error = uuid::Error;
//
//   fn try_from_proto(proto: LoginReturn) -> LoginResponse {
//     LoginResponse {
//       identifier: proto.identifier.try_into().unwrap(),
//     }
//   }
// }

impl IntoProto<RegisterResponse> for RegisterReturn {
  fn into_proto(self) -> RegisterResponse {
    RegisterResponse {
      identifier: self.identifier.to_string(),
    }
  }
}

impl IntoProto<RegisterRequest> for RegisterCommand {
  fn into_proto(self) -> RegisterRequest {
    RegisterRequest {
      email: self.email,
      username: self.username,
    }
  }
}
impl IntoProto<LoginResponse> for LoginReturn {
  fn into_proto(self) -> LoginResponse {
    LoginResponse {
      identifier: self.identifier.to_string(),
    }
  }
}

impl IntoProto<LoginRequest> for LoginCommand {
  fn into_proto(self) -> LoginRequest {
    LoginRequest { email: self.email }
  }
}

impl IntoProto<RefreshRequest> for RefreshCommand {
  fn into_proto(self) -> RefreshRequest {
    RefreshRequest {
      refresh_token: self.refresh_token,
    }
  }
}

impl IntoProto<VerifyRequest> for VerifyCommand {
  fn into_proto(self) -> VerifyRequest {
    VerifyRequest {
      identifier: self.identifier.to_string(),
      email: self.email,
      code: self.code,
    }
  }
}

impl IntoProto<RefreshResponse> for RefreshReturn {
  fn into_proto(self) -> RefreshResponse {
    RefreshResponse {
      access_token: self.access_token,
      refresh_token: self.refresh_token,
    }
  }
}

impl IntoProto<VerifyResponse> for VerifyReturn {
  fn into_proto(self) -> VerifyResponse {
    VerifyResponse {
      access_token: self.access_token,
      refresh_token: self.refresh_token,
      token_duration: Some(prost_types::Duration {
        seconds: self.token_duration.as_secs().try_into().unwrap(),
        nanos: 0,
      }),
    }
  }
}

// impl TryFromProto<LoginRequest> for LoginCommand {
//   type Error = Status;
//   fn try_from_proto(req: LoginRequest) -> Result<Self, Status> {
//     if req.email.is_empty() {
//       return Err(Status::invalid_argument("email is required"));
//     }
//     Ok(LoginCommand { email: req.email })
//   }
// }

impl TryFromProto<VerifyRequest> for VerifyCommand {
  type Error = Status;
  fn try_from_proto(req: VerifyRequest) -> Result<Self, Status> {
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

impl TryFromProto<RefreshRequest> for RefreshCommand {
  type Error = Status;
  fn try_from_proto(req: RefreshRequest) -> Result<Self, Status> {
    if req.refresh_token.is_empty() {
      return Err(Status::invalid_argument("refresh_token is required"));
    }
    Ok(RefreshCommand {
      refresh_token: req.refresh_token,
    })
  }
}

impl TryFromProto<RefreshResponse> for RefreshReturn {
  type Error = Status;

  fn try_from_proto(res: RefreshResponse) -> Result<Self, Self::Error> {
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

impl TryFromProto<VerifyResponse> for VerifyReturn {
  type Error = Status;

  fn try_from_proto(res: VerifyResponse) -> Result<Self, Self::Error> {
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

impl TryFromProto<LoginResponse> for LoginReturn {
  type Error = Status;

  fn try_from_proto(res: LoginResponse) -> Result<Self, Self::Error> {
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

impl IntoStatus for VerifyError {
  fn into_status(self) -> tonic::Status {
    match self {
      VerifyError::InvalidCode => Status::unauthenticated("invalid code"),
      VerifyError::UnknownIdentifier => Status::not_found("unknown identifier"),
      VerifyError::Internal => Status::internal("internal error"),
    }
  }
}

impl IntoStatus for RefreshError {
  fn into_status(self) -> tonic::Status {
    match self {
      RefreshError::Unauthorized => Status::unauthenticated("not authorized"),
      RefreshError::UnknownIdentifier => Status::not_found("unknown identifier"),
      RefreshError::Expired => Status::unauthenticated("refresh token expired"),
      RefreshError::Internal => Status::internal("internal error"),
    }
  }
}
