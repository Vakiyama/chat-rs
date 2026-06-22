use crate::model::MediaHealth;
use crate::webrtc_stream::{
  CallSetup, CaptureEvent, WebRTCConnection, build_mic, build_speaker, setup_client,
};
use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::stats::{StatsReport, StatsReportType};

pub enum VoiceCommand {
  Join { voice_channel_id: Uuid, epoch: u32 },
  Leave,
  Signal(Box<ServerVoice>),
  SubscribeServer { server_id: Uuid },
  SetMuted(bool),
  SetDeafened(bool),
  SetNoiseGate(f32),
  SetInputDevice(Option<String>),
  SetOutputDevice(Option<String>),
}

pub struct VoiceHandle {
  sender: tokio::sync::mpsc::UnboundedSender<VoiceCommand>,
  pub receiver: Arc<Mutex<tokio::sync::mpsc::Receiver<crate::Message>>>,
}

impl VoiceHandle {
  pub fn join(&self, voice_channel_id: Uuid, epoch: u32) {
    let _ = self.sender.send(VoiceCommand::Join {
      voice_channel_id,
      epoch,
    });
  }
  pub fn leave(&self) {
    let _ = self.sender.send(VoiceCommand::Leave);
  }
  pub fn signal(&self, message: ServerVoice) {
    let _ = self.sender.send(VoiceCommand::Signal(message.into()));
  }
  pub fn subscribe_server(&self, server_id: Uuid) {
    let _ = self
      .sender
      .send(VoiceCommand::SubscribeServer { server_id });
  }
  pub fn set_muted(&self, muted: bool) {
    let _ = self.sender.send(VoiceCommand::SetMuted(muted));
  }
  pub fn set_deafened(&self, deafened: bool) {
    let _ = self.sender.send(VoiceCommand::SetDeafened(deafened));
  }
  pub fn set_noise_gate(&self, threshold: f32) {
    let _ = self.sender.send(VoiceCommand::SetNoiseGate(threshold));
  }
  pub fn set_input_device(&self, name: Option<String>) {
    let _ = self.sender.send(VoiceCommand::SetInputDevice(name));
  }
  pub fn set_output_device(&self, name: Option<String>) {
    let _ = self.sender.send(VoiceCommand::SetOutputDevice(name));
  }
}

struct ActiveCall {
  pc: Arc<RTCPeerConnection>,
  // held so the device keeps running; reassigned (old dropped) on a live swap.
  // `None` means that device failed to open — the call runs without it until the
  // user fixes/switches the device.
  mic: Option<cpal::Stream>,
  speaker: Option<cpal::Stream>,
  stats_task: tokio::task::JoinHandle<()>,
  mixer: crate::audio_processing::mixer::Mixer,
  in_rate: u32,
  out_rate: u32,
  started: Arc<Mutex<HashSet<String>>>,
  // kept so a live device swap can feed the running audio path without a rejoin.
  cap_tx: tokio::sync::mpsc::UnboundedSender<CaptureEvent>,
  rnd_tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>,
  // the model's epoch for this call, so live device-swap health reports can be
  // filtered against a stale call the same way join/connection callbacks are.
  epoch: u32,
}

// Worst loss fraction (0.0..1.0) the receiver reports for our outbound audio,
// via RTCP receiver reports relayed back as RemoteInboundRTP stats. This is the
// loss on the path our packets travel to the next hop, so it's the right signal
// for how much FEC redundancy our encoder should add.
fn outbound_loss(r: &StatsReport) -> Option<f64> {
  let mut worst: Option<f64> = None;
  for s in r.reports.values() {
    if let StatsReportType::RemoteInboundRTP(s) = s
      && s.kind == "audio"
    {
      worst = Some(worst.map_or(s.fraction_lost, |w: f64| w.max(s.fraction_lost)));
    }
  }
  worst
}

fn inbound_audio(r: &StatsReport) -> (u32, u64) {
  let (mut n, mut bytes) = (0u32, 0u64);
  for s in r.reports.values() {
    if let StatsReportType::InboundRTP(s) = s
      && s.kind == "audio"
    {
      n += 1;
      bytes += s.bytes_received;
    }
  }
  (n, bytes)
}

