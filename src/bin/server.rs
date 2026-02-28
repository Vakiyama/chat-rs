use std::any::Any;

use bytes::Bytes;
use chat_rs::SERVER_URL;
use chat_rs::{ClientMessage, ServerMessage};
use futures_util::{SinkExt, stream::StreamExt};
use rkyv::rancor::Error;

struct State {
  tx: tokio::sync::broadcast::Sender<Bytes>,
}

#[tokio::main]
async fn main() {
  let (tx, _rx) = tokio::sync::broadcast::channel(32);
  let state = std::sync::Arc::new(State { tx });

  let app = axum::Router::new()
    .route("/", axum::routing::any(ws_handler))
    .with_state(state);

  let listener = tokio::net::TcpListener::bind(SERVER_URL).await.unwrap();

  println!("Listening on {SERVER_URL}");
  axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(
  ws: axum::extract::ws::WebSocketUpgrade,
  axum::extract::State(state): axum::extract::State<std::sync::Arc<State>>,
) -> impl axum::response::IntoResponse {
  ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: axum::extract::ws::WebSocket, state: std::sync::Arc<State>) {
  if socket
    .send(axum::extract::ws::Message::Ping(
      axum::body::Bytes::from_static(&[1, 2, 3]),
    ))
    .await
    .is_ok()
  {
    println!("Pinged!");
  } else {
    println!("Ping Failed!");
    return;
  }

  let (mut sender, mut receiver) = socket.split();

  let mut rx = state.tx.subscribe();

  let mut task_send = tokio::spawn(async move {
    while let Ok(msg) = rx.recv().await {
      if sender
        .send(axum::extract::ws::Message::binary(msg))
        .await
        .is_err()
      {
        break;
      }
    }
  });

  let tx = state.tx.clone();

  let mut task_recv = tokio::spawn(async move {
    while let Some(msg) = receiver.next().await {
      match msg {
        Ok(axum::extract::ws::Message::Binary(binary)) => {
          let client_msg: Result<ClientMessage, _> = binary.try_into();
          if let Ok(client_msg) = client_msg {
            let echo_message = match client_msg {
              ClientMessage::JoinedRoom { from } => ServerMessage::JoinedRoom { from },
              ClientMessage::LeftRoom { from } => ServerMessage::LeftRoom { from },
              ClientMessage::Chat { from, text } => ServerMessage::Chat { from, text },
            };
            println!("echo msg received {echo_message:?}");

            let _ = tx.send(echo_message.try_into().unwrap());
          } else {
            println!("Error when echoing msg: {client_msg:?}")
          }
        }
        Ok(axum::extract::ws::Message::Close(_)) => break,
        Ok(other) => println!("Received: {other:?}"),
        Err(e) => {
          println!("Error: {e}");
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
