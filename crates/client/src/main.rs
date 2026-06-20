use std::hash::Hasher;
use std::sync::Arc;
use std::time::Duration;

use chat_shared::convert::TryIntoDomain;
use chat_shared::domain::stream::{ServerVoice, User};
use chat_shared::domain::user::MeReturn;
use futures_util::SinkExt;
use google_material_symbols::GoogleMaterialSymbols as Icon;
use iced::Theme::CatppuccinFrappe;
use iced::futures::channel::mpsc::Sender;
use iced::widget::{Text, container, text};
use iced::{Element, Font, Subscription, Task, stream};
use uuid::Uuid;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;

pub mod audio_processing;
mod chat_stream;
pub mod config;
pub mod webrtc_stream;

use crate::audio_processing::call_handler::spawn_voice;
use crate::chat_stream::ChatConnection;
use crate::model::{Auth, LinkState, MediaHealth, Screen, Stream, VoiceCall};
use crate::screens::{auth, chat};
use crate::webrtc_stream::WebRTCConnection;

mod client;
mod model;
mod screens;
mod types;

const SPACE_GRID: u16 = 8;

pub const SOURCE_SANS_REGULAR: Font = Font::with_name("Source Sans 3");

fn main() -> iced::Result {
  iced::application(new, update, view)
    .theme(CatppuccinFrappe)
    .font(google_material_symbols::GoogleMaterialSymbols::FONT_BYTES)
    .font(include_bytes!(
      "../resources/source_sans/static/SourceSans3-Regular.ttf"
    ))
    .font(include_bytes!(
      "../resources/source_sans/static/SourceSans3-Light.ttf"
    ))
    .font(include_bytes!(
      "../resources/source_sans/static/SourceSans3-Medium.ttf"
    ))
    .font(include_bytes!(
      "../resources/source_sans/static/SourceSans3-SemiBold.ttf"
    ))
    .font(include_bytes!(
      "../resources/source_sans/static/SourceSans3-Bold.ttf"
    ))
    .font(include_bytes!(
      "../resources/source_sans/static/SourceSans3-ExtraBold.ttf"
    ))
    .default_font(SOURCE_SANS_REGULAR)
    .subscription(subscription)
    .run()
}

// for icon loading

pub const MATERIAL: Font = Font::with_name(Icon::FONT_FAMILY); // "Material Symbols Sharp"

pub fn icon<'a>(i: Icon) -> Text<'a> {
  text(char::from(i).to_string()).font(MATERIAL)
}

fn new() -> (model::Model, Task<Message>) {
  (
    model::Model::default(),
    Task::perform(
      async {
        let mut client = client::get().await;

        if client.has_tokens().await {
          client
            .user
            .me(())
            .await
            .unwrap()
            .into_inner()
            .try_into_domain()
            .ok()
        } else {
          None
        }
      },
      Message::Loaded,
    ),
  )
}

struct VoiceSub {
  call_id: Option<Uuid>,
  rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Message>>>,
}

impl std::hash::Hash for VoiceSub {
  fn hash<H: Hasher>(&self, state: &mut H) {
    // identity is the call, never the receiver
    self.call_id.hash(state);
  }
}

fn subscription(model: &model::Model) -> Subscription<Message> {
  match model.user {
    Auth::LoggedIn(_) => {
      let voice_call_sub = match &model.voice {
        Some(call) => Subscription::run_with(
          VoiceSub {
            call_id: call.voice_call_id,
            rx: call.handle.receiver.clone(),
          },
          |call| {
            let rx = call.rx.clone();
            stream::channel(16, move |mut output: Sender<Message>| async move {
              let mut rx = rx.lock().await;

              while let Some(event) = rx.recv().await {
                if output.send(event).await.is_err() {
                  break;
                }
              }
            })
          },
        ),
        None => Subscription::none(),
      };

      Subscription::batch([
        voice_call_sub,
        Subscription::run(chat_stream::connect).map(|event| event.into()),
        Subscription::run(webrtc_stream::connect).map(|event| event.into()),
        iced::event::listen_with(|event, status, _window| match (event, status) {
          (
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
              key: iced::keyboard::Key::Character(_),
              text: Some(text),
              modifiers,
              ..
            }),
            iced::event::Status::Ignored,
          ) if !modifiers.command() && !modifiers.control() && !modifiers.alt() => {
            Some(Message::Chat(chat::Message::TypeAhead(text.into())))
          }
          _ => None,
        }),
      ])
    }
    Auth::NotLoggedIn => Subscription::none(),
  }
}

