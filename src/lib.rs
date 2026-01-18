use uuid::Uuid;

pub struct Post {
  pub id: Uuid,
  pub content: String,
}

impl Post {
  pub fn new(content: &str) -> Self {
    Self {
      id: Uuid::new_v4(),
      content: content.to_string(),
    }
  }
}

pub mod user;
