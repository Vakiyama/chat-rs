use crate::audio_processing::{call_handler::VoiceHandle, cues::AudioCues};
use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;
use crate::screens::settings::Model as SettingsModel;
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
  // local mic/playback toggles. Deafen implies mute. Persist across calls.
  pub muted: bool,
  pub deafened: bool,
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
  pub active_server_id: Option<Uuid>,
  pub room_presence: HashMap<Uuid, Vec<DisplayVoiceUser>>,
  pub audio_cues: Option<AudioCues>,
}

pub enum Screen {
  Auth(AuthModel),
  Chat(ChatModel),
  Settings(SettingsModel),
}

impl Default for Model {
  fn default() -> Self {
    Model {
      screen: Screen::Auth(AuthModel::new()),
      user: Auth::NotLoggedIn,
      chat_stream: Stream::Disconnected,
      webrtc_stream: Stream::Disconnected,
      voice: None,
      active_server_id: None,
      room_presence: HashMap::new(),
      audio_cues: AudioCues::new()
        .map(|mut cues| {
          cues.set_volume(0.1);
          cues
        })
        .map_err(|err| eprintln!("Warning: audio cues failed to initialize: {err:?}"))
        .ok(),
    }
  }
}
