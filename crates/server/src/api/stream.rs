use std::{
  collections::HashMap,
  pin::Pin,
  sync::{Arc, Mutex},
};

use sea_orm::{EntityTrait, IntoActiveModel, QueryFilter};
use tokio::sync::OnceCell;

use chat_shared::{
  convert::{
    IntoProto, TryIntoDomain,
    stream::proto::{
      ClientTextMessage, ClientVoiceMessage, ServerTextMessage, ServerVoiceMessage,
      stream_service_server::StreamService,
    },
  },
  domain::{
    post::Post,
    stream::{ClientText, ClientVoice, ServerText, User},
  },
};
use std::time::SystemTime;
use tokio::sync::mpsc::{self};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::Response;
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
  interceptor::registry::Registry,
};

use crate::{
  config::CONFIG,
  entities,
  library::{
    database,
    webrtc::{
      Room, broadcast_presence, broadcast_server_presence, handle_answer, handle_leave,
      handle_offer, membership_snapshot, register_presence_subscriber,
      unregister_presence_subscriber,
    },
  },
};

// todo: add text_channel_id rooms to manager
#[derive(Default)]
struct Manager {
  sockets: HashMap<Uuid, mpsc::Sender<Result<ServerTextMessage, tonic::Status>>>,
}

#[derive(Default, Clone)]
pub struct StreamServer {
  manager: Arc<Mutex<Manager>>,
}

pub type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, tonic::Status>> + Send>>;

impl Manager {
  fn remove(&mut self, id: &Uuid) {
    self.sockets.remove(id);
  }

  fn add(&mut self, id: Uuid, sender: mpsc::Sender<Result<ServerTextMessage, tonic::Status>>) {
    println!("Adding new socket id: {id}");

    self.sockets.insert(id, sender);
  }

  // unused for now
  // async fn send(
  //   sender: &mpsc::Sender<Result<ServerTextMessage, tonic::Status>>,
  //   message: ServerTextMessage,
  // ) -> Result<(), SendError<Result<ServerTextMessage, tonic::Status>>> {
  //   sender.send(Ok(message)).await
  // }

  // fn targets(&self, from: &Uuid) -> Vec<mpsc::Sender<Result<ServerTextMessage, tonic::Status>>> {
  //   self
  //     .sockets
  //     .iter()
  //     .filter_map(|(id, sender)| {
  //       if id != from {
  //         Some(sender.clone())
  //       } else {
  //         None
  //       }
  //     })
  //     .collect()
  // }

  fn targets_with_self(&self) -> Vec<mpsc::Sender<Result<ServerTextMessage, tonic::Status>>> {
    self.sockets.values().cloned().collect()
  }

  /// Everyone but the given socket — used for typing notifications, which the
  /// sender shouldn't receive back (they'd see themselves "typing").
  fn targets_without_self(
    &self,
    me: &Uuid,
  ) -> Vec<mpsc::Sender<Result<ServerTextMessage, tonic::Status>>> {
    self
      .sockets
      .iter()
      .filter(|(id, _)| *id != me)
      .map(|(_, sender)| sender.clone())
      .collect()
  }
  /// broadcasts to all passed in targets
  async fn emit(
    targets: Vec<mpsc::Sender<Result<ServerTextMessage, tonic::Status>>>,
    message: ServerTextMessage,
  ) {
    for sender in &targets {
      let _ = sender.send(Ok(message.clone())).await;
    }
  }
}

// temporary room singleton

// static GLOBAL_ROOM: OnceLock<Arc<Room>> = OnceLock::new();

#[derive(Default)]
struct VoiceRoomManager {
  rooms: HashMap<Uuid, Arc<Room>>,
}

static ROOM_MANAGER: std::sync::LazyLock<tokio::sync::Mutex<VoiceRoomManager>> =
  std::sync::LazyLock::new(|| tokio::sync::Mutex::new(VoiceRoomManager::default()));

// Fetch the room for a voice channel, creating it (with its server_id resolved from
// the DB) on first use. The lock is held across the DB lookup so two concurrent
// joiners can't each create a separate Room for the same channel.
async fn get_or_create_room(voice_channel_id: Uuid) -> Arc<Room> {
  let mut room_manager = ROOM_MANAGER.lock().await;
  if let Some(room) = room_manager.rooms.get(&voice_channel_id) {
    return room.clone();
  }

  let db = database::get().await;
  let server_id = entities::voice_channel::Entity::find_by_id(voice_channel_id)
    .one(db)
    .await
    .ok()
    .flatten()
    .and_then(|vc| vc.server_id)
    .unwrap_or_default();

  let room: Arc<Room> = Room::new(voice_channel_id, server_id).into();
  room_manager.rooms.insert(voice_channel_id, room.clone());
  room
}

static WEBRTC_API: OnceCell<API> = OnceCell::const_new();

#[tonic::async_trait]
impl StreamService for StreamServer {
  type ConnectTextStreamStream = ResponseStream<ServerTextMessage>;
  type ConnectVoiceStreamStream = ResponseStream<ServerVoiceMessage>;

