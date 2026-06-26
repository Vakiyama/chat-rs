use crate::{entities, library::database};
use chat_shared::{
  convert::{
    IntoProto, TryIntoDomain,
    server::proto::{
      ServersResponse as ServersResponseProto, SetChannelMuteRequest as SetChannelMuteRequestProto,
      server_service_server::ServerService,
    },
  },
  domain::server::{
    Channel,
    ChannelType::{Text, Voice},
    Server, ServersResponse as DomainServersResponse, SetChannelMuteRequest,
  },
};
use sea_orm::{EntityTrait, IntoActiveModel, LoaderTrait, QueryFilter};
use std::collections::HashSet;
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

    let muted: HashSet<Uuid> = entities::user_text_channel::Entity::find()
      .filter(entities::user_text_channel::COLUMN.user_id.eq(request_user_id))
      .all(db)
      .await
      .map_err(|err| {
        eprintln!("error fetching muted channels: {err}");
        tonic::Status::internal("Error occurred fetching servers.")
      })?
      .into_iter()
      .map(|row| row.text_channel_id)
      .collect();

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
            muted: false,
          })
          .collect();
        let text_channels: Vec<Channel> = text_channel
          .into_iter()
          .map(|tc| Channel {
            muted: muted.contains(&tc.id),
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

  async fn set_channel_mute(
    &self,
    request: tonic::Request<SetChannelMuteRequestProto>,
  ) -> Result<tonic::Response<()>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied().unwrap();
    let SetChannelMuteRequest {
      text_channel_id,
      muted,
    } = request.into_inner().try_into_domain()?;

    let db = database::get().await;

    if muted {
      let already_muted = entities::user_text_channel::Entity::find()
        .filter(entities::user_text_channel::COLUMN.user_id.eq(request_user_id))
        .filter(entities::user_text_channel::COLUMN.text_channel_id.eq(text_channel_id))
        .one(db)
        .await
        .map_err(|err| {
          eprintln!("error checking channel mute: {err}");
          tonic::Status::internal("Error occurred updating mute.")
        })?
        .is_some();

      if !already_muted {
        entities::user_text_channel::Entity::insert(
          entities::user_text_channel::Model {
            user_id: request_user_id,
            text_channel_id,
          }
          .into_active_model(),
        )
        .exec(db)
        .await
        .map_err(|err| {
          eprintln!("error muting channel: {err}");
          tonic::Status::internal("Error occurred updating mute.")
        })?;
      }
    } else {
      entities::user_text_channel::Entity::delete_many()
        .filter(entities::user_text_channel::COLUMN.user_id.eq(request_user_id))
        .filter(entities::user_text_channel::COLUMN.text_channel_id.eq(text_channel_id))
        .exec(db)
        .await
        .map_err(|err| {
          eprintln!("error unmuting channel: {err}");
          tonic::Status::internal("Error occurred updating mute.")
        })?;
    }

    Ok(tonic::Response::new(()))
  }
}
