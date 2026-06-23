use crate::audio_processing::cues::AudioCues;
use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;
use crate::screens::settings::Model as SettingsModel;
use crate::voice_settings::FileVoiceSettingsStore;
use chat_core::voice::{MediaHealth, VoiceHandle};
use chat_core::voice_settings::{UserAudioPref, VoiceSettingsStore};
use chat_shared::domain::stream::{DisplayVoiceUser, User};
use std::collections::HashMap;
use uuid::Uuid;

use crate::chat_stream;

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
  // per-device audio health, reported by the voice actor on join and on every
  // live device swap. `false` means we joined the call but that direction is
  // dead until the user fixes the device: no `input_ok` → nobody hears us, no
  // `output_ok` → we hear nobody. Both default true until proven otherwise.
  pub input_ok: bool,
  pub output_ok: bool,
  // whether mic capture frames are actually arriving. A mic device can open
  // successfully (`input_ok`) yet deliver nothing — unplugged, OS-muted, or
  // broken — in which case this flips false and we surface the same "no mic"
  // warning. Defaults true so a fresh join doesn't flash a warning before the
  // first frames land.
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
  pub webrtc_stream: Stream<chat_core::rtc::WebRTCConnection>,
  pub voice: Option<VoiceCall>,
  pub active_server_id: Option<Uuid>,
  pub room_presence: HashMap<Uuid, Vec<DisplayVoiceUser>>,
  // Per-remote-user playback levels set from the in-call right-click mixer,
  // keyed by user id. Mirrors the persisted `VoiceSettings::per_user_volumes`:
  // the UI reads it to render each slider/mute state and rewrites it (and the
  // file) on change, while the live gain is pushed to the voice actor.
  pub per_user_audio: HashMap<Uuid, UserAudioPref>,
  pub audio_cues: Option<AudioCues>,
  // the chat model kept alive while the settings screen is showing, so returning
  // restores it instantly instead of rebuilding + re-fetching everything. It also
  // keeps receiving background updates (posts, presence) while away.
  pub stashed_chat: Option<ChatModel>,
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
      per_user_audio: FileVoiceSettingsStore.load().per_user_volumes,
      stashed_chat: None,
      // Bind cues to the saved output device so they share the device the call
      // uses. If this fails (e.g. no working device at startup) we leave it None
      // and recover later via AudioCues::rebuild when a device appears/changes.
      audio_cues: AudioCues::new(FileVoiceSettingsStore.load().output_device.as_deref())
        .map(|mut cues| {
          cues.set_volume(0.1);
          cues
        })
        .map_err(|err| eprintln!("Warning: audio cues failed to initialize: {err:?}"))
        .ok(),
    }
  }
}
