use chat_rs::WS_URL;
use iced::futures;
use iced::task::{Never, Sipper, sipper};

use async_tungstenite::tungstenite;
use futures::channel::mpsc;
use futures::sink::SinkExt;
use futures::stream::StreamExt;

use crate::Message;

#[derive(Debug, Clone)]
pub struct Connection(mpsc::Sender<Message>);

impl Connection {
  pub fn send(&mut self, message: Message) {
    self
      .0
      .try_send(message)
      .expect("Send message to echo server");
  }
}

#[derive(Debug, Clone)]
pub enum Event {
  Connected(Connection),
  Disconnected,
  MessageReceived(Message),
}

pub fn connect() -> impl Sipper<Never, Event> {
  sipper(async |mut output| {
    loop {
      let (mut websocket, mut input) = match async_tungstenite::tokio::connect_async(WS_URL).await {
        Ok((websocket, _)) => {
          let (sender, receiver) = mpsc::channel(100);

          output.send(Event::Connected(Connection(sender))).await;

          (websocket.fuse(), receiver)
        }
        Err(_) => {
          tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

          continue;
        }
      };
    }
  })
}
