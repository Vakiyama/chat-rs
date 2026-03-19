use axum::{Json, response::IntoResponse};
use spec_derive::{client, generate};
use spec_derive_core::E;

#[derive(serde::Serialize)]
struct Room {
  id: u64,
  name: String,
}

type RoomResponse = axum::Json<Room>;

enum ApiError {
  Internal,
  BadId,
}

impl IntoResponse for ApiError {
  fn into_response(self) -> axum::response::Response {
    todo!()
  }
}

trait RoomsApi {
  async fn get_room(&self, id: u64) -> Result<RoomResponse, ApiError>;

  async fn create_room(&self, new_id: u64, name: String) -> Result<RoomResponse, ApiError>;
}

#[client]
pub struct Api;

#[generate]
impl RoomsApi for Api {
  #[http(GET, "/room")]
  async fn get_room(&self, #[query] id: u8) -> Result<RoomResponse, ApiError> {
    Ok(Json(Room {
      id: id.into(),
      name: "general".into(),
    }))
  }

  #[http(POST, "/create")]
  async fn create_room(&self, #[json] name: String) -> Result<RoomResponse, ApiError> {
    Ok(Json(Room {
      id: u64::default(),
      name,
    }))
  }
}

fn test() {
  let client = Api::new("localhost:3000");
  todo!()
}
