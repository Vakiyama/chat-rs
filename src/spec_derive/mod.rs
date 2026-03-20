use std::fmt::Display;
use tower_http::trace::TraceLayer;

use axum::{Json, response::IntoResponse};
use spec_derive::{client, generate};
use tower::{ServiceBuilder, timeout::TimeoutLayer};

#[derive(serde::Serialize, serde::Deserialize)]
struct Room {
  id: u64,
  name: String,
}

impl IntoResponse for Room {
  fn into_response(self) -> axum::response::Response {
    todo!()
  }
}

#[derive(serde::Deserialize, Debug)]
enum ApiError {
  Internal,
  BadId,
}

impl std::fmt::Display for ApiError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    todo!()
  }
}

impl IntoResponse for ApiError {
  fn into_response(self) -> axum::response::Response {
    todo!()
  }
}

#[allow(dead_code)]
trait RoomsApi {
  async fn get_room(&self, id: u64) -> Result<Room, ApiError>;

  async fn create_room(&self, new_id: u64, name: String) -> Result<Room, ApiError>;
}

#[client]
pub struct Api;

#[generate]
impl RoomsApi for Api {
  #[http(GET, "/room")]
  async fn get_room(&self, #[query] id: u8) -> Result<Room, ApiError> {
    Ok(Room {
      id: id.into(),
      name: "general".into(),
    })
  }

  #[layer]
  fn with_timeout<T>() -> tower::ServiceBuilder<
    tower::layer::util::Stack<
      tower_http::trace::TraceLayer<
        tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
      >,
      tower::layer::util::Identity,
    >,
  > {
    ServiceBuilder::new().layer(TraceLayer::new_for_http())
  }

  #[http(POST, "/create")]
  async fn create_room(&self, #[json] name: String) -> Result<Room, ApiError> {
    Ok(Room {
      id: u64::default(),
      name,
    })
  }
}

// fn test() {
//   let client = Api::new("localhost:3000");
//   let _ = client.create_room("me".into());
//   let _ = client.get_room(2);
//   todo!()
// }

//
