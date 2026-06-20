use crate::audio_processing::call_handler::VoiceHandle;
use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;
use chat_shared::domain::stream::User;
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
}

pub enum Screen {
  Auth(AuthModel),
  Chat(ChatModel),
}

impl Default for Model {
  fn default() -> Self {
    Model {
      screen: Screen::Auth(AuthModel::new()),
      user: Auth::NotLoggedIn,
      chat_stream: Stream::Disconnected,
      webrtc_stream: Stream::Disconnected,
      voice: None,
    }
  }
}
