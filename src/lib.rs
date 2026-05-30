use bytes::Bytes;
use rkyv::{Archive, Deserialize, Serialize, rancor};

pub mod schema;
pub mod shared;

use crate::schema::user::Model as User;

// todo: this is bad, setup envy config struct w/ port numbers and derive the address
// from the port where needed instead
pub const WS_PORT: i32 = 8000;
pub const WS_URL: &str = "ws://127.0.0.1:8000";
pub const SERVER_URL: &str = "127.0.0.1:3000";
// todo: this is horrid
pub const SERVER_URL_HTTP: &str = "http://127.0.0.1:3000";

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