  async fn connect_voice_stream(
    &self,
    request: tonic::Request<tonic::Streaming<ClientVoiceMessage>>,
  ) -> Result<tonic::Response<Self::ConnectVoiceStreamStream>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied();

    let Some(request_user_id) = request_user_id else {
      return Err(tonic::Status::unauthenticated("Unauthenitcated"));
    };

    let db = database::get().await;
    let user = entities::user::Entity::find_by_id(request_user_id)
      .one(db)
      .await
      .unwrap()
      .unwrap();

    let mut inner_stream = request.into_inner();

    let (tx, rx) = mpsc::channel::<Result<ServerVoiceMessage, tonic::Status>>(128);

    // identifies this voice-stream connection in the server-wide presence registry
    let voice_conn_id = Uuid::new_v4();

    tokio::spawn(async move {
      while let Some(msg) = inner_stream.next().await {
        let msg: Result<ClientVoice, _> = msg.and_then(|msg| msg.try_into_domain());

        let api = WEBRTC_API
          .get_or_init(async || {
            // Create a_client MediaEngine object to configure the supported codec
            let mut media_engine = MediaEngine::default();

            media_engine.register_default_codecs().unwrap();

            // Create a_client InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
            // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
            // this is enabled by default. If you are manually managing You MUST create a_client InterceptorRegistry
            // for each PeerConnection.
            let mut registry = Registry::new();

            registry = register_default_interceptors(registry, &mut media_engine).unwrap();

            let mut api = APIBuilder::new()
              .with_media_engine(media_engine)
              .with_interceptor_registry(registry);

            if let Some(udp_port) = CONFIG.server.udp_port.clone() {
              let mut settings_engine = SettingEngine::default();

              settings_engine.set_network_types(vec![NetworkType::Udp4]);
              // settings_engine.set_nat_1to1_ips(vec![public_ip], RTCIceCandidateType::Host);
              settings_engine.set_udp_network(UDPNetwork::Muxed(UDPMuxDefault::new(
                UDPMuxParams::new(
                  tokio::net::UdpSocket::bind(format!("0.0.0.0:{udp_port}"))
                    .await
                    .unwrap(),
                ),
              )));

              api = api.with_setting_engine(settings_engine);
            }

            api.build()
          })
          .await;

        match msg {
          Ok(ClientVoice::Speaking {
            speaking,
            voice_channel_id,
          }) => {
            let room = get_or_create_room(voice_channel_id).await;

            {
              let mut peers = room.peers.write().await;

              let Some(peer) = peers.get_mut(&request_user_id) else {
                continue;
              };

              peer
                .speaking
                .store(speaking, std::sync::atomic::Ordering::Relaxed);
            }

            broadcast_presence(room.clone()).await;
          }
          Ok(ClientVoice::Offer {
            description: offer,
            voice_channel_id,
          }) => {
            let room = get_or_create_room(voice_channel_id).await;

            if !room.peers.read().await.contains_key(&request_user_id) {
              match handle_offer(
                offer,
                room.clone(),
                request_user_id,
                tx.clone(),
                api,
                voice_channel_id,
                User {
                  id: request_user_id,
                  name: user.username.clone(),
                },
              )
              .await
              {
                Ok(()) => {
                  println!("Success handling session description offer from client");
                }
                // todo; tear down clients if err
                Err(_) => eprintln!("Error when handling initial RTCSessionDescription offer..."),
              }
            } else {
              eprintln!("User offer rejected; already in room.");
            }
          }
          Ok(ClientVoice::Answer {
            description: answer,
            voice_channel_id,
          }) => {
            println!("received answer from peer {request_user_id}");

            let room = get_or_create_room(voice_channel_id).await;

            let _ = handle_answer(room.clone(), request_user_id, answer, voice_channel_id).await;
          }
          Ok(ClientVoice::LeaveRoom { voice_channel_id }) => {
            let room = get_or_create_room(voice_channel_id).await;

            let _ = handle_leave(room.clone(), request_user_id, voice_channel_id)
              .await
              .map_err(|err| println!("Error handling leave: {err}"));
          }
          Ok(ClientVoice::SetMuted {
            muted,
            voice_channel_id,
          }) => {
            let room = get_or_create_room(voice_channel_id).await;

            {
              let peers = room.peers.read().await;
              let Some(peer) = peers.get(&request_user_id) else {
                continue;
              };
              peer
                .muted
                .store(muted, std::sync::atomic::Ordering::Relaxed);
            }

            broadcast_presence(room.clone()).await;
            broadcast_server_presence(room.clone()).await;
          }
          Ok(ClientVoice::SetDeafened {
            deafened,
            voice_channel_id,
          }) => {
            let room = get_or_create_room(voice_channel_id).await;

            {
              let peers = room.peers.read().await;
              let Some(peer) = peers.get(&request_user_id) else {
                continue;
              };
              peer
                .deafened
                .store(deafened, std::sync::atomic::Ordering::Relaxed);
            }

            broadcast_presence(room.clone()).await;
            broadcast_server_presence(room.clone()).await;
          }
          Ok(ClientVoice::SubscribeServer { server_id }) => {
            // register this connection's interest in the server, then dump a
            // snapshot of every active call in it so the client loads current
            // presence immediately.
            register_presence_subscriber(voice_conn_id, request_user_id, server_id, tx.clone());

            let rooms: Vec<Arc<Room>> = {
              let room_manager = ROOM_MANAGER.lock().await;
              room_manager.rooms.values().cloned().collect()
            };

            for room in rooms {
              if room.server_id != server_id {
                continue;
              }
              {
                let peers = room.peers.read().await;
                // skip empty rooms, and rooms we're in (we get the richer in-call
                // snapshot there, which the membership snapshot would clobber).
                if peers.is_empty() || peers.contains_key(&request_user_id) {
                  continue;
                }
              }
              let _ = tx.send(Ok(membership_snapshot(&room).await)).await;
            }
          }
          Err(err) => {
            eprint!("Error in incoming client message: {err:?}")
            // break;
          }
        }
      }

      // grpc msg loop ended, remove peer from any rooms, close pc

      unregister_presence_subscriber(&voice_conn_id);

      let rooms: Vec<(Uuid, Arc<Room>)> = {
        let room_manager = ROOM_MANAGER.lock().await;
        room_manager
          .rooms
          .iter()
          .map(|(id, room)| (*id, room.clone()))
          .collect()
      };
      for (id, room) in rooms {
        let _ = handle_leave(room.clone(), request_user_id, id)
          .await
          .map_err(|err| println!("Error handling leave: {err}"));
      }
    });

