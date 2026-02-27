use rkyv::{Archive, Deserialize, Serialize};

pub mod schema;

use crate::schema::user::Model as User;

pub const WS_URL: &str = "ws://127.0.0.1:3000";
pub const SERVER_URL: &str = "127.0.0.1:3000";

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct ChatMessage {
  from: User,
  text: String,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub enum ClientMessage {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  Chat { from: User, text: String },
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub enum ServerMessage {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  Chat { from: User, text: String },
}
