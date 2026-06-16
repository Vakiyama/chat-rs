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
use sea_orm::{EntityTrait, LoaderTrait, QueryFilter};
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

    // INFO grpc_request{method=/server.v1.ServerService/Servers}: sqlx::query: summary="SELECT \"server\".\"id\", \"server\".\"name\" FROM …" db.statement="\n\nSELECT \"server\".\"id\", \"server\".\"name\" FROM \"server\" WHERE \"user\".\"id\" = $1\n" rows_affected=0 rows_returned=0 elapsed=345.751µs elapsed_secs=0.000345751
    // error fetching servers: Query Error: error returned from database: missing FROM-clause entry for table "user"

    let server_models = entities::server::Entity::find()
      .inner_join(entities::user::Entity)
      .filter(entities::user::COLUMN.id.eq(request_user_id))
      .all(db)
      .await
      .map_err(|err| {
        eprintln!("error fetching servers: {err}");
        tonic::Status::internal("Error occurred fetching servers.")
      })?;

    let voice = server_models
      .load_many(entities::voice_channel::Entity, db)
      .await
      .map_err(|err| {
        eprintln!("error fetching voice channels: {err}");
        tonic::Status::internal("Error occurred fetching servers.")
      })?;
    let text = server_models
      .load_many(entities::text_channel::Entity, db)
      .await
      .map_err(|err| {
        eprintln!("error fetching text channels: {err}");
        tonic::Status::internal("Error occurred fetching servers.")
      })?;

    let servers = server_models
      .into_iter()
      .zip(voice)
      .zip(text)
      .map(|((server, voice_channel), text_channel)| {
        let voice_channels: Vec<Channel> = voice_channel
          .into_iter()
          .map(|vc| Channel {
            id: vc.id,
            name: vc.name,
            r#type: Voice,
          })
          .collect();
        let text_channels: Vec<Channel> = text_channel
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