    let output_stream = ReceiverStream::new(rx);

    Ok(Response::new(
      Box::pin(output_stream) as Self::ConnectVoiceStreamStream
    ))
  }

  async fn connect_text_stream(
    &self,
    request: tonic::Request<tonic::Streaming<ClientTextMessage>>,
  ) -> Result<tonic::Response<Self::ConnectTextStreamStream>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied();
    let db = database::get().await;

    let user = entities::user::Entity::find_by_id(request_user_id.unwrap())
      .one(db)
      .await
      .unwrap()
      .unwrap();

    let mut inner_stream = request.into_inner();

    let (tx, rx) = mpsc::channel(128);

    let socket_id = Uuid::new_v4();
    let manager = self.manager.clone();
    let pong_tx = tx.clone();
    manager.lock().unwrap().add(socket_id, tx);
    let manager = manager.clone();

    tokio::spawn(async move {
      while let Some(msg) = inner_stream.next().await {
        let domain_msg: Result<ClientText, _> = msg.and_then(|msg| msg.try_into_domain());

        match domain_msg {
          Ok(ClientText::CreatePostRequest {
            id,
            content,
            text_channel_id,
          }) => {
            let targets = manager.lock().unwrap().targets_with_self();

            let server_msg = ServerText::Post(Post {
              id,
              author_name: user.username.clone(),
              content: content.clone(),
              created_at: chrono::Utc::now(),
            });

            let channel = entities::channel::Entity::find()
              .filter(
                entities::channel::COLUMN
                  .text_channel_id
                  .eq(text_channel_id),
              )
              .one(db)
              .await
              .unwrap();

            let channel_id = channel.unwrap().id;

            entities::post::Entity::insert(
              entities::post::Model {
                id: uuid::Uuid::new_v4(),
                content,
                channel_id: Some(channel_id),
                author_id: Some(user.id),
                created_at: chrono::Utc::now(),
              }
              .into_active_model(),
            )
            .exec(db)
            .await
            .map_err(|err| {
              eprintln!("error inserting post: {err}");
              tonic::Status::internal("Error occurred inserting post.")
            })
            .unwrap();

            Manager::emit(targets, server_msg.into_proto()).await;
          }
          Ok(ClientText::Ping { timestamp }) => {
            let server_received_at = SystemTime::now()
              .duration_since(SystemTime::UNIX_EPOCH)
              .unwrap_or_default()
              .as_micros() as u64;
            let _ = pong_tx
              .send(Ok(
                ServerText::Pong {
                  timestamp,
                  server_received_at,
                }
                .into_proto(),
              ))
              .await;
          }
          Ok(ClientText::Typing { text_channel_id }) => {
            // fan out to everyone else; the sender is excluded so they don't see
            // themselves typing. Transient — not persisted, no DB work.
            let targets = manager.lock().unwrap().targets_without_self(&socket_id);
            let server_msg = ServerText::Typing {
              from: User {
                id: user.id,
                name: user.username.clone(),
              },
              text_channel_id,
            };
            Manager::emit(targets, server_msg.into_proto()).await;
          }
          Err(err) => {
            eprint!("Error in incoming client message: {err:?}")
            // break;
          }
        }
      }

      manager.lock().unwrap().remove(&socket_id);
    });

    let output_stream = ReceiverStream::new(rx);

    Ok(Response::new(
      Box::pin(output_stream) as Self::ConnectTextStreamStream
    ))
  }
}
