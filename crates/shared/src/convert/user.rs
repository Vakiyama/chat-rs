pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/user.v1.rs"));
}

use crate::{
  convert::{IntoProto, TryFromProto},
  domain::user::*,
};
use proto::*;
use tonic::Status;

impl TryFromProto<MeResponse> for MeReturn {
  type Error = tonic::Status;

  fn try_from_proto(proto: MeResponse) -> Result<Self, Self::Error> {
    let Ok(identifier) = proto.identifier.try_into() else {
      return Err(Status::invalid_argument("Invalid identifier"));
    };

    if proto.username.is_empty() {
      return Err(Status::invalid_argument(
        "response doesn't include username",
      ));
    };

    Ok(MeReturn {
      username: proto.username,
      user_id: identifier,
    })
  }
}

impl IntoProto<MeResponse> for MeReturn {
  fn into_proto(self) -> MeResponse {
    MeResponse {
      username: self.username,
      identifier: self.user_id.into(),
    }
  }
}
