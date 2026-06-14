use chat_shared::{
  convert::{
    IntoProto, TryIntoDomain,
    post::proto::{PostsResponse, posts_service_server::PostsService},
  },
  domain::post::{Post, PostsRequest, PostsResponse as DomainPostsResponse},
};
use sea_orm::{EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::{entities, library::database};

pub struct PostsServer;

#[tonic::async_trait]
impl PostsService for PostsServer {
  async fn posts(
    &self,
    request: tonic::Request<chat_shared::convert::post::proto::PostsRequest>,
  ) -> Result<tonic::Response<PostsResponse>, tonic::Status> {
    let _request_user_id = request
      .extensions()
      .get::<Uuid>()
      .ok_or_else(|| tonic::Status::unauthenticated("Unauthenticated"))?;

    let PostsRequest {
      text_channel_id,
      limit,
      starting_before_timestamp,
    } = request.into_inner().try_into_domain()?;

    let db = database::get().await;

    let mut query = entities::post::Entity::find().has_related(
      entities::channel::Entity,
      entities::channel::COLUMN
        .text_channel_id
        .eq(text_channel_id),
    );

    if let Some(starting_before_timestamp) = starting_before_timestamp {
      query = query.filter(
        entities::post::COLUMN
          .created_at
          .lt(starting_before_timestamp),
      );
    };

    let posts: Vec<Post> = query
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
        author_name: model.author_name,
        content: model.content,
        created_at: model.created_at,
      })
      .collect();

    let has_more = posts.len() > limit as usize;
    let next_timestamp = if has_more {
      Some(posts.last().unwrap().created_at)
    } else {
      None
    };

    Ok(tonic::Response::new(
      DomainPostsResponse {
        posts,
        next_timestamp,
      }
      .into_proto(),
    ))
  }
}
