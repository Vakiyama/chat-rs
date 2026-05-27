use std::sync::{Arc, Mutex};

use axum::Router;
use chat_rs::{
  SERVER_URL, WS_PORT, WS_URL, shared::convert::proto::auth::auth_service_server::AuthServiceServer,
};
use tonic::transport::Server;

use crate::api::auth::{AuthServer, InMemoryCodeStore, InMemoryTokenStore};

mod api;
mod library;
mod websocket;

#[tokio::main]
async fn main() {
  let _env = dotenvy::dotenv();

  let (tx, _rx) = tokio::sync::mpsc::channel(32);
  let state = Arc::new(websocket::State { tx });
  let manager = Arc::new(Mutex::new(websocket::Manager::default()));

  let auth_service: AuthServer<InMemoryCodeStore, InMemoryTokenStore> = AuthServer::default();

  let grpc = Server::builder()
    .add_service(AuthServiceServer::new(auth_service))
    .serve(SERVER_URL.parse().unwrap());

  let app = Router::new()
    .route(
      "/",
      axum::routing::any(|socket, state| websocket::ws_handler(socket, state, manager)),
    )
    .with_state(state);
  let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{WS_PORT}"))
    .await
    .unwrap();

  println!("gRPC listening on: {SERVER_URL}");
  println!("WS listening on: {WS_URL}");

  // run both forever, bail if either fails
  tokio::select! {
      res = grpc => { eprintln!("gRPC exited: {:?}", res); }
      res = axum::serve(listener, app) => { eprintln!("WS exited: {:?}", res); }
  }
}
