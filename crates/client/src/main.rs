use std::hash::Hasher;
use std::sync::Arc;
use std::time::Duration;

use chat_shared::convert::TryIntoDomain;
use chat_shared::domain::stream::{DisplayVoiceUser, ServerVoice, User};
use chat_shared::domain::user::MeReturn;
use futures_util::SinkExt;
use google_material_symbols::GoogleMaterialSymbols as Icon;
use iced::Theme::CatppuccinFrappe;
use iced::futures::channel::mpsc::Sender;
use iced::widget::{Text, column, container, row, text};
use iced::{Center, Element, Font, Length, Subscription, Task, Theme, stream};
use uuid::Uuid;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;

pub mod audio_processing;
mod chat_stream;
pub mod colors;
pub mod config;
pub mod voice_settings;
pub mod webrtc_stream;

use crate::audio_processing::call_handler::spawn_voice;
use crate::audio_processing::cues::Cue;
use crate::chat_stream::ChatConnection;
use crate::model::{Auth, LinkState, MediaHealth, PendingRejoin, Screen, Stream, VoiceCall};
use crate::screens::{auth, chat, settings};
use crate::webrtc_stream::WebRTCConnection;

mod client;
mod model;
mod screens;
mod types;
mod widgets;

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

      let mut subs = vec![
        voice_call_sub,
        iced::window::events().map(Message::Window),
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
      ];

      if model.chat_disconnected_since.is_some() {
        subs.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::None));
      }

      if matches!(model.screen, Screen::Settings(_)) {
        subs.push(iced::event::listen_with(
          |event, _status, _window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
              key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
              ..
            }) => Some(Message::Settings(settings::Message::Close)),
            _ => None,
          },
        ));
        subs.push(
          iced::time::every(Duration::from_millis(20))
            .map(|_| Message::Settings(settings::Message::Tick)),
        );
      }

      Subscription::batch(subs)
    }
    Auth::NotLoggedIn => Subscription::none(),
  }
}

#[derive(Clone)]
pub enum Message {
  Chat(chat::Message),
  Auth(auth::Message),
  Settings(settings::Message),
  Loaded(Option<MeReturn>),
  ChatStreamConnected(ChatConnection),
  ChatStreamDisconnected,
  ChatLatencyUpdated(u32),
  WebRTC(Box<ServerVoice>),
  ServerSentPresenceSnapshot {
    voice_channel_id: Uuid,
    peers: Vec<DisplayVoiceUser>,
  },
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
  CallReconnectTimedOut {
    id: u32,
  },
  VoiceGraceExpired {
    epoch: u32,
  },
  VoiceMediaHealth {
    epoch: u32,
    health: MediaHealth,
  },
  VoiceDeviceHealth {
    epoch: u32,
    input_ok: bool,
    output_ok: bool,
  },
  VoiceMicActivity {
    epoch: u32,
    receiving: bool,
  },
  LoggedIn(User),
  Window((iced::window::Id, iced::window::Event)),
}

fn persist_user_audio(
  per_user_audio: &std::collections::HashMap<Uuid, crate::voice_settings::UserAudioPref>,
) {
  let mut settings = crate::voice_settings::VoiceSettings::load();
  settings.per_user_volumes = per_user_audio.clone();
  settings.save();
}

