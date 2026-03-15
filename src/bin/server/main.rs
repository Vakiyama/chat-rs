use std::sync::{Arc, Mutex};

use chat_rs::SERVER_URL;
use utoipa_axum::router::OpenApiRouter;

mod api;
mod library;
mod websocket;

#[tokio::main]
async fn main() {
  let (tx, _rx) = tokio::sync::mpsc::channel(32);
  let state = Arc::new(websocket::State { tx });

  let manager = Arc::new(Mutex::new(websocket::Manager::default()));

  let (app, mut api_spec) = OpenApiRouter::new()
    .route(
      "/",
      axum::routing::any(|socket, state| websocket::ws_handler(socket, state, manager)),
    )
    .with_state(state)
    .nest("/api", api::router())
    .split_for_parts();

  let app = app.route(
    "/api-docs/openapi.json",
    axum::routing::get({
      api_spec.info.title = "ChatRS API".into();
      api_spec.info.description = Some("Documentation for the ChatRS open api".into());
      move || async move { axum::Json(api_spec) }
    }),
  );

  let listener = tokio::net::TcpListener::bind(SERVER_URL).await.unwrap();

  println!("Listening on {SERVER_URL}");
  axum::serve(listener, app).await.unwrap();
}
