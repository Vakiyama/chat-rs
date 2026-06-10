use jwt_simple::reexports::serde_json;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

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

impl IntoProto<ServerVoiceMessage> for ServerVoice {
  fn into_proto(self) -> ServerVoiceMessage {
    let payload = match self {
      ServerVoice::Offer(rtcsession_description) => {
        server_voice_message::Payload::Offer(SessionDescription {
          rtc_session_description: serde_json::to_string(&rtcsession_description).unwrap(),
        })
      }
      ServerVoice::Answer(rtcsession_description) => {
        server_voice_message::Payload::Offer(SessionDescription {
          rtc_session_description: serde_json::to_string(&rtcsession_description).unwrap(),
        })
      }
    };

    ServerVoiceMessage {
      payload: Some(payload),
    }
  }
}

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

impl TryFromProto<ClientVoiceMessage> for ClientVoice {
  type Error = Status;

  fn try_from_proto(proto: ClientVoiceMessage) -> Result<Self, Self::Error> {
    if let Some(payload) = proto.payload {
      match payload {
        client_voice_message::Payload::Offer(offer) => {
          let session_desc =
            serde_json::from_str::<RTCSessionDescription>(&offer.rtc_session_description)
              .map_err(|_e| tonic::Status::invalid_argument("Invalid session descripion"))?;

          Ok(ClientVoice::Offer(session_desc))
        }
        client_voice_message::Payload::Answer(offer) => {
          let session_desc =
            serde_json::from_str::<RTCSessionDescription>(&offer.rtc_session_description)
              .map_err(|_e| tonic::Status::invalid_argument("Invalid session descripion"))?;

          Ok(ClientVoice::Answer(session_desc))
        }
      }
    } else {
      Err(tonic::Status::invalid_argument("Missing payload"))
    }
  }
}

// 1. the trait bound `ServerVoice: TryFromProto<ServerVoiceMessage>` is not satisfied

impl TryFromProto<ServerVoiceMessage> for ServerVoice {
  type Error = Status;

  fn try_from_proto(proto: ServerVoiceMessage) -> Result<Self, Self::Error> {
    if let Some(payload) = proto.payload {
      match payload {
        server_voice_message::Payload::Offer(session_description) => Ok(ServerVoice::Offer({
          serde_json::from_str(&session_description.rtc_session_description)
            .map_err(|_e| tonic::Status::invalid_argument("invalid rtc_session_description."))?
        })),
        server_voice_message::Payload::Answer(session_description) => Ok(ServerVoice::Answer({
          serde_json::from_str(&session_description.rtc_session_description)
            .map_err(|_e| tonic::Status::invalid_argument("invalid rtc_session_description."))?
        })),
      }
    } else {
      Err(tonic::Status::invalid_argument("Missing payload"))
    }
  }
}

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