pub fn spawn_stats_poller(
  pc: Arc<RTCPeerConnection>,
  epoch: u32,
  tx: tokio::sync::mpsc::Sender<crate::Message>,
  capture_signal_frames: Arc<AtomicU64>,
  fec_loss_perc: Arc<AtomicU32>,
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut prev_bytes: Option<u64> = None;
    let mut flat_audio = 0u32;
    let mut last_emitted: Option<MediaHealth> = None;

    // adaptive FEC target. Rise fast to cover a loss burst, decay slowly so the
    // encoder doesn't drop redundancy the instant a gap clears (loss is bursty).
    // Capped so FEC overhead stays bounded even on a very lossy link.
    let mut fec_smoothed: f64 = 0.0;
    const FEC_MAX_PERC: f64 = 30.0;

    // mic liveness: `capture_signal_frames` only counts capture buffers that
    // carried real signal (peak above a digital-silence floor), so this catches
    // both a dead callback (no frames) AND a device that fires callbacks but
    // delivers pure silence (unplugged / OS-muted / wrong input). We require the
    // counter to stay flat for several consecutive ticks before flagging, so a
    // normal speech pause on a hardware-gated mic doesn't trip it; a real mic's
    // noise floor advances the counter every tick. Recovers immediately once
    // signal returns. Seeded to `true` so the healthy default isn't re-announced.
    let mut prev_signal_frames: Option<u64> = None;
    let mut silent_ticks = 0u32;
    const SILENT_TICKS_LIMIT: u32 = 3; // ~6s of digital silence before flagging
    let mut last_emitted_receiving: Option<bool> = Some(true);

    loop {
      tick.tick().await;
      if pc.connection_state() == RTCPeerConnectionState::Closed {
        break;
      }

      let frames = capture_signal_frames.load(Ordering::Relaxed);
      let advanced = prev_signal_frames.is_none_or(|p| frames > p);
      prev_signal_frames = Some(frames);
      silent_ticks = if advanced { 0 } else { silent_ticks + 1 };
      let receiving = silent_ticks < SILENT_TICKS_LIMIT;
      if last_emitted_receiving != Some(receiving) {
        last_emitted_receiving = Some(receiving);
        let _ = tx
          .send(crate::Message::VoiceMicActivity { epoch, receiving })
          .await;
      }

      let report = pc.get_stats().await;

      // adaptive FEC: map the receiver-reported loss on our outbound audio to an
      // Opus packet-loss-perc the encoder reads. Rise straight to the observed
      // loss, decay slowly between bursts.
      if let Some(loss) = outbound_loss(&report) {
        let target = (loss * 100.0).clamp(0.0, FEC_MAX_PERC);
        fec_smoothed = if target > fec_smoothed {
          target
        } else {
          fec_smoothed * 0.8 + target * 0.2
        };
        fec_loss_perc.store(fec_smoothed.round() as u32, Ordering::Relaxed);
      }

      let (audio_streams, bytes) = inbound_audio(&report);

      let first = prev_bytes.is_none();
      flat_audio = if audio_streams > 0 && prev_bytes.is_some_and(|p| bytes <= p) {
        flat_audio + 1
      } else {
        0
      };
      prev_bytes = Some(bytes);

      let health = if audio_streams > 0 && flat_audio >= 3 {
        MediaHealth::NoAudio
      } else if first {
        MediaHealth::Unknown
      } else {
        MediaHealth::Flowing
      };

      if last_emitted != Some(health) {
        last_emitted = Some(health);
        let _ = tx
          .send(crate::Message::VoiceMediaHealth { epoch, health })
          .await;
      }
    }
  })
}

