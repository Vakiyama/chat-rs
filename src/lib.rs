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

#[derive(Debug, Clone)]
pub struct ChatMessage {
  from: User,
  text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebSocketMessage {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  Chat { from: User, text: String },
}
