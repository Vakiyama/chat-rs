use chat_shared::{
  convert::{
    IntoProto,
    user::proto::{MeResponse, user_service_server::UserService},
  },
  domain::user::MeReturn,
};
use sea_orm::EntityTrait;
use uuid::Uuid;

use crate::{entities, library::database};

pub struct UserServer;

#[tonic::async_trait]
impl UserService for UserServer {
  async fn me(
    &self,
    request: tonic::Request<()>,
  ) -> Result<tonic::Response<MeResponse>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied();

    if request_user_id.is_none() {
      return Err(tonic::Status::not_found("User not found."));
    }

    let db = database::get().await;

    let user = entities::user::Entity::find_by_id(request_user_id.unwrap())
      .one(db)
      .await
      .map_err(|e| {
        eprintln!("Error fetching user: {e}");
        tonic::Status::internal("Unknown error occurred")
      })?;

    if user.is_none() {
      return Err(tonic::Status::not_found("User not found."));
    }

    let user = user.unwrap();

    Ok(tonic::Response::new(
      MeReturn {
        user_id: user.id,
        username: user.username,
      }
      .into_proto(),
    ))
  }
}