#[derive(Clone)]
pub enum Message {
  Chat(chat::Message),
  Auth(auth::Message),
  Loaded(Option<MeReturn>),
  ChatStreamConnected(ChatConnection),
  ChatStreamDisconnected,
  ChatLatencyUpdated(u32),
  WebRTC(Box<ServerVoice>),
  WebRTCSignalStreamConnected(WebRTCConnection),
  WebRTCSignalStreamDisconnected,
  VoiceHandlePeerConnectionChanged {
    state: RTCPeerConnectionState,
    epoch: u32,
  },
  JoinVoiceSuccessful {
    voice_channel_id: Uuid,
  },
  None,
  VoiceGraceExpired {
    epoch: u32,
  },
  VoiceMediaHealth {
    epoch: u32,
    health: MediaHealth,
  },
  LoggedIn(User),
}

fn update(model: &mut model::Model, message: Message) -> iced::Task<Message> {
  match message {
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user
        && let Screen::Chat(chat_model) = &mut model.screen
      {
        match msg {
          chat::Message::JoinVoice { voice_channel_id } => {
            if let Some(voice) = &mut model.voice {
              // bump and drive the join on the model's current epoch so the new pc's
              // state callbacks aren't filtered out by a stale epoch left behind by an
              // earlier auto-reconnect.
              voice.epoch += 1;
              voice.link_state = model::LinkState::Connecting;
              voice.handle.join(voice_channel_id, voice.epoch);
            }
            Task::none()
          }
          chat::Message::LeaveVoice => {
            if let Some(ref mut voice) = model.voice {
              voice.handle.leave();
              voice.voice_call_id = None;
              voice.link_state = model::LinkState::Idle;
            }
            Task::none()
          }
          other => {
            chat::update(chat_model, other, user, model.chat_stream.clone()).map(Message::Chat)
          }
        }
      } else {
        iced::Task::none()
      }
    }
    Message::LoggedIn(user) => {
      model.screen = Screen::Chat(Default::default());
      model.user = Auth::LoggedIn(user);
      iced::Task::done(Message::Chat(chat::Message::Init))
    }
    Message::Auth(msg) => {
      if let Auth::NotLoggedIn = &model.user
        && let Screen::Auth(auth_model) = &mut model.screen
      {
        match msg {
          auth::Message::ApiVerifiedCode(Ok(response)) => {
            let user = User {
              id: response.user_id,
              name: response.username.clone(),
            };

            Task::future(async move {
              client::get()
                .await
                .insert_tokens(response.refresh_token, response.access_token)
                .await;
              Message::LoggedIn(user)
            })
          }
          msg => auth::update(auth_model, msg).map(Message::Auth),
        }
      } else if let Auth::LoggedIn(_) = model.user {
        model.screen = Screen::Chat(Default::default());
        iced::Task::done(Message::Chat(chat::Message::Init))
      } else {
        iced::Task::none()
      }
    }
    Message::None => iced::Task::none(),
    Message::Loaded(me_return) => match me_return {
      Some(response) => {
        model.screen = Screen::Chat(Default::default());
        model.user = Auth::LoggedIn(User {
          id: response.user_id,
          name: response.username.clone(),
        });

        iced::Task::done(Message::Chat(chat::Message::Init))
      }
      None => Task::none(),
    },
    Message::ChatStreamDisconnected => {
      model.chat_stream = Stream::Disconnected;
      Task::none()
    }
    Message::ChatStreamConnected(connection) => {
      model.chat_stream = Stream::Connected(connection);

      Task::none()
    }
    Message::ChatLatencyUpdated(latency_ms) => {
      if let Some(ref mut voice) = model.voice {
        voice.latency_ms = latency_ms;
      }
      Task::none()
    }
    Message::WebRTC(msg) => {
      if let Some(voice) = &model.voice {
        voice.handle.signal(*msg);
      }
      Task::none()
    }
    Message::WebRTCSignalStreamConnected(conn) => {
      model.voice = Some(VoiceCall {
        handle: spawn_voice(conn),
        link_state: model::LinkState::Connecting,
        media: model::MediaHealth::Unknown,
        latency_ms: 0,
        voice_call_id: None,
        epoch: 1,
      });

      Task::none()
    }
    Message::WebRTCSignalStreamDisconnected => {
      model.voice = None; // drops the handle → actor loop ends
      model.webrtc_stream = Stream::Disconnected;
      Task::none()
    }
    Message::JoinVoiceSuccessful { voice_channel_id } => {
      if let Some(ref mut voice) = model.voice {
        voice.voice_call_id = Some(voice_channel_id);
      };
      Task::none()
    }
    Message::VoiceHandlePeerConnectionChanged {
      state: rtcpeer_connection_state,
      epoch,
    } => {
      // if we're in a call
      let Some(ref mut call) = model.voice else {
        return Task::none();
      };

      let Some(voice_call_id) = call.voice_call_id else {
        return Task::none();
      };

      if epoch != call.epoch {
        return Task::none();
      }

      match rtcpeer_connection_state {
        RTCPeerConnectionState::New | RTCPeerConnectionState::Connecting => {
          call.link_state = LinkState::Connecting;
          Task::none()
        }
        RTCPeerConnectionState::Connected => {
          call.link_state = LinkState::Live;
          Task::none()
        }
        RTCPeerConnectionState::Disconnected => {
          if matches!(call.link_state, LinkState::Live | LinkState::Connecting) {
            call.link_state = LinkState::Unstable;
            let epoch = call.epoch;
            Task::perform(
              async { tokio::time::sleep(Duration::from_secs(3)).await },
              move |_| Message::VoiceGraceExpired { epoch },
            )
          } else {
            Task::none()
          }
        }
        RTCPeerConnectionState::Failed => {
          reconnect(call, voice_call_id);
          Task::none()
        }
        RTCPeerConnectionState::Closed => {
          if matches!(call.link_state, LinkState::Reconnecting { .. }) {
            Task::none() // old PC dying during our own reconnect
          } else {
            reconnect(call, voice_call_id);
            Task::none()
          }
        }
        RTCPeerConnectionState::Unspecified => Task::none(),
      }
    }
    Message::VoiceGraceExpired { epoch } => {
      // if we're in a call
      let Some(ref mut call) = model.voice else {
        return Task::none();
      };

      let Some(voice_call_id) = call.voice_call_id else {
        return Task::none();
      };

      if epoch != call.epoch {
        return Task::none();
      }

      if matches!(call.link_state, LinkState::Unstable) {
        reconnect(call, voice_call_id);
      };

      Task::none()
    }
    Message::VoiceMediaHealth { epoch, health } => {
      let Some(ref mut call) = model.voice else {
        return Task::none();
      };

      if epoch != call.epoch {
        return Task::none();
      }

      call.media = health;

      Task::none()
    }
  }
}

