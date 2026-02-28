use std::sync::{Arc, Mutex};

use chat_rs::SERVER_URL;
mod websocket;

#[tokio::main]
async fn main() {
  let (tx, _rx) = tokio::sync::mpsc::channel(32);
  let state = Arc::new(websocket::State { tx });

  let manager = Arc::new(Mutex::new(websocket::Manager::default()));

  let app = axum::Router::new()
    .route(
      "/",
      axum::routing::any(|socket, state| websocket::ws_handler(socket, state, manager)),
    )
    .with_state(state);

  let listener = tokio::net::TcpListener::bind(SERVER_URL).await.unwrap();

  println!("Listening on {SERVER_URL}");
  axum::serve(listener, app).await.unwrap();
}
