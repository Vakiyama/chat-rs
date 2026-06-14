pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/server.v1.rs"));
}

use crate::convert::IntoProto;
use crate::domain::server::*;
use proto::Channel as ChannelProto;
use proto::ChannelType as ChannelTypeProto;
use proto::Server as ServerProto;
use proto::ServersResponse as ServersResponseProto;

impl IntoProto<ServersResponseProto> for ServersResponse {
  fn into_proto(self) -> ServersResponseProto {
    ServersResponseProto {
      servers: self
        .servers
        .into_iter()
        .map(|s| s.into_proto())
        .collect(),
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
      r#type: self.r#type.into_proto() as i32,
    }
  }
}

impl IntoProto<i32> for ChannelType {
  fn into_proto(self) -> i32 {
    match self {
      ChannelType::Unspecified => ChannelTypeProto::MethodTypeUnspecified as i32,
      ChannelType::Text => ChannelTypeProto::Text as i32,
      ChannelType::Voice => ChannelTypeProto::Voice as i32,
    }
  }
}
