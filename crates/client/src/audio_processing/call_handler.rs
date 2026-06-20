use crate::Message;
use crate::model::MediaHealth;
use crate::webrtc_stream::{WebRTCConnection, setup_client};
use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use iced::Task;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

struct ActiveCall {
  pc: Arc<RTCPeerConnection>,
  _mic: cpal::Stream,
  _out: cpal::Stream,
  stats_task: tokio::task::JoinHandle<()>,
  mixer: crate::audio_processing::mixer::Mixer,
  out_rate: u32,
  started: Arc<Mutex<HashSet<String>>>,
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
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut prev_bytes: Option<u64> = None;
    let mut flat_audio = 0u32;
    let mut last_emitted: Option<MediaHealth> = None;

    loop {
      tick.tick().await;
      if pc.connection_state() == RTCPeerConnectionState::Closed {
        break;
      }

      let report = pc.get_stats().await;
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

  tokio::spawn(async move {
    let mut call: Option<ActiveCall> = None; // None = not in a call
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
          )
          .await
          {
            Ok((pc, offer, mic, out, mixer, out_rate)) => {
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

              let stats_task = spawn_stats_poller(pc.clone(), epoch, outbound_tx.clone());
              call = Some(ActiveCall {
                pc,
                _mic: mic,
                _out: out,
                stats_task,
                mixer,
                out_rate,
                started: started.clone(),
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
    ServerVoice::PresenceSnapshot {
      voice_channel_id,
      peers,
      ..
    } => (),
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

  while let Ok((pkt, _)) = track.read_rtp().await {
    if pkt.payload.is_empty() {
      continue;
    }
    let s = *src.get_or_insert(pkt.header.ssrc);
    match decoder.decode_float(Some(&pkt.payload[..]), &mut pcm, false) {
      Ok(n) => {
        let mut at_dev = Vec::new();
        let _ = out_rs.push(&pcm[..n], &mut at_dev);
        mixer.push(s, &at_dev);
      }
      Err(e) => eprintln!("opus decode error: {e}"),
    }
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
