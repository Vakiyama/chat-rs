use chat_shared::{
  convert::{
    IntoProto, TryIntoDomain,
    post::proto::{GetPostsResponse, posts_service_server::PostsService},
  },
  domain::post::{
    CreatePostCommand, GetPostsRequest, GetPostsResponse as DomainGetPostsResponse, Post,
  },
};
use sea_orm::{EntityTrait, ExprTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::{entities, library::database};

pub struct PostsServer;

#[tonic::async_trait]
impl PostsService for PostsServer {
  async fn create_post(
    &self,
    request: tonic::Request<chat_shared::convert::post::proto::CreatePostRequest>,
  ) -> Result<tonic::Response<()>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied().unwrap();

    let CreatePostCommand {
      content,
      channel_id,
    } = request.into_inner().try_into_domain()?;

    let db = database::get().await;

    can_user_access_channel(request_user_id, channel_id).await?;

    entities::post::Entity::insert(
      entities::post::Model {
        id: uuid::Uuid::new_v4(),
        content,
        channel_id: Some(channel_id),
        created_at: chrono::Utc::now(),
      }
      .into_active_model(),
    )
    .exec(db)
    .await
    .map_err(|err| {
      eprintln!("error inserting post: {err}");
      tonic::Status::internal("Error occurred inserting post.")
    })?;

    Ok(tonic::Response::new(()))
  }

  async fn get_posts(
    &self,
    request: tonic::Request<chat_shared::convert::post::proto::GetPostsRequest>,
  ) -> Result<tonic::Response<GetPostsResponse>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied().unwrap();

    let GetPostsRequest {
      channel_id,
      limit,
      starting_before_timestamp,
    } = request.into_inner().try_into_domain()?;

    let db = database::get().await;

    let username: Option<String> = entities::user::Entity::find_by_id(request_user_id)
      .filter(entities::server::Entity::COLUMN.id.eq(request_user_id))
      .select_only()
      .column(entities::user::Column::Username)
      .into_tuple()
      .one(db)
      .await
      .unwrap();

    can_user_access_channel(request_user_id, channel_id).await?;

    let mut query = entities::post::Entity::find().has_related(
      entities::channel::Entity,
      entities::channel::COLUMN.id.eq(channel_id),
    );

    if let Some(starting_before_timestamp) = starting_before_timestamp {
      query = query.filter(
        entities::post::COLUMN
          .created_at
          .lt(starting_before_timestamp),
      );
    };

    let mut posts: Vec<Post> = query
      .limit(limit + 1)
      .order_by_desc(entities::post::Entity::COLUMN.created_at)
      .all(db)
      .await
      .map_err(|err| {
        eprintln!("error fetching posts: {err}");
        tonic::Status::internal("Error occurred fetching posts.")
      })?
      .into_iter()
      .map(|model| Post {
        id: model.id,
        author_name: username.clone().unwrap(),
        content: model.content,
        created_at: model.created_at,
      })
      .collect();

    let has_more = posts.len() > limit as usize;
    let next_timestamp = if has_more {
      posts.pop();
      Some(posts.last().unwrap().created_at)
    } else {
      None
    };

    Ok(tonic::Response::new(
      DomainGetPostsResponse {
        posts,
        next_timestamp,
      }
      .into_proto(),
    ))
  }
}

async fn can_user_access_channel(user_id: Uuid, channel_id: Uuid) -> Result<(), tonic::Status> {
  let db = database::get().await;

  // todo: we don't have the user_user relationship yet, so we just check for a user->server->text_channel relationship

  let server = entities::server::Entity::load()
    .with(entities::user::Entity)
    .with((entities::text_channel::Entity, entities::channel::Entity))
    .filter(
      entities::channel::Entity::COLUMN
        .id
        .eq(channel_id)
        .and(entities::user::COLUMN.id.eq(user_id)),
    )
    .one(db)
    .await
    .unwrap();

  if server.is_none() {
    eprintln!("Requesting user cannot access target server.");
    return Err(tonic::Status::unauthenticated("Unauthenticated"));
  };

  Ok(())
}
