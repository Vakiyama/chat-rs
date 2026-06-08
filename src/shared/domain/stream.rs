use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct User {
  pub id: Uuid,
  pub name: String,
}

pub enum ClientText {
  ChatMessage { from: User, text: String },
}

#[derive(Clone, Debug)]
pub enum ServerText {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  ChatMessage { from: User, text: String },
}
