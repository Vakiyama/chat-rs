use crate::model::MediaHealth;
use crate::webrtc_stream::{WebRTCConnection, setup_client};
use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use std::sync::Arc;
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
}

struct ActiveCall {
  pc: Arc<RTCPeerConnection>,
  _mic: cpal::Stream,
  _out: cpal::Stream,
  stats_task: tokio::task::JoinHandle<()>,
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
            let _ = active.pc.close().await;

            let mut room_id = cloned_room_id.lock().await;

            if let Some(room_id) = *room_id {
              conn.send(ClientVoice::LeaveRoom {
                voice_channel_id: room_id,
              });
            };

            *room_id = None;
          }
          match setup_client().await {
            Ok((pc, offer, mic, out)) => {
              let pc = Arc::new(pc);
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
              });
              *cloned_room_id.lock().await = Some(voice_channel_id);
            }
            Err(e) => eprintln!("voice join failed: {e:?}"),
          }
        }
        VoiceCommand::Signal(msg) => {
          let Some(active) = call.as_ref() else {
            continue;
          };
          if let Err(e) = apply_signal(&active.pc, &mut conn, *msg).await {
            eprintln!("apply signal: {e:?}");
          }
        }
        VoiceCommand::Leave => {
          if let Some(active) = call.take() {
            active.stats_task.abort(); // stop the poller before tearing down
            let _ = active.pc.close().await;
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
  pc: &RTCPeerConnection,
  conn: &mut WebRTCConnection,
  msg: ServerVoice,
) -> anyhow::Result<()> {
  match msg {
    ServerVoice::Answer { description, .. } => {
      pc.set_remote_description(description).await?;
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
  }
  Ok(())
}
