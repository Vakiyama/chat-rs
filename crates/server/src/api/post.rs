use chat_shared::{
  convert::{
    IntoProto, TryIntoDomain,
    post::proto::{GetPostsResponse, posts_service_server::PostsService},
  },
  domain::post::{GetPostsRequest, GetPostsResponse as DomainGetPostsResponse, Post},
};
use sea_orm::{
  EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use uuid::Uuid;

use crate::{entities, library::database};

pub struct PostsServer;

#[tonic::async_trait]
impl PostsService for PostsServer {
  async fn get_posts(
    &self,
    request: tonic::Request<chat_shared::convert::post::proto::GetPostsRequest>,
  ) -> Result<tonic::Response<GetPostsResponse>, tonic::Status> {
    let request_user_id = request.extensions().get::<Uuid>().copied().unwrap();

    let GetPostsRequest {
      text_channel_id,
      limit,
      starting_before_timestamp,
    } = request.into_inner().try_into_domain()?;

    let db = database::get().await;

    let channel = entities::channel::Entity::find()
      .filter(
        entities::channel::COLUMN
          .text_channel_id
          .eq(text_channel_id),
      )
      .one(db)
      .await
      .unwrap();

    let channel_id = channel.unwrap().id;

    can_user_access_channel(request_user_id, text_channel_id).await?;

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

    let channel = entities::channel::Entity::load()
      .with((entities::text_channel::Entity, entities::server::Entity))
      .filter(entities::channel::COLUMN.id.eq(channel_id))
      .one(db)
      .await
      .unwrap();

    channel
      .and_then(|channel| channel.text_channel.into_option())
      .and_then(|text_channel| text_channel.server_id)
      .ok_or(tonic::Status::not_found("Server not found"))?;

    let mut posts: Vec<Post> = query
      .limit(limit + 1)
      .find_both_related(entities::user::Entity)
      .order_by_desc(entities::post::Entity::COLUMN.created_at)
      .all(db)
      .await
      .map_err(|err| {
        eprintln!("error fetching posts: {err}");
        tonic::Status::internal("Error occurred fetching posts.")
      })?
      .into_iter()
      .map(|(post, poster)| Post {
        id: post.id,
        author_name: poster.username,
        content: post.content,
        created_at: post.created_at,
      })
      .collect();

    let has_more = posts.len() > limit as usize;
    let next_timestamp = if has_more {
      posts.pop();
      Some(posts.last().unwrap().created_at)
    } else {
      None
    };

    posts.reverse();

    Ok(tonic::Response::new(
      DomainGetPostsResponse {
        posts,
        next_timestamp,
        text_channel_id,
      }
      .into_proto(),
    ))
  }
}

async fn can_user_access_channel(
  user_id: Uuid,
  text_channel_id: Uuid,
) -> Result<(), tonic::Status> {
  let db = database::get().await;

  let count = entities::text_channel::Entity::find()
    .filter(entities::text_channel::COLUMN.id.eq(text_channel_id))
    .inner_join(entities::server::Entity)
    .join_rev(
      JoinType::InnerJoin,
      entities::user_server::Relation::Server.def(),
    )
    // membership predicate on the junction
    .filter(entities::user_server::COLUMN.user_id.eq(user_id))
    .count(db)
    .await
    .map_err(|err| {
      eprintln!("error checking channel access: {err}");
      tonic::Status::internal("Error occurred checking access.")
    })?;

  if count == 0 {
    eprintln!("Requesting user cannot access target channel.");
    return Err(tonic::Status::unauthenticated("Unauthenticated"));
  }
  Ok(())
}
