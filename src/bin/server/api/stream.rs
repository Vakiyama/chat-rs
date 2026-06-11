use std::{
  collections::HashMap,
  pin::Pin,
  sync::{Arc, Mutex, OnceLock},
};

use chat_rs::shared::{
  convert::{
    IntoProto, TryIntoDomain,
    stream::proto::{
      ClientTextMessage, ClientVoiceMessage, ServerTextMessage, ServerVoiceMessage,
      stream_service_server::StreamService,
    },
  },
  domain::stream::{ClientText, ClientVoice, ServerText, ServerVoice},
};
use tokio::sync::mpsc::{self, error::SendError};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::Response;
use uuid::Uuid;

use crate::library::webrtc::{Room, handle_answer, handle_offer};

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

  fn targets(&self, from: &Uuid) -> Vec<mpsc::Sender<Result<ServerTextMessage, tonic::Status>>> {
    self
      .sockets
      .iter()
      .filter_map(|(id, sender)| {
        if id != from {
          Some(sender.clone())
        } else {
          None
        }
      })
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

static GLOBAL_ROOM: OnceLock<Arc<Room>> = OnceLock::new();

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

    // eventually setup rooms, for now, one global room
    let room = GLOBAL_ROOM.get_or_init(|| {
      Room {
        peers: HashMap::new().into(),
      }
      .into()
    });

    let mut inner_stream = request.into_inner();

    let (tx, rx) = mpsc::channel::<Result<ServerVoiceMessage, tonic::Status>>(128);

    tokio::spawn(async move {
      while let Some(msg) = inner_stream.next().await {
        let msg: Result<ClientVoice, _> = msg.and_then(|msg| msg.try_into_domain());

        match msg {
          Ok(ClientVoice::Offer(desc)) => {
            if !room.peers.read().await.contains_key(&request_user_id) {
              match handle_offer(desc, room.clone(), request_user_id, tx.clone()).await {
                Ok(answer) => {
                  let _ = tx.send(Ok(ServerVoice::Answer(answer).into_proto())).await;
                }
                Err(_) => eprintln!("Error when handling initial RTCSessionDescription offer..."),
              }
            } else {
              eprintln!("User offer rejected; already in room.");
            }
          }
          Ok(ClientVoice::Answer(answer)) => {
            println!("received answer from peer {request_user_id}");
            let _ = handle_answer(room.clone(), request_user_id, answer).await;
          }
          Err(err) => {
            eprint!("Error in incoming client message: {err:?}")
            // break;
          }
        }
      }

      // remove peer from room, close pc
      if let Some(peer) = room.peers.write().await.remove(&request_user_id) {
        let _ = peer.pc.close().await;
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

    let mut inner_stream = request.into_inner();

    let (tx, rx) = mpsc::channel(128);

    let socket_id = Uuid::new_v4();
    let manager = self.manager.clone();
    manager.lock().unwrap().add(socket_id, tx);

    tokio::spawn(async move {
      while let Some(msg) = inner_stream.next().await {
        match msg {
          Ok(msg) => {
            // handler: atm, we only deal with chat message relaying and ignore others
            if let Ok(ClientText::ChatMessage { from, text }) = msg.try_into_domain() {
              let targets = manager.lock().unwrap().targets(&socket_id);

              if let Some(user_id) = request_user_id
                && from.id == user_id
              {
                let server_msg = ServerText::ChatMessage { from, text };

                Manager::emit(targets, server_msg.into_proto()).await;
              };
            }
          }
          Err(err) => {
            eprint!("Error in incoming client message: {err:?}")
            // break;
          }
        }
      }
    });

    let output_stream = ReceiverStream::new(rx);

    Ok(Response::new(
      Box::pin(output_stream) as Self::ConnectTextStreamStream
    ))
  }
}
