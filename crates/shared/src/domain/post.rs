use chrono::Utc;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Post {
  pub id: Uuid,
  pub author_name: String,
  pub content: String,
  pub created_at: chrono::DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct GetPostsResponse {
  pub posts: Vec<Post>,
  pub next_timestamp: Option<chrono::DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct GetPostsRequest {
  pub channel_id: Uuid,
  pub limit: u64,
  pub starting_before_timestamp: Option<chrono::DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct CreatePostCommand {
  pub content: String,
  pub channel_id: Uuid,
}
