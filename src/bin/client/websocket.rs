use chat_rs::{WS_URL, WebSocketMessage};
use iced::futures;
use iced::task::{Never, Sipper, sipper};

use async_tungstenite::tungstenite;
use bytes::Bytes;
use futures::channel::mpsc;
use futures::sink::SinkExt;
use futures::stream::StreamExt;

use crate::Message;

#[derive(Debug, Clone)]
pub struct Connection(mpsc::Sender<WebSocketMessage>);

impl Connection {
  pub fn send(&mut self, message: WebSocketMessage) {
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
  MessageReceived(WebSocketMessage),
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
      // headers setup - https://docs.rs/async-tungstenite/latest/async_tungstenite/tokio/fn.connect_async.html
      // let mut request = "wss://api.example.com".into_client_request().unwrap();
      // request.headers_mut().insert("api-key", "42".parse().unwrap());
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
                        println!("received message, attempting deserialize");
                        let deserialized: Result<WebSocketMessage, _> = message.try_into();

                        if let Ok(server_msg) = deserialized {
                         println!("Deserialized {server_msg:?}");
                         output.send(Event::MessageReceived(server_msg)).await
                        } else {
                            println!("Serialization error: {deserialized:?}")
                        }
                    }
                    Ok(tungstenite::Message::Close(_)) | Err(_) => {
                        output.send(Event::Disconnected).await;
                        // breaks on error to restart the reconnect loop
                        break;
                    }
                    Ok(other) => { println!("Received other: {other}")}
                }
            }
            message = input.select_next_some() => {
                println!("firing msg: {message:?}");
                let serialized: Result<Bytes, _> = message.try_into();

                if let Ok(bytes) = serialized {
                    let result = websocket.send(tungstenite::Message::Binary(bytes)).await;

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
