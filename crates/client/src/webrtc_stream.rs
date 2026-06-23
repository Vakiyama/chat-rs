//! The voice signaling subscription: an iced `Sipper` that runs the grpc voice
//! stream, reconnecting on drop. The voice engine and audio pipeline live in
//! `chat_core::rtc`; this file only bridges the grpc transport to iced messages
//! and hands the engine a [`WebRTCConnection`] to send outbound signaling.

use crate::client;
use chat_core::rtc::WebRTCConnection;
use chat_shared::convert::TryIntoDomain;
use chat_shared::convert::stream::proto::ClientVoiceMessage;
use chat_shared::domain::stream::ServerVoice;
use futures::channel::mpsc;
use futures::stream::StreamExt;
use iced::futures;
use iced::task::{Never, Sipper, sipper};

#[derive(Debug, Clone)]
pub enum Event {
  Connected(WebRTCConnection),
  Disconnected,
  MessageReceived(Box<ServerVoice>),
}

impl From<Event> for crate::Message {
  fn from(val: Event) -> Self {
    match val {
      Event::Connected(connection) => crate::Message::WebRTCSignalStreamConnected(connection),
      Event::Disconnected => crate::Message::WebRTCSignalStreamDisconnected,
      Event::MessageReceived(server_message) => crate::Message::WebRTC(server_message),
    }
  }
}

pub fn connect() -> impl Sipper<Never, Event> {
  sipper(async |mut output| {
    // reconnect loop; awaits 1 second on disconnect/error and retries
    loop {
      let (tx, rx) = mpsc::channel::<ClientVoiceMessage>(100);
      let mut receiver = match client::get().await.stream.connect_voice_stream(rx).await {
        Ok(response) => {
          println!("Firing webrtc connect event.");
          output
            .send(Event::Connected(WebRTCConnection::new(tx)))
            .await;

          response.into_inner().fuse()
        }
        Err(_) => {
          tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
          continue;
        }
      };

      loop {
        match receiver.select_next_some().await {
          Ok(server_msg) => match server_msg.try_into_domain().map(Box::new) {
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
