use iced::keyboard::Event;

use crate::websocket::Connection;

#[derive(Debug, Clone)]
pub enum Message {
  ContentChanged(String),
  Keyboard(Event),
  Disconnected,
  Connected(Connection),
  Websocket(chat_rs::ServerMessage),
}

impl Message {
  pub fn as_str(&self) -> &str {
    match self {
      Message::ContentChanged(_) => todo!(),
      Message::Keyboard(_) => todo!(),
      Message::Disconnected => todo!(),
      Message::Connected(_) => todo!(),
      Message::Websocket(_) => todo!(),
    }
  }
}
