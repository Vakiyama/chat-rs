use std::{
  collections::HashMap,
  pin::Pin,
  sync::{Arc, Mutex},
};

use chat_rs::shared::{
  convert::{
    IntoProto, TryIntoDomain,
    stream::proto::{ClientMessage, ServerMessage, stream_service_server::StreamService},
  },
  domain::stream::{Client, Server},
};
use tokio::sync::mpsc::{self, error::SendError};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::Response;
use uuid::Uuid;

#[derive(Default)]
struct Manager {
  sockets: HashMap<Uuid, mpsc::Sender<Result<ServerMessage, tonic::Status>>>,
}

#[derive(Default, Clone)]
pub struct StreamServer {
  manager: Arc<Mutex<Manager>>,
}

pub type ResponseStream = Pin<Box<dyn Stream<Item = Result<ServerMessage, tonic::Status>> + Send>>;

impl Manager {
  fn remove(&mut self, id: &Uuid) {
    self.sockets.remove(id);
  }

  fn add(&mut self, id: Uuid, sender: mpsc::Sender<Result<ServerMessage, tonic::Status>>) {
    println!("Adding new socket id: {id}");

    self.sockets.insert(id, sender);
  }

  // unused for now
  // async fn send(
  //   sender: &mpsc::Sender<Result<ServerMessage, tonic::Status>>,
  //   message: ServerMessage,
  // ) -> Result<(), SendError<Result<ServerMessage, tonic::Status>>> {
  //   sender.send(Ok(message)).await
  // }

  fn targets(&self, from: &Uuid) -> Vec<mpsc::Sender<Result<ServerMessage, tonic::Status>>> {
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
    targets: Vec<mpsc::Sender<Result<ServerMessage, tonic::Status>>>,
    message: ServerMessage,
  ) {
    for sender in &targets {
      let _ = sender.send(Ok(message.clone())).await;
    }
  }
}

#[tonic::async_trait]
impl StreamService for StreamServer {
  type ConnectStreamStream = ResponseStream;

  async fn connect_stream(
    &self,
    request: tonic::Request<tonic::Streaming<ClientMessage>>,
  ) -> Result<tonic::Response<Self::ConnectStreamStream>, tonic::Status> {
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
            if let Ok(Client::ChatMessage { from, text }) = msg.try_into_domain() {
              let targets = manager.lock().unwrap().targets(&socket_id);

              let server_msg = Server::ChatMessage { from, text };

              Manager::emit(targets, server_msg.into_proto()).await;
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
      Box::pin(output_stream) as Self::ConnectStreamStream
    ))
  }
}
