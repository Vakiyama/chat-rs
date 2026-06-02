use crate::Element;
use crate::{SPACE_GRID, model::WebSocket};

use crate::types::async_data::AsyncData;
use crate::websocket::Connection;
use chat_rs::shared::domain::stream::{Client, Server, User};
use iced::widget::container;
use iced::widget::{Column, column, row, text, text_input};
use iced::{Pixels, Task};

// --------------------------------- MODEL ---------------------------------

pub struct Model {
  posts: AsyncData<Vec<()>, ()>,
  input: String,
  websocket: WebSocket,
}

impl Default for Model {
  fn default() -> Self {
    Self {
      posts: AsyncData::Done(Ok(vec![])),
      input: String::new(),
      websocket: WebSocket::Disconnected,
    }
  }
}

impl Model {
  pub fn send(&mut self, user: &User) -> Result<(), Error> {
    match &mut self.websocket {
      WebSocket::Connected(connection) => {
        todo!()
        //  let name = user.name.clone();
        //  let input = self.input.clone();

        //  self
        //    .posts
        //    .as_mut()
        //    .map(|posts| posts.push(Post::new(&input, &name)));

        //  self.input = "".to_string();

        //  connection.send(Client::ChatMessage {
        //    from: user.clone(),
        //    text: input,
        //  });

        //  Ok(())
      }
      WebSocket::Disconnected => Err(Error::NoConnection),
    }
  }

  pub fn receive(&mut self, message: &str, username: &str) {
    self.posts.as_mut().map(|posts| {
      todo!()
      // posts.push(Post::new(message, username))
    });
  }
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedChatInput(String),
  UserSubmittedChatInput,
  Disconnected,
  Connected(Connection),
  Websocket(Server),
}

pub enum Error {
  NoConnection,
}

// --------------------------------- VIEW ---------------------------------

pub fn view<'a>(model: &'_ Model, chat_title: &'a str) -> Element<'a, Message> {
  let posts = model
    .posts
    .as_ref()
    .get_or(&vec![]) // temp: replace get_or with showing a proper loading view...
    .iter()
    .map(|post| {
      let element: Element<Message> = {
        todo!()
        //   row![text(post.author_name.clone()), text(post.content.clone())]
        //     .spacing(Pixels(SPACE_GRID.into()))
        //     .into();
        // element
      };
    })
    .collect::<Vec<_>>();

  let children: Element<'_, Message> = column![
    container(Column::with_children(posts))
      .padding([SPACE_GRID, 0])
      .height(iced::Fill),
    text_input("Send message", &model.input)
      .on_input(Message::UserChangedChatInput)
      .on_submit(Message::UserSubmittedChatInput)
      .padding(SPACE_GRID)
  ]
  .into();

  column![container(chat_title), children]
    .spacing({ SPACE_GRID } as u32)
    .into()
}
// --------------------------------- UPDATE ---------------------------------

pub fn update(model: &mut Model, message: Message, user: &User) -> Task<Message> {
  match message {
    Message::UserChangedChatInput(new) => {
      model.input = new;
      Task::none()
    }
    Message::UserSubmittedChatInput => {
      if let Err(Error::NoConnection) = model.send(user) {
        println!("Not connected...")
      }
      Task::none()
    }
    Message::Disconnected => {
      model.websocket = WebSocket::Disconnected;
      Task::none()
    }
    Message::Connected(connection) => {
      model.websocket = WebSocket::Connected(connection);
      Task::none()
    }
    Message::Websocket(server_message) => match server_message {
      Server::JoinedRoom { from } => {
        println!("User joined room: {from:?}");
        Task::none()
      }
      Server::LeftRoom { from } => {
        println!("User left room: {from:?}");

        Task::none()
      }
      Server::ChatMessage { from, text } => {
        model.receive(&text, &from.name);

        Task::none()
      }
    },
  }
}