pub fn spawn_voice(mut conn: WebRTCConnection) -> VoiceHandle {
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel::<crate::Message>(32);

  let room_id = Arc::new(Mutex::new(None));
  let cloned_room_id = room_id.clone();

  // mute/deafen persist across calls (like Discord); the audio pipeline reads these
  // atomics live, and we re-announce them to the server on each join.
  let muted = Arc::new(AtomicBool::new(false));
  let deafened = Arc::new(AtomicBool::new(false));

  // Persisted voice settings. The gate threshold is shared (read live by the
  // audio processor); device choices are owned by the actor and read at join
  // time, updated live by Set*Device. Loading here means saved settings take
  // effect the moment voice connects, before the settings screen is ever opened.
  let settings = crate::voice_settings::VoiceSettings::load();
  let gate_threshold = Arc::new(AtomicU32::new(settings.gate_threshold.to_bits()));

  tokio::spawn(async move {
    let mut call: Option<ActiveCall> = None; // None = not in a call
    let mut input_name: Option<String> = settings.input_device;
    let mut output_name: Option<String> = settings.output_device;
    while let Some(cmd) = rx.recv().await {
      match cmd {
        VoiceCommand::Join {
          voice_channel_id,
          epoch,
        } => {
          if let Some(active) = call.take() {
            active.stats_task.abort(); // stop the poller before tearing down
            let _ = active.pc.close().await;

            let mut room_id = cloned_room_id.lock().await;

            if let Some(room_id) = *room_id {
              conn.send(ClientVoice::LeaveRoom {
                voice_channel_id: room_id,
              });
            };

            *room_id = None;
          }
          match setup_client(
            voice_channel_id,
            conn.clone(),
            muted.clone(),
            deafened.clone(),
            gate_threshold.clone(),
            input_name.clone(),
            output_name.clone(),
          )
          .await
          {
            Ok(CallSetup {
              pc,
              offer,
              mic,
              speaker,
              mixer,
              in_rate,
              out_rate,
              cap_tx,
              rnd_tx,
              capture_signal_frames,
              fec_loss_perc,
            }) => {
              let pc = Arc::new(pc);
              let started = Arc::new(Mutex::new(HashSet::new()));
              let started_for_track = started.clone();
              let mixer_for_track = mixer.clone();
              // Single on_track handler that reads ALL inbound audio tracks
              // (both initial-answer tracks and renegotiated tracks) through the
              // same reader. We spawn read_track rather than awaiting it: the
              // reader runs an infinite read loop, so awaiting it inside the
              // handler future would block webrtc-rs's track dispatch. Dedup is
              // owned by read_track against `started`, so we do NOT pre-insert
              // here (doing so would make read_track's own insert see the id as
              // already present and bail immediately).
              pc.on_track(Box::new(move |track, _, _| {
                let started = started_for_track.clone();
                let mixer = mixer_for_track.clone();
                Box::pin(async move {
                  tokio::spawn(read_track(track, mixer, out_rate, started));
                })
              }));
              conn.send(ClientVoice::Offer {
                description: offer,
                voice_channel_id,
              });
              let _ = outbound_tx
                .send(crate::Message::JoinVoiceSuccessful { voice_channel_id })
                .await;
              // surface whether each direction actually came up so the UI can
              // tell the user they joined but have no mic and/or no speaker.
              let _ = outbound_tx
                .send(crate::Message::VoiceDeviceHealth {
                  epoch,
                  input_ok: mic.is_some(),
                  output_ok: speaker.is_some(),
                })
                .await;

              let cb_tx = outbound_tx.clone();
              pc.on_peer_connection_state_change(Box::new(move |new_state| {
                let cb_tx = cb_tx.clone();
                Box::pin(async move {
                  let _ = cb_tx
                    .send(crate::Message::VoiceHandlePeerConnectionChanged {
                      state: new_state,
                      epoch,
                    })
                    .await; // unbounded: no .await inside the callback
                })
              }));

              let stats_task = spawn_stats_poller(
                pc.clone(),
                epoch,
                outbound_tx.clone(),
                capture_signal_frames,
                fec_loss_perc,
              );
              call = Some(ActiveCall {
                pc,
                mic,
                speaker,
                stats_task,
                mixer,
                in_rate,
                out_rate,
                started: started.clone(),
                cap_tx,
                rnd_tx,
                epoch,
              });
              *cloned_room_id.lock().await = Some(voice_channel_id);

              // the server's participant starts un-muted/un-deafened, so re-announce
              // any persisted local state for this fresh join.
              if muted.load(Ordering::Relaxed) {
                conn.send(ClientVoice::SetMuted {
                  muted: true,
                  voice_channel_id,
                });
              }
              if deafened.load(Ordering::Relaxed) {
                conn.send(ClientVoice::SetDeafened {
                  deafened: true,
                  voice_channel_id,
                });
              }
            }
            Err(e) => eprintln!("voice join failed: {e:?}"),
          }
        }
        VoiceCommand::Signal(msg) => {
          let Some(active) = call.as_ref() else {
            continue;
          };
          if let Err(e) = apply_signal(
            &active.pc,
            &mut conn,
            *msg,
            &active.mixer,
            active.started.clone(),
            active.out_rate,
          )
          .await
          {
            eprintln!("apply signal: {e:?}");
          }
        }
        VoiceCommand::SubscribeServer { server_id } => {
          conn.send(ClientVoice::SubscribeServer { server_id });
        }
        VoiceCommand::SetMuted(value) => {
          // the audio pipeline reads this live; only announce to the server when
          // we're actually in a call.
          muted.store(value, Ordering::Relaxed);
          if let Some(voice_channel_id) = *cloned_room_id.lock().await {
            conn.send(ClientVoice::SetMuted {
              muted: value,
              voice_channel_id,
            });
          }
        }
        VoiceCommand::SetDeafened(value) => {
          deafened.store(value, Ordering::Relaxed);
          if let Some(voice_channel_id) = *cloned_room_id.lock().await {
            conn.send(ClientVoice::SetDeafened {
              deafened: value,
              voice_channel_id,
            });
          }
        }
        VoiceCommand::SetNoiseGate(threshold) => {
          // the audio processor reads this atomic live, so this takes effect
          // mid-call without touching the pipeline.
          gate_threshold.store(threshold.to_bits(), Ordering::Relaxed);
        }
        VoiceCommand::SetInputDevice(name) => {
          // remember for the next join, and hot-swap the mic if a call is live.
          input_name = name;
          if let Some(active) = call.as_mut() {
            match build_mic(active.cap_tx.clone(), input_name.as_deref()) {
              Ok((stream, rate)) => {
                // tell the processor to rebuild its resampler if the new device
                // runs at a different rate, then drop the old mic stream.
                if rate != active.in_rate {
                  let _ = active.cap_tx.send(CaptureEvent::Rate(rate));
                  active.in_rate = rate;
                }
                active.mic = Some(stream);
              }
              Err(e) => {
                // the new device failed: drop the old mic so we don't keep
                // publishing from a device the user just switched away from, and
                // report that we now have no working input.
                eprintln!("input device switch failed; mic now silent: {e:?}");
                active.mic = None;
              }
            }
            let _ = outbound_tx
              .send(crate::Message::VoiceDeviceHealth {
                epoch: active.epoch,
                input_ok: active.mic.is_some(),
                output_ok: active.speaker.is_some(),
              })
              .await;
          }
        }
        VoiceCommand::SetOutputDevice(name) => {
          // remember for the next join, and hot-swap the speaker if a call is live.
          output_name = name;
          // A seamless swap is only possible when the new device shares the
          // call's fixed output rate (mixer + per-peer resamplers are sized to
          // it). On a rate mismatch, fall back to a full rebuild via main's
          // epoch-aware rejoin path (brief audio blip).
          let mut rejoin_for: Option<Uuid> = None;
          let mut report_health = false;
          if let Some(active) = call.as_mut() {
            match build_speaker(
              active.mixer.clone(),
              active.rnd_tx.clone(),
              output_name.as_deref(),
            ) {
              Ok((stream, rate)) if rate == active.out_rate => {
                active.speaker = Some(stream); // drop old speaker
                report_health = true;
              }
              Ok((stream, _rate)) => {
                // rate mismatch → full rebuild via rejoin, which re-reports health.
                drop(stream);
                rejoin_for = *cloned_room_id.lock().await;
              }
              Err(e) => {
                // the new device failed: drop the old speaker and report that we
                // now have no working output (we hear nobody).
                eprintln!("output device switch failed; no speaker output: {e:?}");
                active.speaker = None;
                report_health = true;
              }
            }
            if report_health {
              let _ = outbound_tx
                .send(crate::Message::VoiceDeviceHealth {
                  epoch: active.epoch,
                  input_ok: active.mic.is_some(),
                  output_ok: active.speaker.is_some(),
                })
                .await;
            }
          }
          if let Some(voice_channel_id) = rejoin_for {
            eprintln!("output device rate differs from call; rebuilding audio path");
            let _ = outbound_tx
              .send(crate::Message::VoiceRejoinForSettings { voice_channel_id })
              .await;
          }
        }
        VoiceCommand::Leave => {
          if let Some(active) = call.take() {
            active.stats_task.abort(); // stop the poller before tearing down
            let _ = active.pc.close().await;
            let mut room_id = cloned_room_id.lock().await;

            if let Some(room_id) = *room_id {
              conn.send(ClientVoice::LeaveRoom {
                voice_channel_id: room_id,
              });
            };

            *room_id = None;
          }
        }
      }
    }
  });

  VoiceHandle {
    sender: tx,
    receiver: Mutex::new(outbound_rx).into(),
  }
}

