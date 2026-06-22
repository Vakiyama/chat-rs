//! Persisted voice settings (noise gate + device choices).
//!
//! These live on disk so the gate threshold and chosen input/output devices
//! survive restarts. The voice actor loads them when a call's audio path is
//! built, so saved settings take effect the moment voice connects — the UI just
//! mirrors and rewrites this file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
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

fn settings_path() -> Option<PathBuf> {
  let mut dir = dirs::config_dir()?;
  dir.push("chat-rs");
  dir.push("voice_settings.json");
  Some(dir)
}

impl VoiceSettings {
  /// Read settings from disk, falling back to defaults on any error (missing
  /// file, unreadable, or malformed) so a bad file never blocks startup.
  pub fn load() -> Self {
    let Some(path) = settings_path() else {
      return Self::default();
    };
    match std::fs::read_to_string(&path) {
      Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("voice settings: malformed {path:?}, using defaults: {e}");
        Self::default()
      }),
      Err(_) => Self::default(), // not yet written
    }
  }

  /// Persist settings, creating the config directory if needed. Errors are
  /// logged but never surfaced — failing to persist must not break the call.
  pub fn save(&self) {
    let Some(path) = settings_path() else {
      eprintln!("voice settings: no config dir available; not persisting");
      return;
    };
    if let Some(parent) = path.parent()
      && let Err(e) = std::fs::create_dir_all(parent)
    {
      eprintln!("voice settings: failed to create {parent:?}: {e}");
      return;
    }
    match serde_json::to_string_pretty(self) {
      Ok(json) => {
        if let Err(e) = std::fs::write(&path, json) {
          eprintln!("voice settings: failed to write {path:?}: {e}");
        }
      }
      Err(e) => eprintln!("voice settings: failed to serialize: {e}"),
    }
  }
}
