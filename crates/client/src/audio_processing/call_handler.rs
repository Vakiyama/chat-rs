use std::sync::Arc;

use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use webrtc::peer_connection::RTCPeerConnection;

use crate::webrtc_stream::{WebRTCConnection, setup_client};

pub enum VoiceCommand {
  Join,
  Leave,
  Signal(Box<ServerVoice>),
}

#[derive(Clone)]
pub struct VoiceHandle(tokio::sync::mpsc::UnboundedSender<VoiceCommand>);

impl VoiceHandle {
  pub fn join(&self) {
    let _ = self.0.send(VoiceCommand::Join);
  }
  pub fn leave(&self) {
    let _ = self.0.send(VoiceCommand::Leave);
  }
  pub fn signal(&self, message: ServerVoice) {
    let _ = self.0.send(VoiceCommand::Signal(message.into()));
  }
}

struct ActiveCall {
  pc: Arc<RTCPeerConnection>,
  _mic: cpal::Stream,
  _out: cpal::Stream,
}

pub fn spawn_voice(mut conn: WebRTCConnection) -> VoiceHandle {
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(async move {
    let mut call: Option<ActiveCall> = None; // None = not in a call
    while let Some(cmd) = rx.recv().await {
      // serial, in order
      match cmd {
        VoiceCommand::Join => {
          if call.is_some() {
            continue;
          }
          match setup_client().await {
            Ok((pc, offer, mic, out)) => {
              conn.send(ClientVoice::Offer(offer)); // actor sends its own offer
              call = Some(ActiveCall {
                pc: pc.into(),
                _mic: mic,
                _out: out,
              });
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
          }
        }
      }
    }
  });
  VoiceHandle(tx)
}

async fn apply_signal(
  pc: &RTCPeerConnection,
  conn: &mut WebRTCConnection,
  msg: ServerVoice,
) -> anyhow::Result<()> {
  match msg {
    ServerVoice::Answer(sdp) => {
      pc.set_remote_description(sdp).await?;
    }
    ServerVoice::Offer(sdp) => {
      pc.set_remote_description(sdp).await?;
      let ans = pc.create_answer(None).await?;
      pc.set_local_description(ans).await?;
      let local = pc
        .local_description()
        .await
        .ok_or_else(|| anyhow::anyhow!("no local description"))?;
      conn.send(ClientVoice::Answer(local));
    }
  }
  Ok(())
}