fn reconnect(call: &mut VoiceCall, id: Uuid) {
  // Only recover a call that is actually live or already mid-recovery. Idle (the user
  // deliberately left) and Lost (we already gave up) must never silently rejoin — note
  // our own pc.close() during a manual leave emits Closed, which routes here.
  let attempt = match call.link_state {
    LinkState::Reconnecting { attempt } => attempt + 1,
    LinkState::Live | LinkState::Connecting | LinkState::Unstable => 1,
    LinkState::Idle | LinkState::Lost { .. } => return,
  };

  const MAX: u32 = 4;
  if attempt > MAX {
    call.link_state = LinkState::Lost {
      reason: "Too many attempts".into(),
    };
    return;
  }
  // Bump the epoch BEFORE rejoining and drive the new connection on that same epoch, so
  // late callbacks from the dying pc are filtered out while the fresh pc's callbacks
  // (which carry call.epoch) are accepted. Passing a stale epoch here would make the
  // model ignore every state change from the reconnected call.
  call.epoch += 1;
  call.link_state = LinkState::Reconnecting { attempt };
  call.handle.leave();
  call.handle.join(id, call.epoch);
}

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let view = match &model.screen {
    model::Screen::Auth(model) => auth::view(model).map(Message::Auth),
    model::Screen::Chat(chat_model) => screens::chat::view(
      chat_model,
      model
        .voice
        .as_ref()
        .and_then(|voice| voice.voice_call_id.as_ref()),
      model,
    )
    .map(Message::Chat),
  };

  container(container(view).style(container::rounded_box)).into()
}
