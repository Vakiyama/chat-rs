use cpal_kira::traits::{DeviceTrait, HostTrait};
use kira::{
  AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
  backend::cpal::CpalBackendSettings,
  sound::static_sound::StaticSoundData,
  track::{TrackBuilder, TrackHandle},
};
use std::io::Cursor;

/// Find the cpal (0.17, kira's version) output device whose name matches the
/// saved choice. Returns `None` — meaning "let kira use the system default" —
/// when no name is given or the named device can't be found. Best-effort: any
/// enumeration error falls back to the default device too.
fn resolve_cue_device(name: Option<&str>) -> Option<cpal_kira::Device> {
  let name = name?;
  let host = cpal_kira::default_host();
  // Match on `description().name()`, NOT `name()`: the rest of the app saves a
  // device by its cpal 0.18 `Display` string, which is the human-readable
  // `description().name()`. cpal 0.17's `name()` returns the raw ALSA pcm_id
  // instead, which would never match the saved choice.
  host
    .output_devices()
    .ok()?
    .find(|dev| dev.description().map(|d| d.name() == name).unwrap_or(false))
}

/// Every presence event chat-rs emits a sound for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cue {
  Join,
  Leave,
  Mute,
  Unmute,
  Deafen,
  Undeafen,
  PeerJoin,
  PeerLeave,
}

/// Bake each WAV in at compile time. `from_cursor` decodes the in-memory bytes,
/// so there are no loose asset files to ship or lose.
macro_rules! load_cue {
  ($name:literal) => {
    StaticSoundData::from_cursor(Cursor::new(
      include_bytes!(concat!("assets/cues/variants/warm_synth/", $name, ".wav")).as_slice(),
    ))
  };
}
//013   fm_glass/
//014   sine_pad/
//012   soft_bell/
//010   warm_synth/

/// The decoded cue samples, independent of any output device. Decoded once and
/// reused across device rebuilds (kira reference-counts the sample data).
struct Samples {
  join: StaticSoundData,
  leave: StaticSoundData,
  mute: StaticSoundData,
  unmute: StaticSoundData,
  deafen: StaticSoundData,
  undeafen: StaticSoundData,
  peer_join: StaticSoundData,
  peer_leave: StaticSoundData,
}

impl Samples {
  fn load() -> Result<Self, Box<dyn std::error::Error>> {
    Ok(Self {
      join: load_cue!("join")?,
      leave: load_cue!("leave")?,
      mute: load_cue!("mute")?,
      unmute: load_cue!("unmute")?,
      deafen: load_cue!("deafen")?,
      undeafen: load_cue!("undeafen")?,
      peer_join: load_cue!("peer_join")?,
      peer_leave: load_cue!("peer_leave")?,
    })
  }
}

pub struct AudioCues {
  _manager: AudioManager<DefaultBackend>,
  track: TrackHandle,
  samples: Samples,
  // remembered so a device rebuild can re-apply the slider's volume.
  volume: f32,
}

impl AudioCues {
  /// Decodes all cues and binds playback to `output_device` (or the system
  /// default when `None`). Cheap to clone per-play afterwards.
  pub fn new(output_device: Option<&str>) -> Result<Self, Box<dyn std::error::Error>> {
    let samples = Samples::load()?;
    let (manager, track) = Self::build_engine(output_device)?;
    Ok(Self {
      track,
      _manager: manager,
      samples,
      volume: 1.0,
    })
  }

  /// Build a fresh manager + cue sub-track bound to the chosen device.
  fn build_engine(
    output_device: Option<&str>,
  ) -> Result<(AudioManager<DefaultBackend>, TrackHandle), Box<dyn std::error::Error>> {
    let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings {
      backend_settings: CpalBackendSettings {
        device: resolve_cue_device(output_device),
        ..Default::default()
      },
      ..Default::default()
    })?;
    // Dedicated sub-track: set its volume from a settings slider without
    // touching the call audio path.
    let track = manager.add_sub_track(TrackBuilder::new())?;
    Ok((manager, track))
  }

  /// Re-bind playback to a (possibly newly working or newly chosen) output
  /// device. Keeps the already-decoded samples and re-applies the saved volume.
  /// Used when the user picks a different output device, and to recover when
  /// cues were dead at startup but a usable device has since appeared.
  pub fn rebuild(&mut self, output_device: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let (manager, track) = Self::build_engine(output_device)?;
    self._manager = manager;
    self.track = track;
    self.set_volume(self.volume);
    Ok(())
  }

  /// Fire a cue. Overlapping calls (two peers joining at once) layer cleanly;
  /// kira mixes them on its own audio thread, so this never blocks `update`.
  pub fn play(&mut self, cue: Cue) {
    let data = match cue {
      Cue::Join => &self.samples.join,
      Cue::Leave => &self.samples.leave,
      Cue::Mute => &self.samples.mute,
      Cue::Unmute => &self.samples.unmute,
      Cue::Deafen => &self.samples.deafen,
      Cue::Undeafen => &self.samples.undeafen,
      Cue::PeerJoin => &self.samples.peer_join,
      Cue::PeerLeave => &self.samples.peer_leave,
    };
    // Errors here mean the audio device vanished mid-session — log loudly,
    // don't unwrap. A missing ding should never take down a call.
    if let Err(e) = self.track.play(data.clone()) {
      tracing::warn!(?cue, error = %e, "failed to play audio cue");
    }
  }

  /// 0.0..=1.0 from a settings slider. Maps to decibels (-inf .. 0).
  pub fn set_volume(&mut self, linear: f32) {
    self.volume = linear;
    let db = if linear <= 0.0 {
      Decibels::SILENCE
    } else {
      Decibels(20.0 * linear.clamp(0.0, 1.0).log10())
    };
    self.track.set_volume(db, Tween::default());
  }
}
