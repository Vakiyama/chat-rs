use futures_util::TryFutureExt;
use std::{
  collections::HashMap,
  marker::PhantomData,
  sync::{Arc, Mutex},
};

use bytes::Bytes;
use chat_rs::WebSocketMessage;
use futures_util::{SinkExt, stream::StreamExt};
use rkyv::rancor;
use tokio::sync::mpsc;
use uuid::Uuid;

type Sender<T> = tokio::sync::mpsc::Sender<T>;

pub struct State {
  pub tx: Sender<Bytes>,
}

#[derive(Clone, Copy)]
struct Id<Type> {
  value: Uuid,
  _marker: PhantomData<Type>,
}

impl<T> Default for Id<T> {
  fn default() -> Self {
    Self {
      value: Uuid::new_v4(),
      _marker: Default::default(),
    }
  }
}

impl<T> PartialEq for Id<T> {
  fn eq(&self, other: &Self) -> bool {
    self.value == other.value
  }
}

enum Error {
  Send(tokio::sync::mpsc::error::SendError<Bytes>),
  Decode(rancor::Error),
}

#[derive(Default)]
pub struct Manager {
  sockets: HashMap<Uuid, mpsc::Sender<Bytes>>,
}

impl Manager {
  fn remove(&mut self, id: &Uuid) {
    self.sockets.remove(id);
  }

  fn add(&mut self, id: Uuid, sender: Sender<Bytes>) {
    println!("Adding new socket id: {id}");

    self.sockets.insert(id, sender);
  }

  async fn send(sender: &Sender<Bytes>, message: &WebSocketMessage) -> Result<(), Error> {
    let bytes = message.try_into().map_err(Error::Decode);

    sender.send(bytes?).await.map_err(Error::Send)
  }

  // /// emits messages to all sockets in manager
  // async fn broadcast(&self, message: &WebSocketMessage) -> Vec<Result<(), Error>> {
  //   self
  //     .sockets
  //     .values()
  //     .map(async |socket| Manager::send(socket, message).await)
  //     .collect::<Vec<Result<(), _>>>()
  // }

  fn targets(&self, from: &Uuid) -> Vec<Sender<Bytes>> {
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
  async fn emit(targets: Vec<Sender<Bytes>>, message: &WebSocketMessage) {
    for sender in &targets {
      if let Ok(bytes) = Bytes::try_from(message) {
        let _ = sender.send(bytes).await;
      }
    }
  }
}

pub async fn ws_handler(
  ws: axum::extract::ws::WebSocketUpgrade,
  axum::extract::State(_state): axum::extract::State<std::sync::Arc<State>>,
  manager: Arc<Mutex<Manager>>,
) -> impl axum::response::IntoResponse {
  ws.on_upgrade(|socket| handle_socket(socket, manager))
}

async fn handle_socket(socket: axum::extract::ws::WebSocket, manager: Arc<Mutex<Manager>>) {
  let (mut sender, mut receiver) = socket.split();

  let (tx, mut rx) = mpsc::channel::<Bytes>(32);

  let mut task_send = tokio::spawn(async move {
    while let Some(msg) = rx.recv().await {
      if sender
        .send(axum::extract::ws::Message::binary(msg))
        .await
        .is_err()
      {
        break;
      }
    }
  });

  let socket_id = Uuid::new_v4();

  manager.lock().unwrap().add(socket_id, tx.clone());

  let mut task_recv = tokio::spawn(async move {
    while let Some(msg) = receiver.next().await {
      match msg {
        Ok(axum::extract::ws::Message::Binary(binary)) => {
          let client_msg: Result<WebSocketMessage, _> = binary.try_into();

          // i'm deserializing the msg here because we'll use it later surely
          if let Ok(client_msg) = client_msg {
            println!("Received msg from socket id: {socket_id}");

            // we only need manager to get the targets; the emit uses those senders
            // therefore, we can get the targets and drop the lock;
            // the lock cannot cross the async boundary, so by dropping it, we can await the
            // emit further below
            // if we needed to use a lock that implements send, we'd have to use
            // tokio::sync::Mutex.
            let targets = manager.lock().unwrap().targets(&socket_id);

            Manager::emit(targets, &client_msg).await;
          } else {
            println!("Error when echoing msg: {client_msg:?}")
          }
        }
        Ok(axum::extract::ws::Message::Close(_)) => break,
        Ok(other) => println!("Received: {other:?}"),
        Err(e) => {
          println!("Error: {e}");
          manager.lock().unwrap().remove(&socket_id);
          break;
        }
      }
    }
  });

  tokio::select! {
      _ = &mut task_send => task_recv.abort(),
      _ = &mut task_recv => task_send.abort(),
  }

  // let _ = state.tx.send("A user left!".into());
}
