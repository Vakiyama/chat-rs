use uuid::Uuid;

pub struct User {
  pub id: Uuid,
  pub name: String,
}

pub enum Client {
  ChatMessage { from: User, text: String },
}

pub enum Server {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  ChatMessage { from: User, text: String },
}