async fn apply_signal(
  pc: &Arc<RTCPeerConnection>,
  conn: &mut WebRTCConnection,
  msg: ServerVoice,
  mixer: &crate::audio_processing::mixer::Mixer,
  started: Arc<Mutex<HashSet<String>>>,
  out_rate: u32,
) -> anyhow::Result<()> {
  match msg {
    ServerVoice::Answer { description, .. } => {
      let before_receivers = pc.get_receivers().await.len();
      pc.set_remote_description(description).await?;
      // on_track does NOT fire for tracks already present in the initial answer.
      // Manually enumerate receivers and start reading from them. Each track gets
      // its own spawned reader — awaiting here would block this function (and the
      // entire voice actor loop) on the first track's infinite read loop, so the
      // second/third participant would never be read.
      if before_receivers == 0 {
        for receiver in pc.get_receivers().await.iter() {
          let tracks = receiver.tracks().await;
          for track in tracks {
            if track.codec().capability.mime_type.starts_with("audio/") {
              tokio::spawn(read_track(track, mixer.clone(), out_rate, started.clone()));
            }
          }
        }
      }
    }
    ServerVoice::Offer {
      description,
      voice_channel_id,
    } => {
      pc.set_remote_description(description).await?;
      let ans = pc.create_answer(None).await?;
      pc.set_local_description(ans).await?;
      let local = pc
        .local_description()
        .await
        .ok_or_else(|| anyhow::anyhow!("no local description"))?;

      conn.send(ClientVoice::Answer {
        description: local,
        voice_channel_id,
      });
    }
    ServerVoice::PresenceSnapshot { .. } => (),
  }
  Ok(())
}

