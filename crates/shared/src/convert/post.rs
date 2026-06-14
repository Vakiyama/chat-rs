pub mod proto {
  include!(concat!(env!("OUT_DIR"), "/posts.v1.rs"));
}

use crate::convert::IntoProto;
use crate::convert::TryFromProto;
use crate::domain::post::*;
use chrono::DateTime;
use prost_types::Timestamp;
use proto::Post as PostProto;
use proto::PostsRequest as PostsRequestProto;
use proto::PostsResponse as PostsResponseProto;
use tonic::Status;
use uuid::Uuid;

impl IntoProto<PostsResponseProto> for PostsResponse {
  fn into_proto(self) -> PostsResponseProto {
    let next_timestamp = self.next_timestamp.map(|next_timestamp| {
      let seconds = next_timestamp.timestamp();
      let nanos = next_timestamp.timestamp_subsec_nanos() as i32;
      Timestamp { seconds, nanos }
    });

    PostsResponseProto {
      posts: self.posts.into_iter().map(|p| p.into_proto()).collect(),
      next_timestamp,
    }
  }
}

impl IntoProto<PostProto> for Post {
  fn into_proto(self) -> PostProto {
    let seconds = self.created_at.timestamp();
    let nanos = self.created_at.timestamp_subsec_nanos() as i32;
    let created_at = Timestamp { seconds, nanos };

    PostProto {
      id: self.id.to_string(),
      author_name: self.author_name,
      content: self.content,
      created_at: Some(created_at),
    }
  }
}

impl TryFromProto<PostsRequestProto> for PostsRequest {
  type Error = Status;

  fn try_from_proto(proto: PostsRequestProto) -> Result<Self, Self::Error> {
    let text_channel_id = Uuid::parse_str(&proto.text_channel_id)
      .map_err(|_| Status::invalid_argument("invalid text_channel_id"))?;

    let starting_before_timestamp = proto.starting_before_timestamp.and_then(|timestamp| {
      DateTime::from_timestamp(timestamp.seconds, timestamp.nanos.try_into().unwrap_or(0))
    });

    Ok(PostsRequest {
      text_channel_id,
      limit: proto.limit,
      starting_before_timestamp,
    })
  }
}

impl TryFromProto<PostsResponseProto> for PostsResponse {
  type Error = Status;

  fn try_from_proto(proto: PostsResponseProto) -> Result<Self, Self::Error> {
    let mut posts = Vec::new();
    for post in proto.posts {
      posts.push(Post::try_from_proto(post)?);
    }

    let next_timestamp = proto.next_timestamp.and_then(|timestamp| {
      DateTime::from_timestamp(timestamp.seconds, timestamp.nanos.try_into().unwrap_or(0))
    });

    Ok(PostsResponse {
      posts,
      next_timestamp,
    })
  }
}

impl TryFromProto<PostProto> for Post {
  type Error = Status;

  fn try_from_proto(proto: PostProto) -> Result<Self, Self::Error> {
    let id = Uuid::parse_str(&proto.id).map_err(|_| Status::invalid_argument("invalid post id"))?;

    let created_at = proto
      .created_at
      .and_then(|timestamp| {
        DateTime::from_timestamp(timestamp.seconds, timestamp.nanos.try_into().unwrap_or(0))
      })
      .ok_or(tonic::Status::invalid_argument(
        "created_at is invalid or missing.",
      ))?;

    Ok(Post {
      id,
      author_name: proto.author_name,
      content: proto.content,
      created_at,
    })
  }
}
