use chat_rs::shared::{
  convert::{IntoProto, stream::proto::ServerVoiceMessage},
  domain::stream::ServerVoice,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;
use webrtc::{
  api::{
    APIBuilder, interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
  },
  ice_transport::ice_server::RTCIceServer,
  interceptor::registry::Registry,
  peer_connection::{
    RTCPeerConnection, configuration::RTCConfiguration,
    sdp::session_description::RTCSessionDescription,
  },
  rtp_transceiver::rtp_codec::RTPCodecType,
  track::track_local::{TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP},
};

pub type PeerId = Uuid;

struct Participant {
  pc: Arc<RTCPeerConnection>,
  relay: RwLock<Option<Arc<TrackLocalStaticRTP>>>,
  signal_tx: tokio::sync::mpsc::Sender<ServerVoiceMessage>,
}
struct Room {
  peers: RwLock<HashMap<PeerId, Arc<Participant>>>,
}

async fn handle(
  offer: RTCSessionDescription,
  room: Arc<Room>,
  me: PeerId,
  signal_tx: tokio::sync::mpsc::Sender<ServerVoiceMessage>,
) -> anyhow::Result<RTCSessionDescription> {
  // Create a MediaEngine object to configure the supported codec
  let mut media_engine = MediaEngine::default();

  media_engine.register_default_codecs()?;

  // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
  // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
  // this is enabled by default. If you are manually managing You MUST create a InterceptorRegistry
  // for each PeerConnection.
  let mut registry = Registry::new();

  registry = register_default_interceptors(registry, &mut media_engine)?;

  let api = APIBuilder::new()
    .with_media_engine(media_engine)
    .with_interceptor_registry(registry)
    .build();

  let config = RTCConfiguration {
    ice_servers: vec![RTCIceServer {
      urls: vec!["stun:stun.l.google.com:19302".to_owned()],
      ..Default::default()
    }],
    ..Default::default()
  };

  let peer_connection = Arc::new(api.new_peer_connection(config).await?);

  peer_connection
    .add_transceiver_from_kind(RTPCodecType::Audio, None)
    .await?;

  let participant = Participant {
    pc: peer_connection.clone(),
    relay: RwLock::new(None),
    signal_tx,
  };

  room.peers.write().await.insert(me, participant.into());

  let track_room = room.clone();

  peer_connection.on_track(Box::new(move |track, _, _| {
    let room = track_room.clone();
    Box::pin(async move {
      // 1. this publisher's relay track (Opus capability from the incoming track)
      let relay = Arc::new(TrackLocalStaticRTP::new(
        track.codec().capability,
        format!("audio-{me}"),
        format!("relay-{me}"),
      ));

      // 2. register it, then add it to everyone already here
      let me_p = room.peers.read().await.get(&me).cloned();
      if let Some(me_p) = me_p {
        *me_p.relay.write().await = Some(relay.clone());
      }

      let others: Vec<Arc<Participant>> = {
        let peers = room.peers.read().await;
        peers
          .iter()
          .filter(|(id, _)| **id != me)
          .map(|(_, p)| Arc::clone(p))
          .collect()
      };

      for peer in others {
        let sender = peer
          .pc
          .add_track(relay.clone())
          .await
          .map_err(|err| eprintln!("error adding track to peer pc in others: {err:?}"));

        if let Ok(sender) = sender {
          tokio::spawn(async move {
            let mut buf = vec![0u8; 1500];
            while sender.read(&mut buf).await.is_ok() {}
          });

          let _ = renegotiate(&peer)
            .await
            .map_err(|err| eprintln!("renegotiate error: {err:?}"));
        }
      }

      tokio::spawn(async move {
        while let Ok((rtp, _)) = track.read_rtp().await {
          if relay.write_rtp(&rtp).await.is_err() {
            break;
          }
        }
      });
    })
  }));

  let existing: Vec<Arc<Participant>> = {
    let peers = room.peers.read().await;
    peers
      .iter()
      .filter(|(id, _)| **id != me)
      .map(|(_, p)| Arc::clone(p))
      .collect()
  };

  for peer in existing {
    if let Some(relay) = peer.relay.read().await.clone() {
      let sender = peer_connection.add_track(relay).await?;
      tokio::spawn(async move {
        let mut buf = vec![0u8; 1500];
        while sender.read(&mut buf).await.is_ok() {}
      });
    }
  }

  // negotiate, return the answer
  peer_connection.set_remote_description(offer).await?;
  let answer = peer_connection.create_answer(None).await?;
  let mut gather = peer_connection.gathering_complete_promise().await;
  peer_connection.set_local_description(answer).await?;
  let _ = gather.recv().await;
  peer_connection
    .local_description()
    .await
    .ok_or_else(|| anyhow::anyhow!("no local description"))
}

async fn renegotiate(peer: &Participant) -> anyhow::Result<()> {
  let offer = peer.pc.create_offer(None).await?;
  peer.pc.set_local_description(offer).await?;
  let local = peer
    .pc
    .local_description()
    .await
    .ok_or_else(|| anyhow::anyhow!("no local description"))?;
  peer
    .signal_tx
    .send(ServerVoice::Offer(local).into_proto())
    .await?; // push to that peer's stream
  Ok(())
}
