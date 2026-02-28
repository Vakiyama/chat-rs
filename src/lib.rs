use bytes::Bytes;
use rkyv::{Archive, Deserialize, Serialize, rancor};

pub mod schema;

use crate::schema::user::Model as User;

pub const WS_URL: &str = "ws://127.0.0.1:3000";
pub const SERVER_URL: &str = "127.0.0.1:3000";

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct ChatMessage {
  from: User,
  text: String,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize, PartialEq)]
pub enum WebSocketMessage {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  Chat { from: User, text: String },
}

impl TryFrom<Bytes> for WebSocketMessage {
  type Error = rancor::Error;

  fn try_from(value: Bytes) -> Result<Self, Self::Error> {
    let mut aligned = rkyv::util::AlignedVec::<16>::new();
    aligned.extend_from_slice(&value);

    let archived = rkyv::access::<ArchivedWebSocketMessage, Self::Error>(&aligned)?;

    rkyv::deserialize(archived)
  }
}

impl TryFrom<WebSocketMessage> for Bytes {
  type Error = rancor::Error;

  fn try_from(value: WebSocketMessage) -> Result<Self, Self::Error> {
    let bytes: Bytes = rkyv::to_bytes::<Self::Error>(&value)?.into_vec().into();

    Ok(bytes)
  }
}

impl TryFrom<&WebSocketMessage> for Bytes {
  type Error = rancor::Error;

  fn try_from(value: &WebSocketMessage) -> Result<Self, Self::Error> {
    let bytes: Bytes = rkyv::to_bytes::<Self::Error>(value)?.into_vec().into();

    Ok(bytes)
  }
}
