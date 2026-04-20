use std::sync::{Arc, Mutex};

use axum::Router;
use chat_rs::SERVER_URL;
use tower_http::trace::TraceLayer;

mod api;
mod library;
mod websocket;

#[tokio::main]
async fn main() {
  let _env = dotenvy::dotenv();
  let (tx, _rx) = tokio::sync::mpsc::channel(32);
  let state = Arc::new(websocket::State { tx });

  let manager = Arc::new(Mutex::new(websocket::Manager::default()));

  // initialize the tracing subscriber — do this first
  tracing_subscriber::fmt()
    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
    .init();

  let app = Router::new()
    .route(
      "/",
      axum::routing::any(|socket, state| websocket::ws_handler(socket, state, manager)),
    )
    .with_state(state)
    .nest("/api", api::router())
    .layer(TraceLayer::new_for_http());

  let listener = tokio::net::TcpListener::bind(SERVER_URL).await.unwrap();

  println!("Listening on {SERVER_URL}");
  axum::serve(listener, app).await.unwrap();
}
