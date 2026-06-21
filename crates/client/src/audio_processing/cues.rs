use kira::{
  AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
  sound::static_sound::StaticSoundData,
  track::{TrackBuilder, TrackHandle},
};
use std::io::Cursor;

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

pub struct AudioCues {
  _manager: AudioManager<DefaultBackend>,
  track: TrackHandle,
  join: StaticSoundData,
  leave: StaticSoundData,
  mute: StaticSoundData,
  unmute: StaticSoundData,
  deafen: StaticSoundData,
  undeafen: StaticSoundData,
  peer_join: StaticSoundData,
  peer_leave: StaticSoundData,
}

impl AudioCues {
  /// Decodes all cues up front. Cheap to clone per-play afterwards (kira
  /// reference-counts the sample data, so playing never re-allocates).
  pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
    let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())?;
    // Dedicated sub-track: set its volume from a settings slider without
    // touching the call audio path.
    let track = manager.add_sub_track(TrackBuilder::new())?;

    Ok(Self {
      join: load_cue!("join")?,
      leave: load_cue!("leave")?,
      mute: load_cue!("mute")?,
      unmute: load_cue!("unmute")?,
      deafen: load_cue!("deafen")?,
      undeafen: load_cue!("undeafen")?,
      peer_join: load_cue!("peer_join")?,
      peer_leave: load_cue!("peer_leave")?,
      track,
      _manager: manager,
    })
  }

  /// Fire a cue. Overlapping calls (two peers joining at once) layer cleanly;
  /// kira mixes them on its own audio thread, so this never blocks `update`.
  pub fn play(&mut self, cue: Cue) {
    let data = match cue {
      Cue::Join => &self.join,
      Cue::Leave => &self.leave,
      Cue::Mute => &self.mute,
      Cue::Unmute => &self.unmute,
      Cue::Deafen => &self.deafen,
      Cue::Undeafen => &self.undeafen,
      Cue::PeerJoin => &self.peer_join,
      Cue::PeerLeave => &self.peer_leave,
    };
    // Errors here mean the audio device vanished mid-session — log loudly,
    // don't unwrap. A missing ding should never take down a call.
    if let Err(e) = self.track.play(data.clone()) {
      tracing::warn!(?cue, error = %e, "failed to play audio cue");
    }
  }

  /// 0.0..=1.0 from a settings slider. Maps to decibels (-inf .. 0).
  pub fn set_volume(&mut self, linear: f32) {
    let db = if linear <= 0.0 {
      Decibels::SILENCE
    } else {
      Decibels(20.0 * linear.clamp(0.0, 1.0).log10())
    };
    self.track.set_volume(db, Tween::default());
  }
}

// --- Iced wiring sketch ------------------------------------------------------
//
// struct ChatApp { cues: AudioCues, /* ... */ }
//
// In your update():
//   match message {
//       Message::JoinedCall      => self.cues.play(Cue::Join),
//       Message::LeftCall        => self.cues.play(Cue::Leave),
//       Message::ToggledMute(on) => self.cues.play(if on { Cue::Mute } else { Cue::Unmute }),
//       Message::ToggledDeafen(on)=> self.cues.play(if on { Cue::Deafen } else { Cue::Undeafen }),
//       Message::PeerJoined(_)   => self.cues.play(Cue::PeerJoin),
//       Message::PeerLeft(_)     => self.cues.play(Cue::PeerLeave),
//       Message::CueVolume(v)    => self.cues.set_volume(v),
//       // ...
//   }
//
// Build cues once (e.g. in your app's `new`); if `AudioCues::new()` fails
// (no audio device), keep it as Option and skip cues rather than refusing to start.
