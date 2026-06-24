use chat_shared::{
  convert::{IntoProto, stream::proto::ServerVoiceMessage},
  domain::stream::{DisplayVoiceUser, ServerVoice, User},
};
use futures_util::future::join_all;
use std::{
  collections::HashMap,
  sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize},
  },
};
use tokio::sync::RwLock;
use uuid::Uuid;
use webrtc::{
  api::API,
  ice_transport::ice_server::RTCIceServer,
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

// per-peer signaling state: an offer is in flight, and/or another reneg is queued.
// all fields live under one mutex so "queue a follow-up reneg" and "finish + drain
// the queue" are atomic against each other — two independent atomics race into a lost
// wakeup where a reneg is queued but its drain check has already passed.
//
// `gen` increments every time a fresh offer goes in flight. The renegotiation timeout
// task captures the gen of the offer it guards and only acts if that exact negotiation
// is still in flight — otherwise a timeout from an already-answered round would fire
// 3s later and clobber a newer, unrelated round (offer glare).
#[derive(Default)]
struct NegState {
  in_flight: bool,
  pending: bool,
  generation: u64,
}

pub struct Participant {
  pub pc: Arc<RTCPeerConnection>,
  relay: RwLock<Option<Arc<TrackLocalStaticRTP>>>,
  pub signal_tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
  outbound_senders: RwLock<HashMap<PeerId, Arc<RTCRtpSender>>>,
  neg: Mutex<NegState>,
  pub user: User,
  pub speaking: AtomicBool,
  pub muted: AtomicBool,
  pub deafened: AtomicBool,
  // the voice-stream connection that created this participant. PeerId is the user
  // id, so a network-blip reconnect replaces a stale participant with a fresh one
  // under the same key; this distinguishes them so the OLD connection's teardown
  // (when its dead stream is finally detected) can't evict the peer the user has
  // since rejoined with on a new connection. See handle_leave_if_owner.
  pub conn_id: Uuid,
}

#[derive(Default)]
pub struct Room {
  pub peers: RwLock<HashMap<PeerId, Arc<Participant>>>,
  // identity of the room, copied onto every presence snapshot so server-wide
  // subscribers can bucket presence per channel/server. Defaults to nil (tests
  // construct rooms via Room::default()); production rooms are built with Room::new.
  pub voice_channel_id: Uuid,
  pub server_id: Uuid,
  // Count of joins currently negotiating against this room. A joiner holds the
  // room Arc (and may have evicted a stale prior participant of its own) for the
  // whole offer/answer handshake before its participant lands in `peers`. Empty-
  // room eviction must skip a room with any join in flight, or it would drop the
  // room from the manager between handing out the Arc and the participant being
  // inserted — splitting the joiner off into an orphaned room. See
  // reserve_room_for_join / evict_room_if_empty in api/stream.rs.
  pub pending_joins: AtomicUsize,
}

impl Room {
  pub fn new(voice_channel_id: Uuid, server_id: Uuid) -> Self {
    Room {
      peers: Default::default(),
      voice_channel_id,
      server_id,
      pending_joins: AtomicUsize::new(0),
    }
  }
}

// A voice-stream connection watching one server's call presence. `user_id` lets a
// server-wide broadcast skip a subscriber who is themselves a participant in the
// room that changed — that participant already gets the richer in-call snapshot
// (which carries speaking state), so the membership-only snapshot must not clobber it.
struct PresenceSubscriber {
  user_id: Uuid,
  server_id: Uuid,
  tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
}

// conn_id -> subscriber. Keyed per connection (not per user) so a user with two
// clients open each get their own snapshots, and so cleanup is unambiguous on
// stream teardown.
static PRESENCE_REGISTRY: std::sync::LazyLock<Mutex<HashMap<Uuid, PresenceSubscriber>>> =
  std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register_presence_subscriber(
  conn_id: Uuid,
  user_id: Uuid,
  server_id: Uuid,
  tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
) {
  PRESENCE_REGISTRY.lock().unwrap().insert(
    conn_id,
    PresenceSubscriber {
      user_id,
      server_id,
      tx,
    },
  );
}

pub fn unregister_presence_subscriber(conn_id: &Uuid) {
  PRESENCE_REGISTRY.lock().unwrap().remove(conn_id);
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_offer(
  offer: RTCSessionDescription,
  room: Arc<Room>,
  me: PeerId,
  signal_tx: tokio::sync::mpsc::Sender<Result<ServerVoiceMessage, tonic::Status>>,
  api: &API,
  voice_channel_id: Uuid,
  user: User,
  conn_id: Uuid,
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
    neg: Mutex::new(NegState {
      in_flight: true,
      pending: false,
      generation: 0,
    }),
    user,
    speaking: false.into(),
    muted: false.into(),
    deafened: false.into(),
    conn_id,
  });

  let track_room = room.clone();
  let me_weak = Arc::downgrade(&participant);
  peer_connection.on_track(Box::new(move |track, _, _| {
    println!("started audio track on peer connection");
    let room = track_room.clone();
    let me_weak = me_weak.clone();
    Box::pin(async move {
      // grab our own participant directly via a weak ref, not the room map: on_track
      // can fire before we're inserted into the room (it only fires once, so a miss is
      // unrecoverable), and the weak ref breaks the pc -> on_track -> participant -> pc
      // cycle so leave()'s pc.close() can actually drop the connection.
      let Some(me_p) = me_weak.upgrade() else {
        return;
      };

      // Unique track id per join. Clients dedup inbound tracks by track id; a peer that
      // leaves and rejoins keeps the same PeerId, so a peer-stable id ("audio-{me}")
      // collides with the id a staying client already has cached from the previous
      // session, and the rejoiner's audio is silently dropped until that client also
      // rejoins. A fresh id every time keeps the rejoiner distinct.
      let relay = Arc::new(TrackLocalStaticRTP::new(
        track.codec().capability,
        format!("audio-{me}-{}", Uuid::new_v4()),
        format!("relay-{me}"),
      ));
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

        if let Err(e) = renegotiate(peer.clone(), voice_channel_id).await {
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
    .send(Ok(
      ServerVoice::Answer {
        description: answer,
        voice_channel_id,
      }
      .into_proto(),
    ))
    .await?;

  // initial answer is on its way to the client; release our negotiation claim. a peer
  // already in the room may have queued a reneg against us while we were joining.
  let resume = finish_negotiation(&participant);

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

  let has_existing = !existing.is_empty();
  for (_publisher_id, publisher, relay) in existing {
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
  }

  if has_existing || resume {
    renegotiate(participant.clone(), voice_channel_id).await?;
  }

  broadcast_presence(room.clone()).await;
  broadcast_server_presence(room.clone()).await;

  Ok(())
}

// claim the right to send an offer. if one is already in flight, queue exactly one
// follow-up and return None — the next offer reflects all accumulated track changes,
// so any number of queued requests collapse into a single round. On a successful
// claim, returns Some(gen) identifying this negotiation so its timeout can tell
// whether it is still the round in flight.
fn try_begin_negotiation(peer: &Participant) -> Option<u64> {
  let mut s = peer.neg.lock().unwrap();
  if s.in_flight {
    s.pending = true;
    return None;
  }
  s.in_flight = true;
  s.generation = s.generation.wrapping_add(1);
  Some(s.generation)
}

// an offer/answer round finished: release the in-flight claim and report whether a
// follow-up reneg was queued while it was in flight.
//
// This only releases — it must NOT re-claim. The caller drains queued work by calling
// renegotiate() (which re-claims cleanly and spawns its own timeout). If this re-claimed
// itself, renegotiate() would then see in_flight already set, queue `pending`, and send
// nothing — wedging the peer with a queued change that never goes out, which silently
// kills audio in both directions after a join/leave burst.
fn finish_negotiation(peer: &Participant) -> bool {
  let mut s = peer.neg.lock().unwrap();
  s.in_flight = false;
  std::mem::take(&mut s.pending)
}

async fn renegotiate(peer: Arc<Participant>, voice_channel_id: Uuid) -> anyhow::Result<()> {
  let Some(generation) = try_begin_negotiation(&peer) else {
    return Ok(());
  };

  // Spawn a watchdog that resends the offer if the client never answers, otherwise a
  // peer that drops a renegotiation offer would permanently block all subsequent track
  // changes from reaching it. The watchdog owns the in-flight claim until the answer
  // arrives (handle_answer clears in_flight) or we exhaust our retries. It only acts on
  // the generation it started with, so once a newer negotiation supersedes this one the
  // watchdog exits instead of clobbering it with a stale duplicate offer (glare).
  let timeout = tokio::time::Duration::from_secs(3);
  const MAX_RESENDS: u32 = 3;
  let watchdog_peer = peer.clone();
  let watchdog_channel_id = voice_channel_id;
  tokio::spawn(async move {
    let mut generation = generation;
    for _ in 0..MAX_RESENDS {
      tokio::time::sleep(timeout).await;
      {
        let mut s = watchdog_peer.neg.lock().unwrap();
        // answered (in_flight cleared) or superseded by a newer round → stop.
        if !s.in_flight || s.generation != generation {
          return;
        }
        // still unanswered: resend under a fresh generation so a late answer to the
        // previous offer can't be mistaken for an answer to this resend.
        s.generation = s.generation.wrapping_add(1);
        s.pending = false;
        generation = s.generation;
      }
      if let Err(e) = send_offer(&watchdog_peer, watchdog_channel_id).await {
        eprintln!("renegotiate resend failed: {e:?}");
        finish_negotiation(&watchdog_peer);
        return;
      }
    }
    // gave up: release the claim so a future track change can start a fresh round
    // instead of the peer wedging with in_flight stuck true forever.
    let mut s = watchdog_peer.neg.lock().unwrap();
    if s.in_flight && s.generation == generation {
      s.in_flight = false;
    }
  });

  if let Err(e) = send_offer(&peer, voice_channel_id).await {
    // release the claim so a later trigger can retry instead of wedging forever.
    finish_negotiation(&peer);
    return Err(e);
  }
  Ok(())
}

async fn send_offer(peer: &Participant, voice_channel_id: Uuid) -> anyhow::Result<()> {
  let offer = peer.pc.create_offer(None).await?;
  peer.pc.set_local_description(offer).await?;
  let local = peer
    .pc
    .local_description()
    .await
    .ok_or_else(|| anyhow::anyhow!("no local description"))?;
  peer
    .signal_tx
    .send(Ok(
      ServerVoice::Offer {
        description: local,
        voice_channel_id,
      }
      .into_proto(),
    ))
    .await?; // push to that peer's stream
  Ok(())
}

pub async fn handle_leave(
  room: Arc<Room>,
  me: PeerId,
  voice_channel_id: Uuid,
) -> anyhow::Result<()> {
  // idempotent: second call finds nothing and returns
  let Some(leaving) = room.peers.write().await.remove(&me) else {
    return Ok(());
  };
  teardown_peer(room, me, leaving, voice_channel_id).await
}

// Leave, but only if the room's participant for `me` is still the one created by
// `conn_id`. Used by the implicit stream-teardown cleanup: on a network-blip
// reconnect the user rejoins under the same PeerId on a fresh connection, so when
// the OLD (dead) connection's stream is finally detected and its cleanup runs, it
// must NOT evict the freshly-rejoined participant. The check-and-remove happens
// under one write lock so a rejoin landing concurrently can't be clobbered.
pub async fn handle_leave_if_owner(
  room: Arc<Room>,
  me: PeerId,
  conn_id: Uuid,
  voice_channel_id: Uuid,
) -> anyhow::Result<()> {
  let leaving = {
    let mut peers = room.peers.write().await;
    match peers.get(&me) {
      Some(p) if p.conn_id == conn_id => peers.remove(&me).expect("peer present under lock"),
      // no peer, or it belongs to a newer connection → leave it alone
      _ => return Ok(()),
    }
  };
  teardown_peer(room, me, leaving, voice_channel_id).await
}

// Tear down a participant that has already been removed from the room: close its
// pc, drop its relay from every subscriber (renegotiating them), and purge its key
// from remaining publishers' fan-out maps. Split from handle_leave so the removal
// can be gated (see handle_leave_if_owner) without duplicating the teardown.
async fn teardown_peer(
  room: Arc<Room>,
  me: PeerId,
  leaving: Arc<Participant>,
  voice_channel_id: Uuid,
) -> anyhow::Result<()> {
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
    if let Err(e) = renegotiate(sub.clone(), voice_channel_id).await {
      eprintln!("renegotiate({subscriber_id}) after leave failed: {e:?}");
    }
  }

  // reverse bookkeeping: senders ON my pc died with pc.close(),
  // so purge my key from every remaining publisher's map
  for (_, p) in room.peers.read().await.iter() {
    p.outbound_senders.write().await.remove(&me);
  }

  broadcast_presence(room.clone()).await;
  broadcast_server_presence(room.clone()).await;

  Ok(())
}

// In-call presence: the rich snapshot (includes live speaking state) sent to the
// peers actually in the call. Triggered on join/leave/speaking changes.
pub async fn broadcast_presence(room: Arc<Room>) {
  let msg = ServerVoice::PresenceSnapshot {
    voice_channel_id: room.voice_channel_id,
    server_id: room.server_id,
    peers: room
      .peers
      .read()
      .await
      .clone()
      .into_iter()
      .map(|peer| {
        let peer = peer.1;
        DisplayVoiceUser {
          user: peer.user.clone(),
          muted: peer.muted.load(std::sync::atomic::Ordering::Relaxed),
          deafened: peer.deafened.load(std::sync::atomic::Ordering::Relaxed),
          speaking: peer.speaking.load(std::sync::atomic::Ordering::Relaxed),
        }
      })
      .collect(),
  };

  let proto = msg.into_proto();

  let txs: Vec<_> = room
    .peers
    .read()
    .await
    .values()
    .map(|p| p.signal_tx.clone())
    .collect();

  let results = join_all(txs.into_iter().map(|tx| {
    let proto = proto.clone();
    async move { tx.send(Ok(proto)).await }
  }))
  .await;

  // todo: we could use this as an opportunity to heal the call and remove failed presence
  // snapshots peers.
  let result: Result<Vec<_>, _> = results.into_iter().collect();
  let _ = result.map_err(|e| eprintln!("Error sending presence snapshot: {e}"));
}

// Membership-only snapshot (speaking forced false) of a room — what users who are
// NOT in the call see. Empty `peers` is how a server-wide subscriber learns a call
// emptied out and should be cleared.
pub async fn membership_snapshot(room: &Room) -> ServerVoiceMessage {
  ServerVoice::PresenceSnapshot {
    voice_channel_id: room.voice_channel_id,
    server_id: room.server_id,
    peers: room
      .peers
      .read()
      .await
      .values()
      .map(|peer| DisplayVoiceUser {
        user: peer.user.clone(),
        muted: peer.muted.load(std::sync::atomic::Ordering::Relaxed),
        deafened: peer.deafened.load(std::sync::atomic::Ordering::Relaxed),
        speaking: false,
      })
      .collect(),
  }
  .into_proto()
}

// Server-wide presence: push a room's membership snapshot to every connection
// watching that room's server, except participants already in the room (they get
// the richer in-call snapshot via broadcast_presence). Triggered on join/leave.
pub async fn broadcast_server_presence(room: Arc<Room>) {
  let in_room: std::collections::HashSet<Uuid> =
    room.peers.read().await.keys().copied().collect();

  let txs: Vec<_> = {
    let registry = PRESENCE_REGISTRY.lock().unwrap();
    registry
      .values()
      .filter(|sub| sub.server_id == room.server_id && !in_room.contains(&sub.user_id))
      .map(|sub| sub.tx.clone())
      .collect()
  };

  if txs.is_empty() {
    return;
  }

  let proto = membership_snapshot(&room).await;

  let results = join_all(txs.into_iter().map(|tx| {
    let proto = proto.clone();
    async move { tx.send(Ok(proto)).await }
  }))
  .await;

  let result: Result<Vec<_>, _> = results.into_iter().collect();
  let _ = result.map_err(|e| eprintln!("Error sending server presence snapshot: {e}"));
}

pub async fn handle_answer(
  room: Arc<Room>,
  me: PeerId,
  answer: RTCSessionDescription,
  voice_channel_id: Uuid,
) -> anyhow::Result<()> {
  let peer = room
    .peers
    .read()
    .await
    .get(&me)
    .cloned()
    .ok_or_else(|| anyhow::anyhow!("received rtc answer for unknown peer"))?;

  let result = peer.pc.set_remote_description(answer).await;
  // clear the claim even on failure, or the PC wedges forever
  let resume = finish_negotiation(&peer);
  result?;

  if resume {
    renegotiate(peer.clone(), voice_channel_id).await?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::{sync::Arc, time::Duration};

  use chat_shared::convert::stream::proto::ServerVoiceMessage;
  use chat_shared::domain::stream::User;
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

  use crate::library::webrtc::{PeerId, Room, handle_answer, handle_leave, handle_offer};

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

  // Presence snapshots are broadcast on the same signal channel and can arrive at
  // any point between negotiation messages, so the negotiation-focused helpers skip
  // them rather than treating them as an unexpected frame.
  async fn expect_answer(rx: &mut SignalRx) -> anyhow::Result<RTCSessionDescription> {
    loop {
      match recv_signal(rx).await? {
        ServerVoice::Answer { description, .. } => return Ok(description),
        ServerVoice::PresenceSnapshot { .. } => continue,
        other => anyhow::bail!("expected Answer, got {other:?}"),
      }
    }
  }

  async fn expect_offer(rx: &mut SignalRx) -> anyhow::Result<RTCSessionDescription> {
    loop {
      match recv_signal(rx).await? {
        ServerVoice::Offer { description, .. } => return Ok(description),
        ServerVoice::PresenceSnapshot { .. } => continue,
        other => anyhow::bail!("expected Offer, got {other:?}"),
      }
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
    voice_channel_id: Uuid,
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
      voice_channel_id,
      User {
        id: Uuid::new_v4(),
        name: "test".into(),
      },
      Uuid::new_v4(),
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
    voice_channel_id: Uuid,
  ) -> anyhow::Result<()> {
    client.set_remote_description(offer).await?;
    let answer = client.create_answer(None).await?;
    client.set_local_description(answer).await?;
    handle_answer(
      room.clone(),
      id,
      client.local_description().await.unwrap(),
      voice_channel_id,
    )
    .await
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

  // fires once the pc has received `n` inbound tracks (one per remote publisher)
  fn on_track_count(
    client: &Arc<RTCPeerConnection>,
    n: usize,
  ) -> tokio::sync::oneshot::Receiver<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    let mut seen = 0usize;
    client.on_track(Box::new(move |_, _, _| {
      seen += 1;
      if seen >= n
        && let Some(tx) = tx.take()
      {
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
    let room_id = uuid::Uuid::new_v4();
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

    join(&client, &room, Uuid::new_v4(), tx, &mut rx, &api, room_id).await?;

    tokio::time::timeout(Duration::from_secs(10), connected_rx).await??;
    Ok(())
  }

  // B joins a room where A is already publishing:
  //   - B hears A via the post-join renegotiation (Answer then Offer on B's rx)
  //   - A hears B via the reneg triggered by B's media arriving
  #[tokio::test]
  async fn both_directions_with_sequential_joins() -> anyhow::Result<()> {
    let room_id = uuid::Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A joins, publishing immediately
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&a_client).await?);
    let a_heard = on_track_latch(&a_client);
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api, room_id).await?;

    // pin the test to the "relay exists at join time" path
    wait_for_relay(&room, a_id).await?;

    // B joins, publishing immediately
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);
    let b_heard = on_track_latch(&b_client);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api, room_id).await?;

    // NEW-CONTRACT ASSERTION: B's join is immediately followed by a reneg
    // offer carrying A's relay (Answer was consumed inside join())
    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b, room_id).await?;

    // B's media reaching the server renegotiates A
    let offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_to_a, room_id).await?;

    // both directions audible
    tokio::time::timeout(Duration::from_secs(10), b_heard).await??; // B hears A
    tokio::time::timeout(Duration::from_secs(10), a_heard).await??; // A hears B
    Ok(())
  }

  // B joins while A's relay provably doesn't exist; A publishing later
  // must still reach B via the self-heal renegotiation
  #[tokio::test]
  async fn late_publisher_self_heals_via_renegotiation() -> anyhow::Result<()> {
    let room_id = uuid::Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A joins with a negotiated-but-silent mic
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    let a_mic = make_fake_mic(&a_client).await?;
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api, room_id).await?;

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
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api, room_id).await?;

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
    answer_reneg(&b_client, &room, b_id, offer_to_b, room_id).await?;

    tokio::time::timeout(Duration::from_secs(10), b_heard).await??;
    Ok(())
  }

  // A and B are in a call hearing each other; C joins publishing. the regression:
  // C heard A and B, but A and B never heard C because the reneg carrying C's track
  // was queued and silently dropped. all three must hear all others.
  #[tokio::test]
  async fn third_peer_is_heard_by_existing_peers() -> anyhow::Result<()> {
    let room_id = uuid::Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A joins, publishing immediately; expects to hear B then C
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&a_client).await?);
    let a_heard_both = on_track_count(&a_client, 2);
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api, room_id).await?;
    wait_for_relay(&room, a_id).await?;

    // B joins, publishing immediately; expects to hear A then C
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);
    let b_heard_both = on_track_count(&b_client, 2);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api, room_id).await?;

    // settle A <-> B
    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b, room_id).await?;
    let offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_to_a, room_id).await?;
    wait_for_relay(&room, b_id).await?;

    // C joins, publishing immediately; expects to hear A and B
    let (c_tx, mut c_rx) = tokio::sync::mpsc::channel(8);
    let c_id = Uuid::new_v4();
    let c_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&c_client).await?);
    let c_heard_both = on_track_count(&c_client, 2);
    join(&c_client, &room, c_id, c_tx, &mut c_rx, &api, room_id).await?;

    // C hears A and B via the post-join reneg (both relays in one offer)
    let offer_to_c = expect_offer(&mut c_rx).await?;
    answer_reneg(&c_client, &room, c_id, offer_to_c, room_id).await?;

    // C's media reaching the server must renegotiate BOTH existing peers so they hear C
    let offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_to_a, room_id).await?;
    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b, room_id).await?;

    tokio::time::timeout(Duration::from_secs(10), c_heard_both).await??;
    tokio::time::timeout(Duration::from_secs(10), a_heard_both).await??;
    tokio::time::timeout(Duration::from_secs(10), b_heard_both).await??;
    Ok(())
  }

  // Regression for the finish_negotiation double-claim wedge: a track change that
  // arrives while a peer already has an offer in flight is queued as `pending`. When the
  // in-flight offer is answered, the queued change MUST be drained into a fresh offer.
  // The bug had finish_negotiation re-claim the in-flight slot itself, so the caller's
  // renegotiate() saw in_flight==true, re-queued pending, and sent nothing — leaving the
  // peer permanently unable to hear the queued publisher (audio dead in both directions
  // after a join/leave burst). Here A holds C's offer while D's track queues behind it;
  // A must still end up hearing both C and D.
  #[tokio::test]
  async fn queued_renegotiation_is_drained_after_answer() -> anyhow::Result<()> {
    let room_id = uuid::Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A and B join and settle into hearing each other.
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&a_client).await?);
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api, room_id).await?;
    wait_for_relay(&room, a_id).await?;

    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api, room_id).await?;

    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b, room_id).await?;
    let offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_to_a, room_id).await?;
    wait_for_relay(&room, b_id).await?;

    // From here, A must hear two more distinct publishers: C and D.
    let a_heard_cd = on_track_count(&a_client, 2);

    // C joins and publishes; A's reneg offer for C arrives but we deliberately DO NOT
    // answer it yet, so A's negotiation stays in flight.
    let (c_tx, mut c_rx) = tokio::sync::mpsc::channel(8);
    let c_id = Uuid::new_v4();
    let c_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&c_client).await?);
    join(&c_client, &room, c_id, c_tx, &mut c_rx, &api, room_id).await?;
    let held_offer_for_c = expect_offer(&mut a_rx).await?; // A.in_flight == true now

    // D joins and publishes while A's offer is still in flight: D's track is queued as
    // `pending` on A rather than sent immediately.
    let (d_tx, mut d_rx) = tokio::sync::mpsc::channel(8);
    let d_id = Uuid::new_v4();
    let d_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&d_client).await?);
    join(&d_client, &room, d_id, d_tx, &mut d_rx, &api, room_id).await?;
    // ensure D's media has reached the server and queued its reneg against A
    wait_for_relay(&room, d_id).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Now answer C's offer. Draining the answer must flush D's queued track into a new
    // offer — on the buggy code this offer was never produced and the next expect_offer
    // would hang until timeout.
    answer_reneg(&a_client, &room, a_id, held_offer_for_c, room_id).await?;
    let offer_for_d = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_for_d, room_id).await?;

    tokio::time::timeout(Duration::from_secs(10), a_heard_cd).await??;
    Ok(())
  }

  // End-to-end leave + rejoin: A and B hear each other, B leaves, then B rejoins on a
  // fresh peer connection. Both directions must be re-established — B hears A again and
  // A hears the rejoined B again.
  #[tokio::test]
  async fn leave_then_rejoin_restores_both_directions() -> anyhow::Result<()> {
    let room_id = uuid::Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A joins, publishing.
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&a_client).await?);
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api, room_id).await?;
    wait_for_relay(&room, a_id).await?;

    // B joins, publishing; settle A <-> B.
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api, room_id).await?;
    let offer_to_b = expect_offer(&mut b_rx).await?;
    answer_reneg(&b_client, &room, b_id, offer_to_b, room_id).await?;
    let offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, offer_to_a, room_id).await?;

    // B leaves: A is renegotiated to drop B's track and must answer to stay healthy.
    handle_leave(room.clone(), b_id, room_id).await?;
    let drop_offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, drop_offer_to_a, room_id).await?;

    // B rejoins on a brand-new peer connection (the client builds a fresh pc per join).
    let a_hears_rejoin = on_track_latch(&a_client);
    let (b2_tx, mut b2_rx) = tokio::sync::mpsc::channel(8);
    let b2_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b2_client).await?);
    let b2_hears_a = on_track_latch(&b2_client);
    join(&b2_client, &room, b_id, b2_tx, &mut b2_rx, &api, room_id).await?;

    // B hears A again via the post-join reneg carrying A's relay.
    let offer_to_b2 = expect_offer(&mut b2_rx).await?;
    answer_reneg(&b2_client, &room, b_id, offer_to_b2, room_id).await?;

    // B's media reaching the server renegotiates A so A hears the rejoined B.
    let rejoin_offer_to_a = expect_offer(&mut a_rx).await?;
    answer_reneg(&a_client, &room, a_id, rejoin_offer_to_a, room_id).await?;

    tokio::time::timeout(Duration::from_secs(10), b2_hears_a).await??;
    tokio::time::timeout(Duration::from_secs(10), a_hears_rejoin).await??;
    Ok(())
  }

  // Network-blip reconnect race: a user rejoins under the same PeerId on a fresh
  // connection (new conn_id), then the OLD connection's dead stream is finally
  // detected and its teardown fires. That teardown (handle_leave_if_owner with the
  // OLD conn_id) must be a no-op — it must NOT evict the freshly-rejoined peer —
  // while a teardown carrying the CURRENT conn_id still removes it normally.
  #[tokio::test]
  async fn stale_connection_teardown_does_not_evict_rejoined_peer() -> anyhow::Result<()> {
    use crate::library::webrtc::{handle_leave_if_owner, handle_offer};

    let room_id = Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;
    let user_id = Uuid::new_v4();

    let offer_for = async |conn_id: Uuid| -> anyhow::Result<()> {
      let (tx, mut rx) = tokio::sync::mpsc::channel(8);
      let client = make_client_pc().await?;
      start_fake_mic(make_fake_mic(&client).await?);
      let offer = client.create_offer(None).await?;
      let mut gather = client.gathering_complete_promise().await;
      client.set_local_description(offer).await?;
      let _ = gather.recv().await;
      handle_offer(
        client.local_description().await.unwrap(),
        room.clone(),
        user_id,
        tx,
        &api,
        room_id,
        User {
          id: user_id,
          name: "test".into(),
        },
        conn_id,
      )
      .await?;
      let answer = expect_answer(&mut rx).await?;
      client.set_remote_description(answer).await?;
      Ok(())
    };

    // join on conn 1, then "rejoin" on conn 2 (the new Offer overwrites the peer).
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    offer_for(conn1).await?;
    offer_for(conn2).await?;

    // the dead conn-1 stream's teardown must NOT remove the conn-2 participant.
    handle_leave_if_owner(room.clone(), user_id, conn1, room_id).await?;
    {
      let peers = room.peers.read().await;
      let peer = peers.get(&user_id).expect("rejoined peer must survive");
      assert_eq!(peer.conn_id, conn2, "surviving peer is the conn-2 rejoin");
    }

    // the owning connection's teardown still removes it.
    handle_leave_if_owner(room.clone(), user_id, conn2, room_id).await?;
    assert!(
      room.peers.read().await.get(&user_id).is_none(),
      "owning teardown removes the peer"
    );
    Ok(())
  }

  async fn relay_id(room: &Arc<Room>, id: PeerId) -> Option<String> {
    use webrtc::track::track_local::TrackLocal;
    let peers = room.peers.read().await;
    let p = peers.get(&id)?;
    let relay = p.relay.read().await;
    relay.as_ref().map(|r| r.id().to_string())
  }

  // A staying client dedups inbound tracks by track id, so a rejoining peer (same
  // PeerId) MUST get a fresh relay track id — otherwise the staying client treats the
  // rejoiner's audio as already-handled and drops it. Guards the per-join unique id.
  #[tokio::test]
  async fn rejoining_peer_gets_a_fresh_relay_track_id() -> anyhow::Result<()> {
    let room_id = uuid::Uuid::new_v4();
    let room: Arc<Room> = Room::default().into();
    let api = make_server_api()?;

    // A is present so B's on_track has someone to wire a relay toward.
    let (a_tx, mut a_rx) = tokio::sync::mpsc::channel(8);
    let a_id = Uuid::new_v4();
    let a_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&a_client).await?);
    join(&a_client, &room, a_id, a_tx, &mut a_rx, &api, room_id).await?;
    wait_for_relay(&room, a_id).await?;

    // B joins and publishes; capture its relay id.
    let (b_tx, mut b_rx) = tokio::sync::mpsc::channel(8);
    let b_id = Uuid::new_v4();
    let b_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b_client).await?);
    join(&b_client, &room, b_id, b_tx, &mut b_rx, &api, room_id).await?;
    wait_for_relay(&room, b_id).await?;
    let first_id = relay_id(&room, b_id)
      .await
      .expect("B relay after first join");

    // B leaves and rejoins under the same PeerId.
    handle_leave(room.clone(), b_id, room_id).await?;
    let (b2_tx, mut b2_rx) = tokio::sync::mpsc::channel(8);
    let b2_client = make_client_pc().await?;
    start_fake_mic(make_fake_mic(&b2_client).await?);
    join(&b2_client, &room, b_id, b2_tx, &mut b2_rx, &api, room_id).await?;
    wait_for_relay(&room, b_id).await?;
    let second_id = relay_id(&room, b_id).await.expect("B relay after rejoin");

    assert!(first_id.starts_with(&format!("audio-{b_id}")));
    assert!(second_id.starts_with(&format!("audio-{b_id}")));
    assert_ne!(
      first_id, second_id,
      "rejoining peer must get a distinct relay track id"
    );
    Ok(())
  }
}
