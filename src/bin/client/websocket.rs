use rkyv::rancor::Error;

use chat_rs::{ClientMessage, ServerMessage, WS_URL};
use iced::futures;
use iced::task::{Never, Sipper, sipper};

use async_tungstenite::tungstenite;
use futures::channel::mpsc;
use futures::sink::SinkExt;
use futures::stream::StreamExt;

use crate::Message;

#[derive(Debug, Clone)]
pub struct Connection(mpsc::Sender<ClientMessage>);

impl Connection {
  pub fn send(&mut self, message: ClientMessage) {
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
  MessageReceived(ServerMessage),
}

impl From<Event> for Message {
  fn from(val: Event) -> Self {
    match val {
      Event::Connected(connection) => Message::Connected(connection),
      Event::Disconnected => Message::Disconnected,
      Event::MessageReceived(server_message) => Message::Websocket(server_message),
    }
  }
}

pub fn connect() -> impl Sipper<Never, Event> {
  sipper(async |mut output| {
    // reconnect loop; awaits 1 second on disconnect/error and retries
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

      loop {
        futures::select! {
            received = websocket.select_next_some() => {
                match received {
                    Ok(tungstenite::Message::Binary(message)) => {
                        let deserialized = rkyv::access::<chat_rs::ArchivedServerMessage, Error>(&message)
                            .and_then(rkyv::deserialize);

                        if let Ok(server_msg) = deserialized {
                         output.send(Event::MessageReceived(server_msg)).await
                        } else {
                            println!("Serliaziation error: {deserialized:?}")
                        }
                    }
                    Err(_) => {
                        output.send(Event::Disconnected).await;
                        // breaks on error to restart the reconnect loop
                        break;
                    }
                    Ok(other) => { println!("Received other: {other}")}
                }
            }
            message = input.select_next_some() => {
                let serialized = rkyv::to_bytes::<Error>(&message);

                if let Ok(client_msg) = serialized {
                    let result = websocket.send(tungstenite::Message::Binary(client_msg.into_vec().into())).await;

                    if result.is_err() {
                      output.send(Event::Disconnected).await;
                    }
                } else {
                    println!("Serialization Error: {serialized:?}")
                }
            }
        }
      }
    }
  })
}
