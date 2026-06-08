pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/stream.v1.rs"));
}

use crate::shared::{
  convert::{IntoProto, TryFromProto, TryIntoDomain},
  domain::stream::*,
};
use proto::*;
use tonic::Status;
use uuid::Uuid;

// into proto

impl IntoProto<DisplayUser> for User {
  fn into_proto(self) -> DisplayUser {
    DisplayUser {
      id: self.id.to_string(),
      name: self.name,
    }
  }
}

impl IntoProto<ClientTextMessage> for ClientText {
  fn into_proto(self) -> ClientTextMessage {
    match self {
      ClientText::ChatMessage { from, text } => ClientTextMessage {
        payload: Some(client_text_message::Payload::Chat(ChatMessage {
          from: Some(from.into_proto()),
          text,
        })),
      },
    }
  }
}

impl IntoProto<ServerTextMessage> for ServerText {
  fn into_proto(self) -> ServerTextMessage {
    match self {
      ServerText::ChatMessage { from, text } => ServerTextMessage {
        payload: Some(server_text_message::Payload::Chat(ChatMessage {
          from: Some(from.into_proto()),
          text,
        })),
      },
      ServerText::JoinedRoom { from } => ServerTextMessage {
        payload: Some(server_text_message::Payload::JoinedRoom(JoinedRoom {
          from: Some(from.into_proto()),
        })),
      },
      ServerText::LeftRoom { from } => ServerTextMessage {
        payload: Some(server_text_message::Payload::LeftRoom(LeftRoom {
          from: Some(from.into_proto()),
        })),
      },
    }
  }
}

// try from proto

impl TryFromProto<ServerTextMessage> for ServerText {
  type Error = Status;

  fn try_from_proto(proto: ServerTextMessage) -> Result<Self, Self::Error> {
    if let Some(payload) = proto.payload {
      match payload {
        server_text_message::Payload::JoinedRoom(joined_room) => {
          let Some(user) = joined_room.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(ServerText::JoinedRoom {
            from: user.try_into_domain()?,
          })
        }
        server_text_message::Payload::LeftRoom(left_room) => {
          let Some(user) = left_room.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(ServerText::LeftRoom {
            from: user.try_into_domain()?,
          })
        }
        server_text_message::Payload::Chat(chat_message) => {
          let Some(user) = chat_message.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(ServerText::ChatMessage {
            from: user.try_into_domain()?,
            text: chat_message.text,
          })
        }
      }
    } else {
      Err(tonic::Status::invalid_argument("Missing payload"))
    }
  }
}

impl TryFromProto<ClientTextMessage> for ClientText {
  type Error = Status;

  fn try_from_proto(proto: ClientTextMessage) -> Result<Self, Self::Error> {
    if let Some(payload) = proto.payload {
      match payload {
        client_text_message::Payload::Chat(chat_message) => {
          let Some(user) = chat_message.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(ClientText::ChatMessage {
            from: user.try_into_domain()?,
            text: chat_message.text,
          })
        }
      }
    } else {
      Err(tonic::Status::invalid_argument("Missing payload"))
    }
  }
}

impl TryFromProto<DisplayUser> for User {
  type Error = Status;

  fn try_from_proto(proto: DisplayUser) -> Result<Self, Self::Error> {
    let identifier = Uuid::parse_str(&proto.id)
      .map_err(|_| Status::invalid_argument("invalid identifier format"))?;

    Ok(Self {
      id: identifier,
      name: proto.name,
    })
  }
}
