use crate::screens::chat;
use chat_shared::convert::stream::proto::ClientTextMessage;
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::stream::{ClientText, ServerText};
use futures::channel::mpsc;
use futures::stream::StreamExt;
use iced::futures;
use iced::task::{Never, Sipper, sipper};
use std::time::{Duration, SystemTime};

use chat_core::client;

#[derive(Debug, Clone)]
pub struct ChatConnection {
  sender: mpsc::Sender<ClientTextMessage>,
}

impl ChatConnection {
  pub fn send(&mut self, message: ClientText) {
    self
      .sender
      .try_send(message.into_proto())
      .expect("Send message to server");
  }

  pub fn try_send(
    &mut self,
    message: ClientText,
  ) -> Result<(), mpsc::TrySendError<ClientTextMessage>> {
    self.sender.try_send(message.into_proto())
  }
}

#[derive(Debug, Clone)]
pub enum Event {
  Connected(ChatConnection),
  Disconnected,
  MessageReceived(ServerText),
  LatencyUpdated(u64),
}

impl From<Event> for crate::Message {
  fn from(val: Event) -> Self {
    match val {
      Event::Connected(connection) => crate::Message::ChatStreamConnected(connection),
      Event::Disconnected => crate::Message::ChatStreamDisconnected,
      Event::MessageReceived(server_message) => {
        crate::Message::Chat(chat::Message::Stream(server_message))
      }
      Event::LatencyUpdated(latency_ms) => crate::Message::ChatLatencyUpdated(latency_ms as u32),
    }
  }
}

const MAX_BACKOFF: Duration = Duration::from_secs(30);
const PING_INTERVAL: Duration = Duration::from_secs(2);

fn backoff() -> impl Iterator<Item = Duration> {
  use tokio_retry::strategy::{ExponentialBackoff, jitter};
  ExponentialBackoff::from_millis(2)
    .factor(500)
    .max_delay(MAX_BACKOFF)
    .map(jitter)
}

pub fn connect() -> impl Sipper<Never, Event> {
  sipper(async |mut output| {
    let mut delays = backoff();

    loop {
      let (tx, rx) = mpsc::channel::<ClientTextMessage>(100);
      let mut ping_tx = tx.clone();
      let connection = ChatConnection { sender: tx };

      let mut receiver = match client::get().await.stream.connect_text_stream(rx).await {
        Ok(response) => {
          delays = backoff();
          println!("Firing chat stream connect event.");
          output.send(Event::Connected(connection)).await;

          response.into_inner().fuse()
        }
        Err(_) => {
          tokio::time::sleep(delays.next().unwrap_or(MAX_BACKOFF)).await;
          continue;
        }
      };

      let mut ping_interval = tokio::time::interval(PING_INTERVAL);
      ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

      loop {
        tokio::select! {
          server_msg = receiver.next() => {
            match server_msg {
              Some(Ok(msg)) => match msg.try_into_domain() {
                Ok(ServerText::Pong { timestamp, .. }) => {
                  let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64;
                  let latency_ms = now.saturating_sub(timestamp) / 1000;
                  if latency_ms > 0 {
                    let _ = output.send(Event::LatencyUpdated(latency_ms)).await;
                  }
                }
                Ok(msg) => {
                  let _ = output.send(Event::MessageReceived(msg)).await;
                }
                Err(e) => println!("Error parsing incoming bidi server msg. {e}"),
              },
              Some(Err(_)) => {
                output.send(Event::Disconnected).await;
                break;
              }
              None => {
                output.send(Event::Disconnected).await;
                break;
              }
            }
          }
          _ = ping_interval.tick() => {
            let timestamp = SystemTime::now()
              .duration_since(SystemTime::UNIX_EPOCH)
              .unwrap_or_default()
              .as_micros() as u64;
            let ping = ClientText::Ping { timestamp };
            let _ = ping_tx.try_send(ping.into_proto());
          }
        }
      }

      tokio::time::sleep(delays.next().unwrap_or(MAX_BACKOFF)).await;
    }
  })
}
