use chat_rs::SERVER_URL;
use futures_util::{SinkExt, stream::StreamExt};

struct State {
  tx: tokio::sync::broadcast::Sender<String>,
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

  let _ = state.tx.send("New user joined!".to_string());

  let mut task_send = tokio::spawn(async move {
    while let Ok(msg) = rx.recv().await {
      if sender
        .send(axum::extract::ws::Message::text(msg))
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
        Ok(axum::extract::ws::Message::Text(text)) => {
          let _ = tx.send(text.to_string());
          println!("{text}");
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

  let _ = state.tx.send("A user left!".to_string());
}
