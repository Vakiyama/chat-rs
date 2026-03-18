use spec_derive::generate;

struct Api;

struct RoomResponse {
  id: u64,
  name: String,
}

enum ApiError {
  Internal,
  BadId,
}

trait RoomsApi {
  async fn get_room(&self, id: u64) -> Result<RoomResponse, ApiError>;
}

#[generate]
impl RoomsApi for Api {
  async fn get_room(&self, id: u64) -> Result<RoomResponse, ApiError> {
    Ok(RoomResponse {
      id,
      name: "general".into(),
    })
  }

  #[http(POST, "/create")]
  async fn create_room(&self, new_id: u64, #[body] name: String) -> Result<RoomResponse, ApiError> {
    Ok(RoomResponse { id: new_id, name })
  }
}
