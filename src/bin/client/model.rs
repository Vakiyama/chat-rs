use chat_rs::ClientMessage;
use chat_rs::schema::post::Model as Post;
use chat_rs::schema::user::Model as User;
use uuid::Uuid;

use crate::websocket;

pub enum WebSocket {
  Connected(websocket::Connection),
  Disconnected,
}

pub struct Model {
  pub posts: Vec<Post>,
  pub input: String,
  pub websocket: WebSocket,
  pub user: User,
}

pub enum Error {
  NoConnection,
}

impl Default for Model {
  fn default() -> Self {
    Model {
      posts: vec![
        Post::new("Post 1", "RootPoison"),
        Post::new("Post 2", "Cecilian"),
      ],
      input: String::new(),
      websocket: WebSocket::Disconnected,
      user: User {
        id: Uuid::new_v4(),
        name: "placeholder client".to_string(),
      },
    }
  }
}

impl Model {
  pub fn send(&mut self) -> Result<(), Error> {
    match &mut self.websocket {
      WebSocket::Connected(connection) => {
        let name = self.user.name.clone();
        let input = self.input.clone();

        self.posts.push(Post::new(&input, &name));
        self.input = "".to_string();

        connection.send(ClientMessage::Chat {
          from: self.user.clone(),
          text: input,
        });

        Ok(())
      }
      WebSocket::Disconnected => Err(Error::NoConnection),
    }
  }

  pub fn receive(&mut self, message: &str, username: &str) {
    self.posts.push(Post::new(message, username));
  }
}
