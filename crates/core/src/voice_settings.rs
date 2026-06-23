//! Persisted voice settings (noise gate + device choices + per-user levels).
//!
//! The data lives in core; persistence is a platform concern behind
//! [`VoiceSettingsStore`] so the desktop can use a config-dir file while android
//! uses its own store. The voice actor loads settings when a call's audio path
//! is built, so saved settings take effect the moment voice connects.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Remembered playback preference for one remote user, set from the in-call
/// right-click mixer. `volume` is a linear multiplier (`1.0` = unchanged, up to
/// `2.0` = +6 dB); `muted` silences without losing the remembered volume.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct UserAudioPref {
  pub volume: f32,
  pub muted: bool,
}

impl Default for UserAudioPref {
  fn default() -> Self {
    Self {
      volume: 1.0,
      muted: false,
    }
  }
}

impl UserAudioPref {
  /// The single linear gain the audio path applies: muted collapses to silence
  /// while keeping `volume` intact for when the user un-mutes.
  pub fn effective_gain(&self) -> f32 {
    if self.muted { 0.0 } else { self.volume }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
  /// RMS gate threshold in 0.0..=1.0. `0.0` disables the gate (always open).
  pub gate_threshold: f32,
  /// Preferred input device name, or `None` for the system default.
  pub input_device: Option<String>,
  /// Preferred output device name, or `None` for the system default.
  pub output_device: Option<String>,
  /// Per-remote-user playback levels set from the in-call mixer, keyed by user
  /// id. Absent users play at unity. Empty by default; persisted across runs.
  pub per_user_volumes: HashMap<Uuid, UserAudioPref>,
}

impl Default for VoiceSettings {
  fn default() -> Self {
    Self {
      gate_threshold: 0.008, // 30%
      input_device: None,
      output_device: None,
      per_user_volumes: HashMap::new(),
    }
  }
}

/// Platform persistence for [`VoiceSettings`]. Both ends fall back to defaults
/// rather than erroring so a missing or bad store never blocks a call: `load`
/// returns defaults on any failure and `save` is best-effort.
pub trait VoiceSettingsStore: Send + Sync {
  fn load(&self) -> VoiceSettings;
  fn save(&self, settings: &VoiceSettings);
}