fn update(model: &mut model::Model, message: Message) -> iced::Task<Message> {
  match message {
    Message::Settings(msg) => match msg {
      settings::Message::Close => match model.stashed_chat.take() {
        Some(chat_model) => {
          model.screen = Screen::Chat(chat_model);
          iced::Task::none()
        }
        None => {
          model.screen = Screen::Chat(Default::default());
          iced::Task::done(Message::Chat(chat::Message::Init))
        }
      },
      settings::Message::LogOut => {
        model.voice = None;
        model.pending_rejoin = None;
        model.chat_disconnected_since = None;
        model.webrtc_stream = Stream::Disconnected;
        model.chat_stream = Stream::Disconnected;
        model.active_server_id = None;
        model.room_presence.clear();
        model.stashed_chat = None;
        model.user = Auth::NotLoggedIn;
        model.screen = Screen::Auth(auth::Model::new());
        Task::future(async {
          client::get().await.clear_tokens().await;
          Message::None
        })
      }
      msg => {
        if let Some(voice) = &model.voice {
          match &msg {
            settings::Message::NoiseGateChanged(threshold) => {
              voice.handle.set_noise_gate(*threshold)
            }
            settings::Message::InputDeviceSelected(name) => {
              voice.handle.set_input_device(Some(name.clone()))
            }
            settings::Message::OutputDeviceSelected(name) => {
              voice.handle.set_output_device(Some(name.clone()))
            }
            _ => {}
          }
        }
        if let settings::Message::OutputDeviceSelected(name) = &msg {
          match &mut model.audio_cues {
            Some(cues) => {
              if let Err(e) = cues.rebuild(Some(name)) {
                eprintln!("audio cues rebuild failed: {e:?}");
              }
            }
            None => {
              model.audio_cues = crate::audio_processing::cues::AudioCues::new(Some(name))
                .map(|mut cues| {
                  cues.set_volume(0.1);
                  cues
                })
                .map_err(|e| eprintln!("audio cues init failed: {e:?}"))
                .ok();
            }
          }
        }
        if let Screen::Settings(settings_model) = &mut model.screen {
          settings::update(settings_model, msg).map(Message::Settings)
        } else {
          iced::Task::none()
        }
      }
    },
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user {
        match msg {
          chat::Message::JoinVoice { voice_channel_id } => {
            if let Some(voice) = &mut model.voice {
              voice.epoch += 1;
              voice.link_state = model::LinkState::Connecting;
              voice.mic_receiving = true;
              voice.input_ok = true;
              voice.output_ok = true;
              voice.handle.join(voice_channel_id, voice.epoch);
            }
            Task::none()
          }
          chat::Message::LeaveVoice => {
            let was_reconnecting = model.pending_rejoin.take().is_some();
            if let Some(ref mut voice) = model.voice {
              voice.handle.leave();
              voice.voice_call_id = None;
              voice.link_state = model::LinkState::Idle;
            }
            if (model.voice.is_some() || was_reconnecting)
              && let Some(ref mut cues) = model.audio_cues
            {
              cues.play(Cue::Leave);
            }
            Task::none()
          }
          chat::Message::ActiveServerChanged { server_id } => {
            model.active_server_id = Some(server_id);
            if let Some(voice) = &model.voice {
              voice.handle.subscribe_server(server_id);
            }
            Task::none()
          }
          chat::Message::ToggleMute => {
            if let Some(ref mut voice) = model.voice {
              voice.muted = !voice.muted;
              voice.handle.set_muted(voice.muted);
              if let Some(ref mut cues) = model.audio_cues {
                cues.play(if voice.muted { Cue::Mute } else { Cue::Unmute });
              };
            }
            Task::none()
          }
          chat::Message::ToggleDeafen => {
            if let Some(ref mut voice) = model.voice {
              voice.deafened = !voice.deafened;
              voice.muted = voice.deafened;
              voice.handle.set_deafened(voice.deafened);
              voice.handle.set_muted(voice.muted);
              if let Some(ref mut cues) = model.audio_cues {
                cues.play(if voice.deafened {
                  Cue::Deafen
                } else {
                  Cue::Undeafen
                });
              };
            }
            Task::none()
          }
          chat::Message::SetUserVolume { user_id, volume } => {
            let pref = model.per_user_audio.entry(user_id).or_default();
            pref.volume = volume.clamp(0.0, 2.0);
            let gain = pref.effective_gain();
            if let Some(voice) = &model.voice {
              voice.handle.set_user_volume(user_id, gain);
            }
            Task::none()
          }
          chat::Message::UserVolumeReleased { .. } => {
            persist_user_audio(&model.per_user_audio);
            Task::none()
          }
          chat::Message::ToggleUserMute { user_id } => {
            let pref = model.per_user_audio.entry(user_id).or_default();
            pref.muted = !pref.muted;
            let gain = pref.effective_gain();
            persist_user_audio(&model.per_user_audio);
            if let Some(voice) = &model.voice {
              voice.handle.set_user_volume(user_id, gain);
            }
            Task::none()
          }
          chat::Message::GoToSettings => {
            let prev = std::mem::replace(&mut model.screen, Screen::Settings(Default::default()));
            if let Screen::Chat(chat_model) = prev {
              model.stashed_chat = Some(chat_model);
            }
            Task::none()
          }
          chat::Message::PlayCue(cue) => {
            if model.audio_cues.is_none() {
              let device = crate::voice_settings::VoiceSettings::load().output_device;
              model.audio_cues = crate::audio_processing::cues::AudioCues::new(device.as_deref())
                .map(|mut cues| {
                  cues.set_volume(0.1);
                  cues
                })
                .map_err(|e| eprintln!("audio cues init failed: {e:?}"))
                .ok();
            }
            if let Some(ref mut cues) = model.audio_cues {
              cues.play(cue);
            };

            Task::none()
          }
          other => {
            let chat_model = match &mut model.screen {
              Screen::Chat(chat_model) => Some(chat_model),
              _ => model.stashed_chat.as_mut(),
            };
            if let Some(chat_model) = chat_model {
              chat::update(
                chat_model,
                other,
                user,
                model.chat_stream.clone(),
                &model.window_state,
              )
              .map(Message::Chat)
            } else {
              iced::Task::none()
            }
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
      model
        .chat_disconnected_since
        .get_or_insert_with(std::time::Instant::now);
      Task::none()
    }
    Message::ChatStreamConnected(connection) => {
      model.chat_stream = Stream::Connected(connection);
      model.chat_disconnected_since = None;
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
        match *msg {
          ServerVoice::PresenceSnapshot {
            voice_channel_id,
            server_id: _,
            peers,
          } => Task::done(Message::ServerSentPresenceSnapshot {
            voice_channel_id,
            peers,
          }),
          _ => {
            voice.handle.signal(*msg);
            Task::none()
          }
        }
      } else {
        Task::none()
      }
    }
    Message::WebRTCSignalStreamConnected(conn) => {
      let mut voice = VoiceCall {
        handle: spawn_voice(conn),
        link_state: model::LinkState::Connecting,
        media: model::MediaHealth::Unknown,
        latency_ms: 0,
        voice_call_id: None,
        epoch: 1,
        presence_snapshot: vec![],
        muted: false,
        deafened: false,
        input_ok: true,
        output_ok: true,
        mic_receiving: true,
      };

      if let Some(server_id) = model.active_server_id {
        voice.handle.subscribe_server(server_id);
      }

      if let Some(rejoin) = &model.pending_rejoin {
        voice.muted = rejoin.muted;
        voice.deafened = rejoin.deafened;
        voice.handle.set_deafened(rejoin.deafened);
        voice.handle.set_muted(rejoin.muted);
        voice.link_state = model::LinkState::Reconnecting { attempt: 1 };
        voice.handle.join(rejoin.voice_channel_id, voice.epoch);
      }

      model.room_presence.clear();
      model.chat_disconnected_since = None;
      model.voice = Some(voice);

      Task::none()
    }
    Message::WebRTCSignalStreamDisconnected => {
      model.webrtc_stream = Stream::Disconnected;

      let in_call = model
        .voice
        .as_ref()
        .and_then(|v| v.voice_call_id.map(|id| (id, v.muted, v.deafened)));
      model.voice = None; // drops the handle → actor loop ends

      let Some((voice_channel_id, muted, deafened)) =
        in_call.filter(|_| model.pending_rejoin.is_none())
      else {
        return Task::none();
      };

      model.reconnect_seq = model.reconnect_seq.wrapping_add(1);
      let id = model.reconnect_seq;
      model.pending_rejoin = Some(PendingRejoin {
        voice_channel_id,
        muted,
        deafened,
        id,
      });

      Task::perform(
        async { tokio::time::sleep(Duration::from_secs(15)).await },
        move |_| Message::CallReconnectTimedOut { id },
      )
    }
    Message::CallReconnectTimedOut { id } => {
      if model.pending_rejoin.as_ref().is_some_and(|r| r.id == id) {
        model.pending_rejoin = None;
        if let Some(ref mut cues) = model.audio_cues {
          cues.play(Cue::Leave);
        }
      }
      Task::none()
    }
    Message::JoinVoiceSuccessful { voice_channel_id } => {
      model.pending_rejoin = None;
      if let Some(ref mut voice) = model.voice {
        voice.voice_call_id = Some(voice_channel_id);
        let reconnecting = matches!(voice.link_state, LinkState::Reconnecting { .. });
        if model.audio_cues.is_none() {
          let device = crate::voice_settings::VoiceSettings::load().output_device;
          model.audio_cues = crate::audio_processing::cues::AudioCues::new(device.as_deref())
            .map(|mut cues| {
              cues.set_volume(0.1);
              cues
            })
            .map_err(|e| eprintln!("audio cues init failed: {e:?}"))
            .ok();
        }
        if !reconnecting && let Some(ref mut cues) = model.audio_cues {
          cues.play(Cue::Join);
        };
      };
      Task::none()
    }
    Message::VoiceHandlePeerConnectionChanged {
      state: rtcpeer_connection_state,
      epoch,
    } => {
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
          if matches!(call.link_state, LinkState::Reconnecting { .. })
            && let Some(ref mut cues) = model.audio_cues
          {
            cues.play(Cue::Join);
          }
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
    Message::VoiceDeviceHealth {
      epoch,
      input_ok,
      output_ok,
    } => {
      let Some(ref mut call) = model.voice else {
        return Task::none();
      };

      if epoch != call.epoch {
        return Task::none();
      }

      call.input_ok = input_ok;
      call.output_ok = output_ok;

      Task::none()
    }
    Message::VoiceMicActivity { epoch, receiving } => {
      let Some(ref mut call) = model.voice else {
        return Task::none();
      };

      if epoch != call.epoch {
        return Task::none();
      }

      call.mic_receiving = receiving;

      Task::none()
    }
    Message::ServerSentPresenceSnapshot {
      voice_channel_id,
      peers,
    } => {
      model.room_presence.insert(voice_channel_id, peers.clone());

      if let Some(ref mut call) = model.voice
        && call.voice_call_id == Some(voice_channel_id)
      {
        if let Some(ref mut cues) = model.audio_cues {
          if call.presence_snapshot.len() > peers.len() {
            cues.play(Cue::PeerLeave);
          } else if call.presence_snapshot.len() < peers.len() {
            cues.play(Cue::PeerJoin);
          }
        }

        call.presence_snapshot = peers;
      }

      Task::none()
    }
    Message::Window((_id, event)) => {
      match event {
        iced::window::Event::Focused => model.window_state = model::WindowState::Focused,
        iced::window::Event::Unfocused => model.window_state = model::WindowState::NotFocused,
        _ => (),
      };
      Task::none()
    }
  }
}

fn reconnect(call: &mut VoiceCall, id: Uuid) {
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
  call.epoch += 1;
  call.link_state = LinkState::Reconnecting { attempt };
  call.handle.leave();
  call.handle.join(id, call.epoch);
}

const BANNER_AFTER: Duration = Duration::from_secs(2);
const OVERLAY_AFTER: Duration = Duration::from_secs(10);

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let content = match &model.screen {
    model::Screen::Auth(model) => auth::view(model).map(Message::Auth),
    model::Screen::Chat(chat_model) => screens::chat::view(
      chat_model,
      model
        .voice
        .as_ref()
        .and_then(|voice| voice.voice_call_id.as_ref())
        .or_else(|| model.pending_rejoin.as_ref().map(|r| &r.voice_channel_id)),
      model,
    )
    .map(Message::Chat),
    model::Screen::Settings(settings_model) => {
      screens::settings::view(settings_model).map(Message::Settings)
    }
  };

  let down_for = matches!(model.user, Auth::LoggedIn(_))
    .then(|| model.chat_disconnected_since.map(|t| t.elapsed()))
    .flatten();

  let view: Element<'_, Message> = match down_for {
    Some(elapsed) if elapsed >= OVERLAY_AFTER => no_connection_overlay(),
    Some(elapsed) if elapsed >= BANNER_AFTER => column![reconnecting_banner(), content].into(),
    _ => content,
  };

  container(container(view).style(container::rounded_box)).into()
}

fn reconnecting_banner<'a>() -> Element<'a, Message> {
  container(
    row![
      icon(Icon::Sync).size(16),
      text("Reconnecting to server…").size(14),
    ]
    .spacing(SPACE_GRID as u32)
    .align_y(Center),
  )
  .width(Length::Fill)
  .padding(SPACE_GRID)
  .style(|theme: &Theme| {
    let warning = theme.extended_palette().warning;
    container::Style {
      background: Some(warning.weak.color.into()),
      text_color: Some(warning.weak.text),
      ..container::Style::default()
    }
  })
  .into()
}

fn no_connection_overlay<'a>() -> Element<'a, Message> {
  container(
    column![
      icon(Icon::CloudOff).size(48),
      text("No connection").size(22).font(Font {
        weight: iced::font::Weight::Bold,
        ..SOURCE_SANS_REGULAR
      }),
      text("Trying to reach the server…")
        .size(14)
        .style(text::secondary),
    ]
    .spacing(SPACE_GRID as u32)
    .align_x(Center),
  )
  .width(Length::Fill)
  .height(Length::Fill)
  .center_x(Length::Fill)
  .center_y(Length::Fill)
  .into()
}
