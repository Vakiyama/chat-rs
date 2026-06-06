use crate::screens::chat;
use chat_rs::shared::convert::stream::proto::ClientMessage;
use chat_rs::shared::convert::{IntoProto, TryIntoDomain};
use chat_rs::shared::domain::stream::{Client, Server};
use futures::channel::mpsc;
use futures::stream::StreamExt;
use iced::futures;
use iced::task::{Never, Sipper, sipper};

use crate::client;

#[derive(Debug, Clone)]
pub struct Connection(mpsc::Sender<ClientMessage>);

impl Connection {
  pub fn send(&mut self, message: Client) {
    self
      .0
      .try_send(message.into_proto())
      .expect("Send message to server");
  }
}

#[derive(Debug, Clone)]
pub enum Event {
  Connected(Connection),
  Disconnected,
  MessageReceived(Server),
}

impl From<Event> for crate::Message {
  fn from(val: Event) -> Self {
    match val {
      Event::Connected(connection) => crate::Message::StreamConnected(connection),
      Event::Disconnected => crate::Message::StreamDisconnected,
      Event::MessageReceived(server_message) => {
        crate::Message::Chat(chat::Message::Stream(server_message))
      }
    }
  }
}

pub fn connect() -> impl Sipper<Never, Event> {
  sipper(async |mut output| {
    // reconnect loop; awaits 1 second on disconnect/error and retries
    loop {
      let (tx, rx) = mpsc::channel::<ClientMessage>(100);
      let mut receiver = match client::get().await.stream.connect_stream(rx).await {
        Ok(response) => {
          println!("Firing connect event.");
          output.send(Event::Connected(Connection(tx))).await;

          response.into_inner().fuse()
        }
        Err(e) => {
          tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
          continue;
        }
      };

      loop {
        match receiver.select_next_some().await {
          Ok(server_msg) => match server_msg.try_into_domain() {
            Ok(msg) => output.send(Event::MessageReceived(msg)).await,
            Err(e) => println!("Error parsing incoming bidi server msg. {e}"),
          },
          Err(_) => {
            output.send(Event::Disconnected).await;
            break;
          }
        }
      }
    }
  })
}
