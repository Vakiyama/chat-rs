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

impl IntoProto<ClientMessage> for Client {
  fn into_proto(self) -> ClientMessage {
    match self {
      Client::ChatMessage { from, text } => ClientMessage {
        payload: Some(client_message::Payload::Chat(ChatMessage {
          from: Some(from.into_proto()),
          text,
        })),
      },
    }
  }
}

impl IntoProto<ServerMessage> for Server {
  fn into_proto(self) -> ServerMessage {
    match self {
      Server::ChatMessage { from, text } => ServerMessage {
        payload: Some(server_message::Payload::Chat(ChatMessage {
          from: Some(from.into_proto()),
          text,
        })),
      },
      Server::JoinedRoom { from } => ServerMessage {
        payload: Some(server_message::Payload::JoinedRoom(JoinedRoom {
          from: Some(from.into_proto()),
        })),
      },
      Server::LeftRoom { from } => ServerMessage {
        payload: Some(server_message::Payload::LeftRoom(LeftRoom {
          from: Some(from.into_proto()),
        })),
      },
    }
  }
}

// try from proto

impl TryFromProto<ServerMessage> for Server {
  type Error = Status;

  fn try_from_proto(proto: ServerMessage) -> Result<Self, Self::Error> {
    if let Some(payload) = proto.payload {
      match payload {
        server_message::Payload::JoinedRoom(joined_room) => {
          let Some(user) = joined_room.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(Server::JoinedRoom {
            from: user.try_into_domain()?,
          })
        }
        server_message::Payload::LeftRoom(left_room) => {
          let Some(user) = left_room.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(Server::LeftRoom {
            from: user.try_into_domain()?,
          })
        }
        server_message::Payload::Chat(chat_message) => {
          let Some(user) = chat_message.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(Server::ChatMessage {
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

impl TryFromProto<ClientMessage> for Client {
  type Error = Status;

  fn try_from_proto(proto: ClientMessage) -> Result<Self, Self::Error> {
    if let Some(payload) = proto.payload {
      match payload {
        client_message::Payload::Chat(chat_message) => {
          let Some(user) = chat_message.from else {
            return Err(tonic::Status::invalid_argument("Missing user"));
          };

          Ok(Client::ChatMessage {
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
