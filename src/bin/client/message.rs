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
