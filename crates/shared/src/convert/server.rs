pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/server.v1.rs"));
}

use crate::convert::IntoProto;
use crate::convert::TryFromProto;
use crate::convert::TryIntoDomain;
use crate::convert::stream::parse_id;
use crate::domain::server::*;
use proto::Channel as ChannelProto;
use proto::ChannelType as ChannelTypeProto;
use proto::Server as ServerProto;
use proto::ServersResponse as ServersResponseProto;
use proto::SetChannelMuteRequest as SetChannelMuteRequestProto;

impl IntoProto<ServersResponseProto> for ServersResponse {
  fn into_proto(self) -> ServersResponseProto {
    ServersResponseProto {
      servers: self.servers.into_iter().map(|s| s.into_proto()).collect(),
    }
  }
}

impl IntoProto<ServerProto> for Server {
  fn into_proto(self) -> ServerProto {
    ServerProto {
      id: self.id.to_string(),
      name: self.name,
      channels: self.channels.into_iter().map(|c| c.into_proto()).collect(),
    }
  }
}

impl IntoProto<ChannelProto> for Channel {
  fn into_proto(self) -> ChannelProto {
    ChannelProto {
      id: self.id.to_string(),
      name: self.name,
      r#type: self.r#type.into_proto(),
      muted: self.muted,
    }
  }
}

impl IntoProto<i32> for ChannelType {
  fn into_proto(self) -> i32 {
    match self {
      ChannelType::Text => ChannelTypeProto::Text as i32,
      ChannelType::Voice => ChannelTypeProto::Voice as i32,
    }
  }
}

impl TryFromProto<ServersResponseProto> for ServersResponse {
  type Error = tonic::Status;

  fn try_from_proto(proto: ServersResponseProto) -> Result<Self, Self::Error> {
    Ok(Self {
      servers: proto
        .servers
        .into_iter()
        .map(|server_proto| server_proto.try_into_domain())
        .collect::<Result<Vec<Server>, tonic::Status>>()?,
    })
  }
}

impl TryFromProto<ServerProto> for Server {
  type Error = tonic::Status;

  fn try_from_proto(proto: ServerProto) -> Result<Self, Self::Error> {
    Ok(Self {
      id: parse_id(proto.id)?,
      name: proto.name,
      channels: proto
        .channels
        .into_iter()
        .map(|channel_proto| channel_proto.try_into_domain())
        .collect::<Result<Vec<Channel>, tonic::Status>>()?,
    })
  }
}

impl TryFromProto<ChannelProto> for Channel {
  type Error = tonic::Status;

  fn try_from_proto(proto: ChannelProto) -> Result<Self, Self::Error> {
    Ok(Self {
      id: parse_id(proto.id)?,
      name: proto.name,
      r#type: match proto.r#type {
        1 => Ok(ChannelType::Text),
        2 => Ok(ChannelType::Voice),
        _ => Err(tonic::Status::invalid_argument("Invalid channel type")),
      }?,
      muted: proto.muted,
    })
  }
}

impl IntoProto<SetChannelMuteRequestProto> for SetChannelMuteRequest {
  fn into_proto(self) -> SetChannelMuteRequestProto {
    SetChannelMuteRequestProto {
      text_channel_id: self.text_channel_id.to_string(),
      muted: self.muted,
    }
  }
}

impl TryFromProto<SetChannelMuteRequestProto> for SetChannelMuteRequest {
  type Error = tonic::Status;

  fn try_from_proto(proto: SetChannelMuteRequestProto) -> Result<Self, Self::Error> {
    Ok(Self {
      text_channel_id: parse_id(proto.text_channel_id)?,
      muted: proto.muted,
    })
  }
}

// enum ChannelType {
//   METHOD_TYPE_UNSPECIFIED = 0;
//   TEXT = 1;
//   VOICE = 2;
// }
