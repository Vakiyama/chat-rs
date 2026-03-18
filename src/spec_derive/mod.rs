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
  async fn get_room(&self, /* #[path] */ id: u64) -> Result<RoomResponse, ApiError> {
    Ok(RoomResponse {
      id,
      name: "general".into(),
    })
  }
}
