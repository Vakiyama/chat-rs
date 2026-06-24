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

      // While the heartbeat is down, tick once a second so the connection-loss
      // banner/overlay thresholds cross even during the chat stream's backoff
      // silence (it can sit quiet for up to 30s between retries). Message::None
      // just re-runs view, which recomputes elapsed from chat_disconnected_since.
      if model.chat_disconnected_since.is_some() {
        subs.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::None));
      }

      // Settings-only subscriptions: Esc closes back to chat, and a timer drives
      // the live mic-level meter. Scoped so they can't fire on chat/auth.
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
  // the in-call signaling reconnect window elapsed; if `id` still matches the
  // active pending_rejoin we give up and drop the user from the call UI.
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
  // per-device audio health for the current call: whether the mic and speaker
  // streams actually came up. Reported on join and on every live device swap.
  VoiceDeviceHealth {
    epoch: u32,
    input_ok: bool,
    output_ok: bool,
  },
  // whether mic capture frames are currently arriving. Drops false when the mic
  // opened but stopped delivering (unplugged / OS-muted / broken).
  VoiceMicActivity {
    epoch: u32,
    receiving: bool,
  },
  LoggedIn(User),
}

// Mirror the in-memory per-user mixer levels into the persisted voice settings.
// Load-modify-save so we only overwrite this one field and keep the gate/device
// choices the settings screen owns. Failure is logged (inside save), never fatal.
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
      // back to chat (close button / Esc). The chat model was dropped on the way
      // in, so rebuild it and re-init like a fresh login does.
      settings::Message::Close => match model.stashed_chat.take() {
        // restore the chat model we stashed on the way in — instant, no reload.
        Some(chat_model) => {
          model.screen = Screen::Chat(chat_model);
          iced::Task::none()
        }
        // no stash (e.g. we opened straight into settings): build + init fresh.
        None => {
          model.screen = Screen::Chat(Default::default());
          iced::Task::done(Message::Chat(chat::Message::Init))
        }
      },
      settings::Message::LogOut => {
        // tear down voice + streams (subscriptions gate on LoggedIn, so they
        // stop once user flips to NotLoggedIn) and return to the auth screen.
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
        // forward live audio changes to the running voice handle (if any); the
        // settings model owns display state + persistence to disk.
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
        // Re-bind audio cues to the chosen output device so they follow the same
        // device the call uses. This also recovers cues that failed to start
        // (e.g. no working device at startup) once a usable device is selected.
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
              // bump and drive the join on the model's current epoch so the new pc's
              // state callbacks aren't filtered out by a stale epoch left behind by an
              // earlier auto-reconnect.
              voice.epoch += 1;
              voice.link_state = model::LinkState::Connecting;
              // clear any stale device warnings from a prior call; the fresh
              // join re-reports device health and mic liveness.
              voice.mic_receiving = true;
              voice.input_ok = true;
              voice.output_ok = true;
              voice.handle.join(voice_channel_id, voice.epoch);
            }
            Task::none()
          }
          chat::Message::LeaveVoice => {
            // Leaving while "Reconnecting..." (voice is None, only pending_rejoin
            // is set) must cancel the auto-rejoin so the stream coming back doesn't
            // surprise-rejoin us. The stale give-up timer no-ops once this is None.
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
            // remember the active server (so we can re-subscribe on reconnect) and
            // subscribe now if the voice stream is already up.
            model.active_server_id = Some(server_id);
            if let Some(voice) = &model.voice {
              voice.handle.subscribe_server(server_id);
            }
            Task::none()
          }
          chat::Message::ToggleMute => {
            if let Some(ref mut voice) = model.voice {
              // an explicit mute toggle never changes deafen state.
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
              // deafen implies mute: deafening forces mute on, undeafening clears
              // both. The actor mirrors these onto the audio pipeline + server.
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
            // live update while dragging: record the new level (clamped to the
            // slider's 0..=200% range) and push the gain to the call so the user
            // hears the change immediately. Persistence happens on release.
            let pref = model.per_user_audio.entry(user_id).or_default();
            pref.volume = volume.clamp(0.0, 2.0);
            let gain = pref.effective_gain();
            if let Some(voice) = &model.voice {
              voice.handle.set_user_volume(user_id, gain);
            }
            Task::none()
          }
          chat::Message::UserVolumeReleased { .. } => {
            // drag finished — persist the whole map once.
            persist_user_audio(&model.per_user_audio);
            Task::none()
          }
          chat::Message::ToggleUserMute { user_id } => {
            // flip mute while keeping the remembered volume for un-mute.
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
            // stash the live chat model so returning from settings is instant.
            // (chat_model isn't borrowed in this arm, so replacing screen is fine.)
            let prev = std::mem::replace(&mut model.screen, Screen::Settings(Default::default()));
            if let Screen::Chat(chat_model) = prev {
              model.stashed_chat = Some(chat_model);
            }
            Task::none()
          }
          other => {
            // apply to whichever chat model is live: the on-screen one, or the
            // stashed one while we're in settings (so it stays current).
            let chat_model = match &mut model.screen {
              Screen::Chat(chat_model) => Some(chat_model),
              _ => model.stashed_chat.as_mut(),
            };
            if let Some(chat_model) = chat_model {
              chat::update(chat_model, other, user, model.chat_stream.clone()).map(Message::Chat)
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
        // model.screen = Screen::Settings(Default::default());
        model.user = Auth::LoggedIn(User {
          id: response.user_id,
          name: response.username.clone(),
        });

        iced::Task::done(Message::Chat(chat::Message::Init))
        // Task::none()
      }
      None => Task::none(),
    },
    Message::ChatStreamDisconnected => {
      model.chat_stream = Stream::Disconnected;
      // Stamp the moment we lost the heartbeat (first drop only, so a flapping
      // stream keeps the original elapsed). Drives the banner/overlay timing.
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
        // assume healthy until a join (or device swap) reports otherwise.
        input_ok: true,
        output_ok: true,
        mic_receiving: true,
      };

      // (re)subscribe to the active server's call presence so a fresh or
      // reconnected voice stream immediately receives a snapshot of all rooms.
      if let Some(server_id) = model.active_server_id {
        voice.handle.subscribe_server(server_id);
      }

      // Auto-rejoin the call we were dropped from. Restore mute/deafen onto the
      // fresh handle's atomics (set_* just stores them while no call is active),
      // then join — which re-announces them to the server, the same way a normal
      // fresh join does. We leave pending_rejoin set (so the card keeps showing
      // "Reconnecting…" with no flicker until voice_call_id lands) and clear it in
      // JoinVoiceSuccessful once the rejoin is confirmed. Until then the give-up
      // timer still guards us if the join never completes.
      if let Some(rejoin) = &model.pending_rejoin {
        voice.muted = rejoin.muted;
        voice.deafened = rejoin.deafened;
        voice.handle.set_deafened(rejoin.deafened);
        voice.handle.set_muted(rejoin.muted);
        voice.link_state = model::LinkState::Reconnecting { attempt: 1 };
        voice.handle.join(rejoin.voice_channel_id, voice.epoch);
      }

      // Drop any presence we were showing for this server's rooms; it's rebuilt
      // from the fresh SubscribeServer / in-call snapshots. This clears ghosts
      // (e.g. a peer that vanished while we were disconnected) instead of leaving
      // them stuck on screen — empty rooms send no snapshot, so they stay cleared.
      model.room_presence.clear();

      model.voice = Some(voice);

      Task::none()
    }
    Message::WebRTCSignalStreamDisconnected => {
      model.webrtc_stream = Stream::Disconnected;

      // If we were in a call, remember it (and our mute/deafen state) so we can
      // auto-rejoin once the stream reconnects, instead of silently dropping the
      // user. A manual leave clears voice_call_id, so this won't surprise-rejoin.
      // The card keeps showing "Reconnecting..." off this pending_rejoin until we
      // either recover or the give-up timer below fires. Capture before dropping
      // the handle below (which ends the actor loop and tears down audio).
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

      // Arm a give-up timer: if the stream hasn't reconnected and re-joined us by
      // the deadline, CallReconnectTimedOut drops us from the call client-side.
      Task::perform(
        async { tokio::time::sleep(Duration::from_secs(30)).await },
        move |_| Message::CallReconnectTimedOut { id },
      )
    }
    Message::CallReconnectTimedOut { id } => {
      // Only give up if this is still the same pending reconnect we armed the timer
      // for. A successful rejoin (or a newer disconnect) leaves a different/empty
      // pending_rejoin, in which case this is a stale tick and we do nothing.
      if model.pending_rejoin.as_ref().is_some_and(|r| r.id == id) {
        model.pending_rejoin = None;
        if let Some(ref mut cues) = model.audio_cues {
          cues.play(Cue::Leave);
        }
      }
      Task::none()
    }
    Message::JoinVoiceSuccessful { voice_channel_id } => {
      // The rejoin (if any) is confirmed; drop the pending state so its give-up
      // timer no-ops. voice_call_id below now drives the card.
      model.pending_rejoin = None;
      if let Some(ref mut voice) = model.voice {
        voice.voice_call_id = Some(voice_channel_id);
        // Cues may have failed to initialize at startup (no working device then).
        // A join is a good moment to retry against the currently-saved device so
        // the join/leave/peer cues come back to life without a restart.
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
          cues.play(Cue::Join);
        };
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

      // keep the existing in-call presence in sync when the snapshot is for the
      // call we're actually in (that one carries live speaking state).
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

// How long the heartbeat must be down before we surface it: a slim banner first,
// then a full "no connection" overlay once it's clearly not a momentary blip.
const BANNER_AFTER: Duration = Duration::from_secs(2);
const OVERLAY_AFTER: Duration = Duration::from_secs(10);

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let content = match &model.screen {
    model::Screen::Auth(model) => auth::view(model).map(Message::Auth),
    model::Screen::Chat(chat_model) => screens::chat::view(
      chat_model,
      // keep the joined channel highlighted while we're reconnecting: fall back to
      // the pending_rejoin channel when the live voice handle is gone.
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

  // App-level connection loss. Only meaningful once logged in (the streams only
  // run then). The 1s tick subscription keeps this re-evaluating while down so the
  // thresholds actually fire during the chat stream's quiet backoff windows.
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

// Slim full-width strip pinned above the app while the heartbeat is down but we
// haven't yet given up — reassures the user we're actively retrying.
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

// Full-screen takeover once we've been disconnected long enough that the app is
// effectively unusable; replaces the normal content until the server is back.
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
