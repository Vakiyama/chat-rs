//! Desktop persistence for [`VoiceSettings`]: a JSON file under the platform
//! config dir. The data types and the [`VoiceSettingsStore`] trait live in
//! `chat_core`; android provides its own store.

use chat_core::voice_settings::{VoiceSettings, VoiceSettingsStore};
use std::path::PathBuf;

pub struct FileVoiceSettingsStore;

fn settings_path() -> Option<PathBuf> {
  let mut dir = dirs::config_dir()?;
  dir.push("chat-rs");
  dir.push("voice_settings.json");
  Some(dir)
}

impl VoiceSettingsStore for FileVoiceSettingsStore {
  /// Read settings from disk, falling back to defaults on any error (missing
  /// file, unreadable, or malformed) so a bad file never blocks startup.
  fn load(&self) -> VoiceSettings {
    let Some(path) = settings_path() else {
      return VoiceSettings::default();
    };
    match std::fs::read_to_string(&path) {
      Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("voice settings: malformed {path:?}, using defaults: {e}");
        VoiceSettings::default()
      }),
      Err(_) => VoiceSettings::default(), // not yet written
    }
  }

  /// Persist settings, creating the config directory if needed. Errors are
  /// logged but never surfaced — failing to persist must not break the call.
  fn save(&self, settings: &VoiceSettings) {
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
    match serde_json::to_string_pretty(settings) {
      Ok(json) => {
        if let Err(e) = std::fs::write(&path, json) {
          eprintln!("voice settings: failed to write {path:?}: {e}");
        }
      }
      Err(e) => eprintln!("voice settings: failed to serialize: {e}"),
    }
  }
}