async fn read_track(
  track: Arc<webrtc::track::track_remote::TrackRemote>,
  mixer: crate::audio_processing::mixer::Mixer,
  out_rate: u32,
  started: Arc<Mutex<HashSet<String>>>,
) {
  let id = track.id().to_string();
  // Single source of dedup. track.id() is stable from SDP parse time, unlike
  // ssrc() which can be 0 until the first packet. If another reader already owns
  // this id, bail. The async Mutex makes this check-and-insert atomic across the
  // enumeration path and the on_track path racing on the same track.
  if !started.lock().await.insert(id.clone()) {
    return;
  }

  let mut out_rs = match crate::audio_processing::resampler::Resampler::new(
    crate::webrtc_stream::TARGET_RATE,
    out_rate,
  ) {
    Ok(r) => r,
    Err(e) => {
      eprintln!("Failed to create resampler for track {id}: {e}");
      return;
    }
  };
  let mut decoder =
    match audiopus::coder::Decoder::new(audiopus::SampleRate::Hz48000, audiopus::Channels::Mono) {
      Ok(d) => d,
      Err(e) => {
        eprintln!("Failed to create decoder for track {id}: {e}");
        return;
      }
    };
  let mut pcm = vec![0f32; 5760]; // 120ms @ 48k: handles oversized FEC/PLC frames

  // ssrc() may be 0 at spawn time (before the first RTP packet), which would
  // collide every enumerated track onto mixer slot 0. Resolve the mixer key
  // lazily from the wire header — the packet always carries the real ssrc.
  let mut src: Option<u32> = None;

  // loss concealment state. We watch RTP sequence numbers for gaps and fill them
  // before decoding the packet that landed, so the playback timeline stays
  // continuous (the mixer underruns — and clicks — only when the queue truly
  // empties). One 20ms Opus frame is 960 samples @ 48k mono.
  const FRAME: usize = 960;
  // cap how much we synthesize for a single gap. Opus PLC decays to near-silence
  // within a few frames anyway, and a large gap is usually an outage or the peer
  // muting (which stops their RTP) — flooding the mixer past this is pointless.
  const MAX_CONCEAL: u16 = 5; // ~100ms
  let mut last_seq: Option<u16> = None;

  let push_frame =
    |out_rs: &mut crate::audio_processing::resampler::Resampler, s: u32, samples: &[f32]| {
      let mut at_dev = Vec::new();
      let _ = out_rs.push(samples, &mut at_dev);
      mixer.push(s, &at_dev);
    };

  while let Ok((pkt, _)) = track.read_rtp().await {
    let seq = pkt.header.sequence_number;
    let s = *src.get_or_insert(pkt.header.ssrc);

    // drop late/duplicate packets (seq not strictly ahead of the last we used);
    // forward distance > half the seq space means it's actually behind (wrapped).
    if let Some(prev) = last_seq {
      let fwd = seq.wrapping_sub(prev);
      if fwd == 0 || fwd > 0x8000 {
        continue;
      }
    }

    if pkt.payload.is_empty() {
      // keep the timeline anchored on empty (e.g. padding) packets without
      // concealing — they aren't audio, but they aren't losses either.
      last_seq = Some(seq);
      continue;
    }

    // number of packets missing between the last one we decoded and this one.
    let gap = match last_seq {
      Some(prev) => seq.wrapping_sub(prev).wrapping_sub(1).min(MAX_CONCEAL),
      None => 0,
    };
    if gap > 0 {
      // the most recent lost frame can be reconstructed exactly: this packet
      // carries an in-band FEC (LBRR) copy of it. Conceal the older losses with
      // Opus PLC (generated silence/continuation), then FEC-recover the last one.
      for _ in 0..gap - 1 {
        if let Ok(n) = decoder.decode_float(None::<&[u8]>, &mut pcm[..FRAME], false) {
          push_frame(&mut out_rs, s, &pcm[..n]);
        }
      }
      match decoder.decode_float(Some(&pkt.payload[..]), &mut pcm[..FRAME], true) {
        Ok(n) => push_frame(&mut out_rs, s, &pcm[..n]),
        // no FEC data present for that frame → fall back to one more PLC frame.
        Err(_) => {
          if let Ok(n) = decoder.decode_float(None::<&[u8]>, &mut pcm[..FRAME], false) {
            push_frame(&mut out_rs, s, &pcm[..n]);
          }
        }
      }
    }

    match decoder.decode_float(Some(&pkt.payload[..]), &mut pcm, false) {
      Ok(n) => push_frame(&mut out_rs, s, &pcm[..n]),
      Err(e) => eprintln!("opus decode error: {e}"),
    }
    last_seq = Some(seq);
  }

  // Release the dedup key so this track id can be read again. Without this the set
  // leaks an entry per track for the lifetime of the call, and — because the server's
  // relay id is tied to the publisher — a peer that rejoins would be treated as
  // "already started" and never read again.
  started.lock().await.remove(&id);

  if let Some(s) = src {
    mixer.remove(s);
  }
}
