use crate::screens::chat;

#[derive(Debug, Clone)]
pub enum Message {
  Chat(chat::Message),
}
