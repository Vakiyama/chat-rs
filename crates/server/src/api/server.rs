use crate::{entities, library::database};
use chat_shared::{
  convert::{
    IntoProto,
    server::proto::{
      ServersResponse as ServersResponseProto, server_service_server::ServerService,
    },
  },
  domain::server::{
    Channel,
    ChannelType::{Text, Voice},
    Server, ServersResponse as DomainServersResponse,
  },
};
use sea_orm::QueryFilter;
use uuid::Uuid;

pub struct ServerServer;

#[tonic::async_trait]
impl ServerService for ServerServer {
  async fn servers(
    &self,
    request: tonic::Request<()>,
  ) -> Result<tonic::Response<ServersResponseProto>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied().unwrap();
    let db = database::get().await;

    let servers = entities::server::Entity::load()
      .with(entities::user::Entity)
      .with(entities::text_channel::Entity)
      .with(entities::voice_channel::Entity)
      .filter(entities::user::COLUMN.id.eq(request_user_id))
      .all(db)
      .await
      .map_err(|err| {
        eprintln!("error fetching servers: {err}");
        tonic::Status::internal("Error occurred fetching servers.")
      })?
      .into_iter()
      .map(|server| {
        let voice_channels: Vec<Channel> = server
          .voice_channel
          .into_iter()
          .map(|vc| Channel {
            id: vc.id,
            name: vc.name,
            r#type: Voice,
          })
          .collect();

        let text_channels: Vec<Channel> = server
          .text_channel
          .into_iter()
          .map(|tc| Channel {
            id: tc.id,
            name: tc.name,
            r#type: Text,
          })
          .collect();

        Server {
          id: server.id,
          name: server.name,
          channels: voice_channels.into_iter().chain(text_channels).collect(),
        }
      })
      .collect();

    Ok(tonic::Response::new(
      DomainServersResponse { servers }.into_proto(),
    ))
  }
}
