use crate::Element;
use crate::{SPACE_GRID, model::Stream};

use crate::stream::Connection;
use crate::types::async_data::AsyncData;
use chat_rs::shared::domain::stream::{Client, Server, User};
use iced::widget::{Column, column, space, text, text_input};
use iced::widget::{container, row};
use iced::{Pixels, Task};

// --------------------------------- MODEL ---------------------------------

pub struct Model {
  posts: AsyncData<Vec<Server>, tonic::Status>,
  input: String,
  stream: Stream,
}

impl Default for Model {
  fn default() -> Self {
    Self {
      posts: AsyncData::Done(Ok(vec![])),
      input: String::new(),
      stream: Stream::Disconnected,
    }
  }
}

impl Model {
  pub fn send(&mut self, user: &User) -> Result<(), Error> {
    match &mut self.stream {
      Stream::Connected(connection) => {
        let input = self.input.clone();

        self.posts.as_mut().map(|posts| {
          posts.push(Server::ChatMessage {
            from: user.clone(),
            text: input.clone(),
          })
        });

        self.input = "".to_string();

        connection.send(Client::ChatMessage {
          from: user.clone(),
          text: input,
        });

        Ok(())
      }
      Stream::Disconnected => Err(Error::NoConnection),
    }
  }

  pub fn receive(&mut self, text: String, from: User) {
    self
      .posts
      .as_mut()
      .map(|posts| posts.push(Server::ChatMessage { from, text }));
  }
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedChatInput(String),
  UserSubmittedChatInput,
  Disconnected,
  Connected(Connection),
  Stream(Server),
}

pub enum Error {
  NoConnection,
}

// --------------------------------- VIEW ---------------------------------

pub fn view<'a>(model: &'_ Model, chat_title: &'a str) -> Element<'a, Message> {
  let posts = model
    .posts
    .as_ref()
    .get_or(&Vec::new()) // temp: replace get_or with showing a proper loading view...
    .iter()
    .map(|post| {
      let element: Element<Message> = {
        if let Server::ChatMessage {
          from,
          text: incoming_text,
        } = post
        {
          row![text(from.name.clone()), text(incoming_text.clone())]
            .spacing(Pixels(SPACE_GRID.into()))
            .into()
        } else {
          space().into()
        }
      };

      element
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
  println!("{message:#?}");
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
      model.stream = Stream::Disconnected;
      Task::none()
    }
    Message::Connected(connection) => {
      model.stream = Stream::Connected(connection);
      Task::none()
    }
    Message::Stream(server_message) => match server_message {
      Server::JoinedRoom { from } => {
        println!("User joined room: {from:?}");
        Task::none()
      }
      Server::LeftRoom { from } => {
        println!("User left room: {from:?}");

        Task::none()
      }
      Server::ChatMessage { from, text } => {
        model.receive(text, from);

        Task::none()
      }
    },
  }
}
