use std::sync::Arc;

use chat_rs::shared::convert::stream::proto::ClientVoiceMessage;
use chat_rs::shared::convert::{IntoProto, TryIntoDomain};
use chat_rs::shared::domain::stream::{ClientVoice, ServerVoice};
use futures::channel::mpsc;
use futures::stream::StreamExt;
use iced::task::{Never, Sipper, sipper};
use iced::{Task, futures};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::client;
use crate::model::{Model, Stream};

#[derive(Debug, Clone)]
pub struct WebRTCConnection(mpsc::Sender<ClientVoiceMessage>);

impl WebRTCConnection {
  pub fn send(&mut self, message: ClientVoice) {
    self
      .0
      .try_send(message.into_proto())
      .expect("Send message to server");
  }
}

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
          println!("Firing connect event.");
          output
            .send(Event::Connected(WebRTCConnection(tx).into()))
            .await;

          response.into_inner().fuse()
        }
        Err(e) => {
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

// handle messages

pub fn handle(model: &mut Model, msg: Box<ServerVoice>) -> Task<crate::Message> {
  let Some(client) = &model.webrtc_client else {
    eprintln!("No webrtc client found when receiving server offer/answer");
    return Task::none();
  };

  let client = client.clone();
  let webrtc_stream = model.webrtc_stream.clone();

  match *msg {
    ServerVoice::Offer(rtcsession_description) => Task::future(async move {
      let Stream::Connected(mut connection) = webrtc_stream else {
        eprintln!("No webrtc connection found when receiving server offer");
        return crate::Message::None;
      };

      let _ = {
        async || -> anyhow::Result<()> {
          client
            .set_remote_description(rtcsession_description)
            .await?;
          let reneg_answer = client.create_answer(None).await?;
          client.set_local_description(reneg_answer).await?;

          let answer = client
            .local_description()
            .await
            .ok_or(anyhow::anyhow!("No local description"))?;

          connection.send(ClientVoice::Answer(answer));
          Ok(())
        }
      }()
      .await
      .map_err(|e| eprintln!("Error handling server offer: {e:?}"));

      crate::Message::None
    }),
    ServerVoice::Answer(rtcsession_description) => Task::future(async move {
      let _ = client
        .set_remote_description(rtcsession_description)
        .await
        .map_err(|e| eprintln!("Error setting remote desc from server asnwer: {e:?}"));

      crate::Message::None
    }),
  }
}

pub fn start() -> Task<crate::Message> {
  Task::future(async move {
    let client = setup_client()
      .await
      .map_err(|e| eprintln!("Error setting up initial client! {e:?}"));

    match client {
      Ok((client, offer)) => crate::Message::WebRTCClientCreated(client.into(), offer),
      Err(_) => crate::Message::None,
    }
  })
}

pub async fn setup_client() -> anyhow::Result<(RTCPeerConnection, RTCSessionDescription)> {
  let mut media_engine = MediaEngine::default();

  media_engine.register_default_codecs()?;
  let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
  let api = APIBuilder::new()
    .with_media_engine(media_engine)
    .with_interceptor_registry(registry)
    .build();

  // no ice servers keeps tests to localhost only

  let client = api
    .new_peer_connection(RTCConfiguration::default())
    .await
    .map_err(|e| anyhow::anyhow!("Error setting up new peer conn {e:?}"))?;
  let offer = client.create_offer(None).await?;
  let mut gather = client.gathering_complete_promise().await;
  client.set_local_description(offer).await?;
  let _ = gather.recv().await;
  let offer = client.local_description().await.unwrap();

  // connection.send(ClientVoice::Offer(offer));

  Ok((client, offer))
}
