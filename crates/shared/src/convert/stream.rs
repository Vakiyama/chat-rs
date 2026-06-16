use chrono::DateTime;
use jwt_simple::reexports::serde_json;
use prost_types::Timestamp;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/stream.v1.rs"));
}

use crate::{
  convert::{self, IntoProto, TryFromProto, TryIntoDomain},
  domain::{post::Post, stream::*},
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
        server_voice_message::Payload::Answer(SessionDescription {
          rtc_session_description: serde_json::to_string(&rtcsession_description).unwrap(),
        })
      }
    };

    ServerVoiceMessage {
      payload: Some(payload),
    }
  }
}

impl IntoProto<ClientVoiceMessage> for ClientVoice {
  fn into_proto(self) -> ClientVoiceMessage {
    let payload = match self {
      ClientVoice::Offer(rtcsession_description) => {
        client_voice_message::Payload::Offer(SessionDescription {
          rtc_session_description: serde_json::to_string(&rtcsession_description).unwrap(),
        })
      }
      ClientVoice::Answer(rtcsession_description) => {
        client_voice_message::Payload::Answer(SessionDescription {
          rtc_session_description: serde_json::to_string(&rtcsession_description).unwrap(),
        })
      }
    };

    ClientVoiceMessage {
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
      ClientText::CreatePostRequest {
        id,
        content,
        text_channel_id,
      } => ClientTextMessage {
        payload: Some(client_text_message::Payload::Post(CreatePostRequest {
          id: id.into(),
          text_channel_id: text_channel_id.to_string(),
          content,
        })),
      },
    }
  }
}

impl IntoProto<ServerTextMessage> for ServerText {
  fn into_proto(self) -> ServerTextMessage {
    match self {
      ServerText::Post(post) => {
        let seconds = post.created_at.timestamp();
        let nanos = post.created_at.timestamp_subsec_nanos() as i32;
        let created_at = Timestamp { seconds, nanos };

        ServerTextMessage {
          payload: Some(server_text_message::Payload::Chat(
            convert::stream::proto::Post {
              id: post.id.into(),
              author_name: post.author_name,
              content: post.content,
              created_at: Some(created_at),
            },
          )),
        }
      }
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
          // let Some(user) = chat_message.from else {
          //   return Err(tonic::Status::invalid_argument("Missing user"));
          // };

          let created_at = chat_message
            .created_at
            .and_then(|timestamp| {
              DateTime::from_timestamp(timestamp.seconds, timestamp.nanos.try_into().unwrap_or(0))
            })
            .ok_or(tonic::Status::invalid_argument(
              "created_at is invalid or missing.",
            ))?;

          Ok(ServerText::Post(Post {
            id: chat_message
              .id
              .try_into()
              .map_err(|_| tonic::Status::invalid_argument("failed to parse id"))?,
            author_name: chat_message.author_name,
            content: chat_message.content,
            created_at,
          }))
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
        client_text_message::Payload::Post(chat_message) => Ok(ClientText::CreatePostRequest {
          id: parse_id(chat_message.id)?,
          content: chat_message.content,
          text_channel_id: parse_id(chat_message.text_channel_id)?,
        }),
      }
    } else {
      Err(tonic::Status::invalid_argument("Missing payload"))
    }
  }
}

pub fn parse_id(id_str: String) -> Result<Uuid, tonic::Status> {
  id_str
    .try_into()
    .map_err(|_| tonic::Status::invalid_argument("failed to parse id"))
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
