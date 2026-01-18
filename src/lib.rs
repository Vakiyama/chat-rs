pub struct Post {
  pub content: String,
}

impl Post {
  pub fn new(content: &str) -> Self {
    Self {
      content: content.to_string(),
    }
  }
}
