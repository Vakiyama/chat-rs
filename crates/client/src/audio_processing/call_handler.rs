use std::sync::Arc;

use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use tokio::sync::Mutex;
use uuid::Uuid;
use webrtc::peer_connection::RTCPeerConnection;

use crate::webrtc_stream::{WebRTCConnection, setup_client};

pub enum VoiceCommand {
  Join { voice_channel_id: Uuid },
  Leave,
  Signal(Box<ServerVoice>),
}

#[derive(Clone)]
pub struct VoiceHandle {
  sender: tokio::sync::mpsc::UnboundedSender<VoiceCommand>,
  pub room_id: Arc<Mutex<Option<Uuid>>>,
}

impl VoiceHandle {
  pub fn join(&self, voice_channel_id: Uuid) {
    let _ = self.sender.send(VoiceCommand::Join { voice_channel_id });
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
}

// TODO: need to rework this handler to emit messages into the update loop
// currently there's no easy way for the client to know if a call's pc is having issues
// connecting
// i'm worried about client/server state mismatch
// as the call binding below is its own source of truth

pub fn spawn_voice(mut conn: WebRTCConnection) -> VoiceHandle {
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  let room_id = Arc::new(Mutex::new(None));
  let cloned_room_id = room_id.clone();
  tokio::spawn(async move {
    let mut call: Option<ActiveCall> = None; // None = not in a call
    while let Some(cmd) = rx.recv().await {
      match cmd {
        VoiceCommand::Join { voice_channel_id } => {
          if let Some(active) = call.take() {
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
              conn.send(ClientVoice::Offer {
                description: offer,
                voice_channel_id,
              }); // actor sends its own offer
              call = Some(ActiveCall {
                pc: pc.into(),
                _mic: mic,
                _out: out,
              });
              let mut room_id = cloned_room_id.lock().await;
              *room_id = Some(voice_channel_id);
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
    room_id: room_id.clone(),
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
