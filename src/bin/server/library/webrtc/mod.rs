use chat_rs::{
  config::CONFIG,
  shared::{
    convert::{IntoProto, stream::proto::ServerVoiceMessage},
    domain::stream::ServerVoice,
  },
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;
use webrtc::{
  api::{
    APIBuilder, interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
    setting_engine::SettingEngine,
  },
  ice::{
    udp_mux::{UDPMux, UDPMuxDefault, UDPMuxParams},
    udp_network::UDPNetwork,
  },
  ice_transport::{ice_candidate_type::RTCIceCandidateType, ice_server::RTCIceServer},
  interceptor::registry::Registry,
  peer_connection::{
    RTCPeerConnection, configuration::RTCConfiguration,
    sdp::session_description::RTCSessionDescription,
  },
  rtp_transceiver::rtp_codec::RTPCodecType,
  track::track_local::{TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP},
};

pub type PeerId = Uuid;

pub struct Participant {
  pub pc: Arc<RTCPeerConnection>,
  relay: RwLock<Option<Arc<TrackLocalStaticRTP>>>,
  signal_tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
  // todo: simultaneous renegs can cause issues, potential solution is an atomic bool below for
  // queuing renegs when we hit this condition.
  // defer negotiating when flag is already active:

  // renegotiate becomes: if negotiating is set, set needs_renegotiation and return (the wish is queued, not lost). Otherwise set negotiating, send the offer. In handle_answer, after applying: clear negotiating, and if needs_renegotiation was set, clear it and renegotiate once more — the fresh offer naturally includes all track changes accumulated since, so N queued requests collapse into one round. This is the standard SFU pattern. Two joins in the same instant is rare enough that I'd do fix 1 today, write Race 2 down in a comment at the renegotiate call site, and implement the flag pair when you wire up leave handling (which adds remove_track → more renegotiation triggers → the window widens).

  //  negotiating: AtomicBool,   // an offer is in flight
  //  needs_renegotiation: AtomicBool,
}

#[derive(Default)]
pub struct Room {
  pub peers: RwLock<HashMap<PeerId, Arc<Participant>>>,
}

pub async fn handle_offer(
  offer: RTCSessionDescription,
  room: Arc<Room>,
  me: PeerId,
  signal_tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
) -> anyhow::Result<RTCSessionDescription> {
  // Create a_client MediaEngine object to configure the supported codec
  let mut media_engine = MediaEngine::default();

  media_engine.register_default_codecs()?;

  // Create a_client InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
  // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
  // this is enabled by default. If you are manually managing You MUST create a_client InterceptorRegistry
  // for each PeerConnection.
  let mut registry = Registry::new();

  registry = register_default_interceptors(registry, &mut media_engine)?;

  let mut api = APIBuilder::new()
    .with_media_engine(media_engine)
    .with_interceptor_registry(registry);

  if let Some(public_ip) = CONFIG.server.public_ip.clone()
    && let Some(udp_port) = CONFIG.server.udp_port.clone()
  {
    let mut settings_engine = SettingEngine::default();

    settings_engine.set_nat_1to1_ips(vec![public_ip], RTCIceCandidateType::Host);
    settings_engine.set_udp_network(UDPNetwork::Muxed(UDPMuxDefault::new(UDPMuxParams::new(
      tokio::net::UdpSocket::bind(format!("0.0.0.0:{udp_port}"))
        .await
        .unwrap(),
    ))));

    api = api.with_setting_engine(settings_engine);
  }

  let api = api.build();

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

  let track_room = room.clone();

  peer_connection.on_track(Box::new(move |track, _, _| {
    println!("started audio track on peer connection");
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

          // todo: simultaneous renegs can cause issues, potential solution is an atomic bool below for
          // queuing renegs when we hit this condition.
          // see Participant struct def for potential fix
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

  // negotations done, safe to insert into room struct
  room.peers.write().await.insert(me, participant.into());

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
    .send(Ok(ServerVoice::Offer(local).into_proto()))
    .await?; // push to that peer's stream
  Ok(())
}

pub async fn handle_answer(
  room: Arc<Room>,
  me: PeerId,
  answer: RTCSessionDescription,
) -> anyhow::Result<()> {
  let peer = room
    .peers
    .read()
    .await
    .get(&me)
    .cloned()
    .ok_or_else(|| anyhow::anyhow!("received rtc answer for unknown peer"))?;

  peer.pc.set_remote_description(answer).await?;

  Ok(())
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use chat_rs::shared::{convert::TryIntoDomain, domain::stream::ServerVoice};
  use uuid::Uuid;
  use webrtc::{
    api::{
      APIBuilder,
      interceptor_registry::register_default_interceptors,
      media_engine::{MIME_TYPE_OPUS, MediaEngine},
    },
    interceptor::registry::Registry,
    media::Sample,
    peer_connection::{
      RTCPeerConnection, configuration::RTCConfiguration,
      peer_connection_state::RTCPeerConnectionState,
    },
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::track_local_static_sample::TrackLocalStaticSample,
  };

  use crate::library::webrtc::{PeerId, Room, handle_answer, handle_offer};

  // helper waits for relay in room for peer id
  async fn wait_for_relay(room: &Arc<Room>, id: PeerId) -> anyhow::Result<()> {
    for _ in 0..100 {
      if let Some(p) = room.peers.read().await.get(&id)
        && p.relay.read().await.is_some()
      {
        return Ok(());
      }
      tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    anyhow::bail!("relay for {id} never appeared")
  }

  // creates a_client localhost only client peer connection
  async fn make_client_pc() -> anyhow::Result<Arc<RTCPeerConnection>> {
    let mut media_engine = MediaEngine::default();

    media_engine.register_default_codecs()?;
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
    let api = APIBuilder::new()
      .with_media_engine(media_engine)
      .with_interceptor_registry(registry)
      .build();

    // no ice servers keeps tests to localhost only

    Ok(
      api
        .new_peer_connection(RTCConfiguration::default())
        .await?
        .into(),
    )
  }

  // since our selective forwarding unit (SFU) setup doesn't read opus packets,
  // our headers are the only thing that need to be correct; we can include junk
  // bytes as the actual media, simplyfing testing
  // adds the track to the PC (so it's negotiated) but does not transmit yet
  async fn make_fake_mic(
    pc: &Arc<RTCPeerConnection>,
  ) -> anyhow::Result<Arc<TrackLocalStaticSample>> {
    let track = Arc::new(TrackLocalStaticSample::new(
      RTCRtpCodecCapability {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        ..Default::default()
      },
      "audio".into(),
      "fake-mic".into(),
    ));
    let sender = pc.add_track(track.clone()).await?;
    tokio::spawn(async move {
      let mut buf = vec![0u8; 1500];
      while sender.read(&mut buf).await.is_ok() {}
    });
    Ok(track)
  }

  // starts the 20ms writer loop — server-side on_track fires shortly after this
  fn start_fake_mic(track: Arc<TrackLocalStaticSample>) {
    tokio::spawn(async move {
      let mut interval = tokio::time::interval(std::time::Duration::from_millis(20));
      loop {
        interval.tick().await;
        let _ = track
          .write_sample(&Sample {
            data: bytes::Bytes::from_static(&[0u8; 40]),
            duration: std::time::Duration::from_millis(20),
            ..Default::default()
          })
          .await;
      }
    });
  }

  #[tokio::test]
  async fn offer_handshake_reaches_connected() -> anyhow::Result<()> {
    let room: Arc<Room> = Room::default().into();

    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    let client = make_client_pc().await?;
    client
      .add_transceiver_from_kind(
        webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio,
        None,
      )
      .await?;

    let (connected_tx, connected_rx) = tokio::sync::oneshot::channel();
    let mut connected_tx = Some(connected_tx);

    client.on_peer_connection_state_change(Box::new(move |new_state| {
      if new_state == RTCPeerConnectionState::Connected
        && let Some(tx) = connected_tx.take()
      {
        // sends signal to connected_rx that we've reached desired state for test to finish
        let _ = tx.send(());
      }

      Box::pin(async {})
    }));

    let offer = client.create_offer(None).await?;
    let mut gather = client.gathering_complete_promise().await;
    client.set_local_description(offer).await?;
    let _ = gather.recv().await;
    let offer = client.local_description().await.unwrap();

    // server side handling of clientside offer
    let answer = handle_offer(offer, room.clone(), Uuid::new_v4(), tx).await?;
    client.set_remote_description(answer).await?;

    // connected_rx will fire when Connected state is reached in on_.._state_change
    tokio::time::timeout(std::time::Duration::from_secs(10), connected_rx).await??;
    Ok(())
  }

  #[tokio::test]
  async fn second_join_renegotiates_first_peer() -> anyhow::Result<()> {
    let room: Arc<Room> = Room::default().into();

    // A joins
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    // triggers on_track in handle_offer listener, adding peer to track
    start_fake_mic(make_fake_mic(&a_client).await?);

    let (a_recv_tx, a_recv_rx) = tokio::sync::oneshot::channel();
    let mut a_recv_tx = Some(a_recv_tx);
    a_client.on_track(Box::new(move |_, _, _| {
      if let Some(tx) = a_recv_tx.take() {
        let _ = tx.send(());
      }

      Box::pin(async {})
    }));

    // a negotiates
    let offer = a_client.create_offer(None).await?;
    let mut gather = a_client.gathering_complete_promise().await;
    a_client.set_local_description(offer).await?;
    let _ = gather.recv().await;

    // server handles a offer, sends answer, a sets new description
    let answer = handle_offer(
      a_client.local_description().await.unwrap(),
      room.clone(),
      a_id,
      a_tx,
    )
    .await?;
    a_client.set_remote_description(answer).await?;

    //  A's media must reach the server before B joins,
    //  or B hears A via the self-hel path instead of initial answer
    wait_for_relay(&room, a_id).await?;

    // B joins
    let (b_tx, _b_rx) = tokio::sync::mpsc::channel(8);
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);

    let offer = b_client.create_offer(None).await?;
    let mut gather = b_client.gathering_complete_promise().await;
    b_client.set_local_description(offer).await?;
    let _ = gather.recv().await;
    let answer = handle_offer(
      b_client.local_description().await.unwrap(),
      room.clone(),
      Uuid::new_v4(),
      b_tx,
    )
    .await?;
    b_client.set_remote_description(answer).await?;

    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), a_rx.recv())
      .await?
      .ok_or_else(|| anyhow::anyhow!("A's stream closed"))??;
    let ServerVoice::Offer(reneg_offer) = msg.try_into_domain()? else {
      anyhow::bail!("Expected renegotation offer, got other msg");
    };

    // A sets new sdp from renegotation
    a_client.set_remote_description(reneg_offer).await?;
    let reneg_answer = a_client.create_answer(None).await?;
    a_client.set_local_description(reneg_answer).await?;
    handle_answer(
      room.clone(),
      a_id,
      a_client.local_description().await.unwrap(),
    )
    .await?;

    // B's audio reaches A after reneg (it's continously transmitting from spawn fake mic earlier)
    tokio::time::timeout(std::time::Duration::from_secs(10), a_recv_rx).await??;
    Ok(())
  }

  // confirms order of on_track fires triggers correct renegs and ends in right state
  #[tokio::test]
  async fn late_publisher_self_heals_via_renegotiation() -> anyhow::Result<()> {
    let room: Arc<Room> = Room::default().into();

    // A joins with a negotiated-but-silent mic: track is in the SDP, no packets yet
    let (a_tx, _a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    let a_mic = make_fake_mic(&a_client).await?;

    let offer = a_client.create_offer(None).await?;
    let mut gather = a_client.gathering_complete_promise().await;
    a_client.set_local_description(offer).await?;
    let _ = gather.recv().await;
    let answer = handle_offer(
      a_client.local_description().await.unwrap(),
      room.clone(),
      a_id,
      a_tx,
    )
    .await?;
    a_client.set_remote_description(answer).await?;

    // B joins while A's relay provably doesn't exist — B is listen-only
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    b_client
      .add_transceiver_from_kind(
        webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio,
        None,
      )
      .await?;

    let (b_recv_tx, b_recv_rx) = tokio::sync::oneshot::channel();
    let mut b_recv_tx = Some(b_recv_tx);
    b_client.on_track(Box::new(move |_, _, _| {
      if let Some(tx) = b_recv_tx.take() {
        let _ = tx.send(());
      }
      Box::pin(async {})
    }));

    let offer = b_client.create_offer(None).await?;
    let mut gather = b_client.gathering_complete_promise().await;
    b_client.set_local_description(offer).await?;
    let _ = gather.recv().await;
    let answer = handle_offer(
      b_client.local_description().await.unwrap(),
      room.clone(),
      b_id,
      b_tx,
    )
    .await?;
    b_client.set_remote_description(answer).await?;

    // sanity: we really are on the self-heal path — A's relay was absent at B's join
    {
      let peers = room.peers.read().await;
      let a_p = peers
        .get(&a_id)
        .ok_or_else(|| anyhow::anyhow!("A not in room"))?;
      assert!(
        a_p.relay.read().await.is_none(),
        "test precondition broken: A's relay already existed"
      );
    }

    // A starts speaking: server on_track(A) fires, wires relay to B, renegotiates B
    start_fake_mic(a_mic);

    // B receives the server-initiated reneg offer
    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), b_rx.recv())
      .await?
      .ok_or_else(|| anyhow::anyhow!("B's stream closed"))??;
    let ServerVoice::Offer(reneg_offer) = msg.try_into_domain()? else {
      anyhow::bail!("expected renegotiation offer, got other msg");
    };

    // B answers; test plays the gRPC loop's role
    b_client.set_remote_description(reneg_offer).await?;
    let reneg_answer = b_client.create_answer(None).await?;
    b_client.set_local_description(reneg_answer).await?;
    handle_answer(
      room.clone(),
      b_id,
      b_client.local_description().await.unwrap(),
    )
    .await?;

    // A's audio reaches B despite B having joined "too early"
    tokio::time::timeout(std::time::Duration::from_secs(10), b_recv_rx).await??;
    Ok(())
  }
}
