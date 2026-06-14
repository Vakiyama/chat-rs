use chat_shared::{
  convert::{
    IntoProto, server::proto::ServersResponse as ServersResponseProto,
    server::proto::server_service_server::ServerService,
  },
  domain::server::ServersResponse as DomainServersResponse,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::{entities, library::database};

pub struct ServerServer;

#[tonic::async_trait]
impl ServerService for ServerServer {
  async fn servers(
    &self,
    request: tonic::Request<()>,
  ) -> Result<tonic::Response<ServersResponseProto>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied();

    todo!()
  }
}
