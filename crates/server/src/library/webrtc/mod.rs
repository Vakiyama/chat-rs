use crate::config::CONFIG;
use chat_shared::{
  convert::{IntoProto, stream::proto::ServerVoiceMessage},
  domain::stream::ServerVoice,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;
use webrtc::{
  api::{
    API, APIBuilder, interceptor_registry::register_default_interceptors,
    media_engine::MediaEngine, setting_engine::SettingEngine,
  },
  ice::{
    network_type::NetworkType,
    udp_mux::{UDPMuxDefault, UDPMuxParams},
    udp_network::UDPNetwork,
  },
  ice_transport::{ice_candidate_type::RTCIceCandidateType, ice_server::RTCIceServer},
  interceptor::registry::Registry,
  peer_connection::{
    RTCPeerConnection, configuration::RTCConfiguration,
    sdp::session_description::RTCSessionDescription,
  },
  rtp_transceiver::{
    RTCRtpTransceiverInit, rtp_codec::RTPCodecType, rtp_sender::RTCRtpSender,
    rtp_transceiver_direction::RTCRtpTransceiverDirection,
  },
  track::track_local::{TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP},
};

pub type PeerId = Uuid;

pub struct Participant {
  pub pc: Arc<RTCPeerConnection>,
  relay: RwLock<Option<Arc<TrackLocalStaticRTP>>>,
  signal_tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
  outbound_senders: RwLock<HashMap<PeerId, Arc<RTCRtpSender>>>,
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
  api: &API,
) -> anyhow::Result<()> {
  let config = RTCConfiguration {
    ice_servers: vec![RTCIceServer {
      urls: vec!["stun:stun.l.google.com:19302".to_owned()],
      ..Default::default()
    }],
    ..Default::default()
  };
  let peer_connection = Arc::new(api.new_peer_connection(config).await?);

  // initial m-line is client→server mic only; server→client audio always
  // arrives via server-offered m-lines in renegotiation
  peer_connection
    .add_transceiver_from_kind(
      RTPCodecType::Audio,
      Some(RTCRtpTransceiverInit {
        direction: RTCRtpTransceiverDirection::Recvonly,
        send_encodings: vec![],
      }),
    )
    .await?;

  let participant = Arc::new(Participant {
    pc: peer_connection.clone(),
    relay: RwLock::new(None),
    signal_tx: signal_tx.clone(),
    outbound_senders: Default::default(),
  });

  let track_room = room.clone();
  peer_connection.on_track(Box::new(move |track, _, _| {
    println!("started audio track on peer connection");
    let room = track_room.clone();
    Box::pin(async move {
      let relay = Arc::new(TrackLocalStaticRTP::new(
        track.codec().capability,
        format!("audio-{me}"),
        format!("relay-{me}"),
      ));

      // this participant must exist; bail loudly if not
      let Some(me_p) = room.peers.read().await.get(&me).cloned() else {
        eprintln!("on_track fired for {me} but they're not in the room");
        return;
      };
      *me_p.relay.write().await = Some(relay.clone());

      let others: Vec<(PeerId, Arc<Participant>)> = {
        let peers = room.peers.read().await;
        peers
          .iter()
          .filter(|(id, _)| **id != me)
          .map(|(id, p)| (*id, Arc::clone(p))) // Copy the id out of the guard
          .collect()
      };

      for (peer_id, peer) in others {
        let transceiver = match peer
          .pc
          .add_transceiver_from_track(
            relay.clone(),
            Some(RTCRtpTransceiverInit {
              direction: RTCRtpTransceiverDirection::Sendonly,
              send_encodings: vec![],
            }),
          )
          .await
        {
          Ok(s) => s,
          Err(e) => {
            eprintln!("add_transceiver_from_track to {peer_id} failed: {e:?}");
            continue;
          }
        };
        let sender = transceiver.sender().await;

        me_p
          .outbound_senders
          .write()
          .await
          .insert(peer_id, sender.clone());

        tokio::spawn(async move {
          let mut buf = vec![0u8; 1500];
          while sender.read(&mut buf).await.is_ok() {}
        });

        // todo: reneg serialization (negotiating/needs_renegotiation flags)
        if let Err(e) = renegotiate(&peer).await {
          eprintln!("renegotiate({peer_id}) error: {e:?}");
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

  // ── initial negotiation ──
  peer_connection.set_remote_description(offer).await?;
  let answer = peer_connection.create_answer(None).await?;
  let mut gather = peer_connection.gathering_complete_promise().await;
  peer_connection.set_local_description(answer).await?;
  let _ = gather.recv().await;
  let answer = peer_connection
    .local_description()
    .await
    .ok_or_else(|| anyhow::anyhow!("no local description"))?;

  // negotiation done: join the room, then answer the client
  room.peers.write().await.insert(me, participant.clone());
  signal_tx
    .send(Ok(ServerVoice::Answer(answer).into_proto()))
    .await?;

  // ── deliver existing publishers via one server-initiated renegotiation ──
  let existing: Vec<(PeerId, Arc<Participant>, Arc<TrackLocalStaticRTP>)> = {
    let peers = room.peers.read().await;
    let mut out = Vec::new();
    for (id, p) in peers.iter() {
      if *id == me {
        continue;
      }
      if let Some(relay) = p.relay.read().await.clone() {
        out.push((*id, Arc::clone(p), relay));
      }
    }
    out
  };

  if !existing.is_empty() {
    for (publisher_id, publisher, relay) in existing {
      // add_track after remove_track fails with ErrRTPSenderNewTrackHasIncorrectEnvelope
      // this setup leaves transceivers for long lived clients in sdp's, not a major issue but will
      // cost sdp bytes
      let transceiver = peer_connection
        .add_transceiver_from_track(
          relay.clone(),
          Some(RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Sendonly,
            send_encodings: vec![],
          }),
        )
        .await?;
      let sender = transceiver.sender().await;

      publisher
        .outbound_senders
        .write()
        .await
        .insert(me, sender.clone());
      tokio::spawn(async move {
        let mut buf = vec![0u8; 1500];
        while sender.read(&mut buf).await.is_ok() {}
      });
      let _ = publisher_id; // (or use it in logs)
    }
    renegotiate(&participant).await?;
  }

  Ok(())
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

pub async fn handle_leave(room: Arc<Room>, me: PeerId) -> anyhow::Result<()> {
  // idempotent: second call finds nothing and returns
  let Some(leaving) = room.peers.write().await.remove(&me) else {
    return Ok(());
  };

  // snapshot the fan-out before closing anything
  let outbound: Vec<(PeerId, Arc<RTCRtpSender>)> = {
    let map = leaving.outbound_senders.read().await;
    map.iter().map(|(id, s)| (*id, s.clone())).collect()
  };

  let _ = leaving.pc.close().await; // frees the mux conn — the poisoning fix

  for (subscriber_id, sender) in outbound {
    let Some(sub) = room.peers.read().await.get(&subscriber_id).cloned() else {
      continue; // subscriber left in the meantime
    };
    if let Err(e) = sub.pc.remove_track(&sender).await {
      eprintln!("remove_track on {subscriber_id} failed: {e:?}");
      continue;
    }
    if let Err(e) = renegotiate(&sub).await {
      eprintln!("renegotiate({subscriber_id}) after leave failed: {e:?}");
    }
  }

  // reverse bookkeeping: senders ON my pc died with pc.close(),
  // so purge my key from every remaining publisher's map
  for (_, p) in room.peers.read().await.iter() {
    p.outbound_senders.write().await.remove(&me);
  }

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
  use std::{sync::Arc, time::Duration};

  use chat_shared::convert::stream::proto::ServerVoiceMessage;
  use chat_shared::{convert::TryIntoDomain, domain::stream::ServerVoice};
  use uuid::Uuid;
  use webrtc::{
    api::{
      API, APIBuilder,
      interceptor_registry::register_default_interceptors,
      media_engine::{MIME_TYPE_OPUS, MediaEngine},
    },
    interceptor::registry::Registry,
    media::Sample,
    peer_connection::{
      RTCPeerConnection, configuration::RTCConfiguration,
      peer_connection_state::RTCPeerConnectionState,
      sdp::session_description::RTCSessionDescription,
    },
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::track_local_static_sample::TrackLocalStaticSample,
  };

  use crate::library::webrtc::{PeerId, Room, handle_answer, handle_offer};

  type SignalRx = tokio::sync::mpsc::Receiver<Result<ServerVoiceMessage, tonic::Status>>;

  fn make_server_api() -> anyhow::Result<API> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs()?;
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
    Ok(
      APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build(),
    )
  }

  // next server->client signal, or error after 10s
  async fn recv_signal(rx: &mut SignalRx) -> anyhow::Result<ServerVoice> {
    let msg = tokio::time::timeout(Duration::from_secs(10), rx.recv())
      .await?
      .ok_or_else(|| anyhow::anyhow!("signal stream closed"))??;
    Ok(msg.try_into_domain()?)
  }

  async fn expect_answer(rx: &mut SignalRx) -> anyhow::Result<RTCSessionDescription> {
    match recv_signal(rx).await? {
      ServerVoice::Answer(desc) => Ok(desc),
      other => anyhow::bail!("expected Answer, got {other:?}"),
    }
  }

  async fn expect_offer(rx: &mut SignalRx) -> anyhow::Result<RTCSessionDescription> {
    match recv_signal(rx).await? {
      ServerVoice::Offer(desc) => Ok(desc),
      other => anyhow::bail!("expected Offer, got {other:?}"),
    }
  }

  // client-side initial join: gather non-trickle, hand offer to server, apply answer from rx
  async fn join(
    client: &Arc<RTCPeerConnection>,
    room: &Arc<Room>,
    id: PeerId,
    tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
    rx: &mut SignalRx,
    api: &API,
  ) -> anyhow::Result<()> {
    let offer = client.create_offer(None).await?;
    let mut gather = client.gathering_complete_promise().await;
    client.set_local_description(offer).await?;
    let _ = gather.recv().await;
    handle_offer(
      client.local_description().await.unwrap(),
      room.clone(),
      id,
      tx,
      api,
    )
    .await?;
    let answer = expect_answer(rx).await?;
    client.set_remote_description(answer).await?;
    Ok(())
  }

  // client answers a server-initiated renegotiation; test plays the gRPC loop's role
  async fn answer_reneg(
    client: &Arc<RTCPeerConnection>,
    room: &Arc<Room>,
    id: PeerId,
    offer: RTCSessionDescription,
  ) -> anyhow::Result<()> {
    client.set_remote_description(offer).await?;
    let answer = client.create_answer(None).await?;
    client.set_local_description(answer).await?;
    handle_answer(room.clone(), id, client.local_description().await.unwrap()).await
  }

  // one-shot on_track latch
  fn on_track_latch(client: &Arc<RTCPeerConnection>) -> tokio::sync::oneshot::Receiver<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    client.on_track(Box::new(move |_, _, _| {
      if let Some(tx) = tx.take() {
        let _ = tx.send(());
      }
      Box::pin(async {})
    }));
    rx
  }

  async fn wait_for_relay(room: &Arc<Room>, id: PeerId) -> anyhow::Result<()> {
    for _ in 0..100 {
      if let Some(p) = room.peers.read().await.get(&id)
        && p.relay.read().await.is_some()
      {
        return Ok(());
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("relay for {id} never appeared")
  }

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
    let api = make_server_api()?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
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
        let _ = tx.send(());
      }
      Box::pin(async {})
    }));

    join(&client, &room, Uuid::new_v4(), tx, &mut rx, &api).await?;

    tokio::time::timeout(Duration::from_secs(10), connected_rx).await??;
    Ok(())
  }

  // B joins a room where A is already publishing:
  //   - B hears A via the post-join renegotiation (Answer then Offer on B's rx)
  //   - A hears B via the reneg triggered by B's media arriving
  #[tokio::test]
  async fn both_directions_with_sequential_joins() -> anyhow::Result<()> {
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A joins, publishing immediately
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&a_client).await?);
    let a_heard = on_track_latch(&a_client);
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api).await?;

    // pin the test to the "relay exists at join time" path
    wait_for_relay(&room, a_id).await?;

    // B joins, publishing immediately
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);
    let b_heard = on_track_latch(&b_client);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api).await?;

    // NEW-CONTRACT ASSERTION: B's join is immediately followed by a reneg
    // offer carrying A's relay (Answer was consumed inside join())
    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b).await?;

    // B's media reaching the server renegotiates A
    let offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_to_a).await?;

    // both directions audible
    tokio::time::timeout(Duration::from_secs(10), b_heard).await??; // B hears A
    tokio::time::timeout(Duration::from_secs(10), a_heard).await??; // A hears B
    Ok(())
  }

  // B joins while A's relay provably doesn't exist; A publishing later
  // must still reach B via the self-heal renegotiation
  #[tokio::test]
  async fn late_publisher_self_heals_via_renegotiation() -> anyhow::Result<()> {
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A joins with a negotiated-but-silent mic
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    let a_mic = make_fake_mic(&a_client).await?;
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api).await?;

    // B joins listen-only while A's relay is absent
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    b_client
      .add_transceiver_from_kind(
        webrtc::rtp_transceiver::rtp_codec::RTPCodecType::Audio,
        None,
      )
      .await?;
    let b_heard = on_track_latch(&b_client);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api).await?;

    // sanity: no relay existed, so join() consumed an Answer and NO offer
    // should have been sent to B yet
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

    // A starts speaking: on_track(A) wires relay to B and renegotiates B
    start_fake_mic(a_mic);

    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b).await?;

    tokio::time::timeout(Duration::from_secs(10), b_heard).await??;
    Ok(())
  }
}
