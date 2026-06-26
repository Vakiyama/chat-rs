use crate::audio_processing::{call_handler::VoiceHandle, cues::AudioCues};
use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;
use crate::screens::settings::Model as SettingsModel;
use crate::voice_settings::UserAudioPref;
use chat_shared::domain::stream::{DisplayVoiceUser, User};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{chat_stream, webrtc_stream};

#[derive(Clone)]
pub enum Stream<T> {
  Connected(T),
  Disconnected,
}

pub enum Auth {
  LoggedIn(User),
  NotLoggedIn,
}

// The call to restore after the voice signaling stream drops and reconnects.
// Captured the moment the stream disconnects (while we still know which call we
// were in and our mute/deafen state), then drained on reconnect to auto-rejoin.
pub struct PendingRejoin {
  pub voice_channel_id: Uuid,
  pub muted: bool,
  pub deafened: bool,
  // generation stamp (from Model::reconnect_seq) so a stale give-up timer only
  // drops the call it was armed for, not one we've since recovered or re-dropped.
  pub id: u32,
}

#[derive(Debug)]
pub enum LinkState {
  Idle,                          // closed / never started
  Connecting,                    // raw: New | Connecting
  Live,                          // raw: Connected
  Unstable,                      // raw: Disconnected — grace window, may recover
  Reconnecting { attempt: u32 }, // you're driving recovery (ICE restart / rejoin)
  Lost { reason: String },       // gave up <- update to non string value if we can
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MediaHealth {
  Unknown,           // no baseline yet
  Flowing,           // inbound audio bytes climbing
  NoAudio,           // connected but no inbound audio for a while (see DTX caveat)
  TransportDegraded, // nominated pair stopped getting STUN responses — link dying
}

pub struct VoiceCall {
  pub link_state: LinkState,
  pub media: MediaHealth,
  pub latency_ms: u32,
  pub handle: VoiceHandle,
  pub voice_call_id: Option<Uuid>,
  pub epoch: u32,
  pub presence_snapshot: Vec<DisplayVoiceUser>,
  pub muted: bool,
  pub deafened: bool,
  pub input_ok: bool,
  pub output_ok: bool,
  pub mic_receiving: bool,
}

impl std::hash::Hash for VoiceCall {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.voice_call_id.hash(state);
  }
}

pub struct Model {
  pub screen: Screen,
  pub user: Auth,
  pub chat_stream: Stream<chat_stream::ChatConnection>,
  pub webrtc_stream: Stream<webrtc_stream::WebRTCConnection>,
  pub voice: Option<VoiceCall>,
  pub pending_rejoin: Option<PendingRejoin>,
  pub reconnect_seq: u32,
  pub chat_disconnected_since: Option<std::time::Instant>,
  pub active_server_id: Option<Uuid>,
  pub room_presence: HashMap<Uuid, Vec<DisplayVoiceUser>>,
  pub per_user_audio: HashMap<Uuid, UserAudioPref>,
  pub audio_cues: Option<AudioCues>,
  pub stashed_chat: Option<ChatModel>,
  pub window_state: WindowState,
}

pub enum Screen {
  Auth(AuthModel),
  Chat(ChatModel),
  Settings(SettingsModel),
}

#[derive(Clone)]
pub enum WindowState {
  Focused,
  NotFocused,
}

impl Default for Model {
  fn default() -> Self {
    Model {
      screen: Screen::Auth(AuthModel::new()),
      user: Auth::NotLoggedIn,
      chat_stream: Stream::Disconnected,
      webrtc_stream: Stream::Disconnected,
      voice: None,
      pending_rejoin: None,
      reconnect_seq: 0,
      chat_disconnected_since: None,
      active_server_id: None,
      room_presence: HashMap::new(),
      per_user_audio: crate::voice_settings::VoiceSettings::load().per_user_volumes,
      stashed_chat: None,
      window_state: WindowState::Focused,
      audio_cues: AudioCues::new(
        crate::voice_settings::VoiceSettings::load()
          .output_device
          .as_deref(),
      )
      .map(|mut cues| {
        cues.set_volume(0.1);
        cues
      })
      .map_err(|err| eprintln!("Warning: audio cues failed to initialize: {err:?}"))
      .ok(),
    }
  }
}
