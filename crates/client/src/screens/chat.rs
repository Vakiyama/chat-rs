use crate::screens::chat::View::NoneSelected;
use crate::{Element, chat_stream};
use crate::{SPACE_GRID, model::Stream};

use crate::types::async_data::AsyncData;
use chat_shared::domain::post::Post;
use chat_shared::domain::server::Server;
use chat_shared::domain::stream::{ClientText, ServerText, User};
use chrono::Utc;
use iced::widget::{Column, column, space, text, text_input};
use iced::widget::{container, row};
use iced::{Pixels, Task};
use uuid::Uuid;

// --------------------------------- MODEL ---------------------------------

#[derive(Default)]
pub struct Model {
  // posts: AsyncData<Vec<ServerText>, tonic::Status>,
  servers: AsyncData<Vec<Server>, tonic::Status>,
  view: View,
  input: String,
}

#[derive(Default)]
enum View {
  #[default]
  NoneSelected,
  TextChannel(TextChannel),
}

struct TextChannel {
  id: Uuid,
  server_id: Uuid,
  posts: AsyncData<Vec<RenderedPost>, tonic::Status>,
}

enum RenderedPost {
  Sending {
    id: uuid::Uuid,
    created_at: chrono::DateTime<Utc>,
    content: String,
  },
  Sent(Post),
}

impl TextChannel {
  pub fn send(
    &mut self,
    user: &User,
    mut stream: Stream<chat_stream::ChatConnection>,
    content: String,
  ) -> Result<(), Error> {
    match &mut stream {
      Stream::Connected(connection) => {
        let input = content;

        //        self.posts.as_mut().map(|posts| {
        //          posts.push(ServerText::Post(RenderedPost::Sending {
        //            id: uuid::Uuid::new_v4(),
        //            content,
        //            created_at: (),
        //          }))
        //        });
        //
        //        self.input = "".to_string();
        //
        //        connection.send(ClientText::ChatMessage {
        //          from: user.clone(),
        //          text: input,
        //        });
        //
        Ok(())
      }
      Stream::Disconnected => Err(Error::NoConnection),
    }
  }

  pub fn receive(&mut self, text: String, from: User) {
    todo!()
  }
}

#[derive(Debug, Clone)]
pub enum Message {
  UserChangedChatInput(String),
  UserSubmittedChatInput,
  Stream(ServerText),
}

pub enum Error {
  NoConnection,
}

// --------------------------------- VIEW ---------------------------------

pub fn view<'a>(model: &'_ Model, chat_title: &'a str) -> Element<'a, Message> {
  todo!()
  // let posts = model
  //   .posts
  //   .as_ref()
  //   .get_or(&Vec::new()) // temp: replace get_or with showing a proper loading view...
  //   .iter()
  //   .map(|post| {
  //     let element: Element<Message> = {
  //       if let ServerText::ChatMessage {
  //         from,
  //         text: incoming_text,
  //       } = post
  //       {
  //         row![text(from.name.clone()), text(incoming_text.clone())]
  //           .spacing(Pixels(SPACE_GRID.into()))
  //           .into()
  //       } else {
  //         space().into()
  //       }
  //     };

  //     element
  //   })
  //   .collect::<Vec<_>>();

  // let children: Element<'_, Message> = column![
  //   container(Column::with_children(posts))
  //     .padding([SPACE_GRID, 0])
  //     .height(iced::Fill),
  //   text_input("Send message", &model.input)
  //     .on_input(Message::UserChangedChatInput)
  //     .on_submit(Message::UserSubmittedChatInput)
  //     .padding(SPACE_GRID)
  // ]
  // .into();

  // column![container(chat_title), children]
  //   .spacing({ SPACE_GRID } as u32)
  //   .into()
}
// --------------------------------- UPDATE ---------------------------------

pub fn update(
  model: &mut Model,
  message: Message,
  user: &User,
  stream: Stream<chat_stream::ChatConnection>,
) -> Task<Message> {
  match message {
    Message::UserChangedChatInput(new) => {
      model.input = new;
      Task::none()
    }
    Message::UserSubmittedChatInput => {
      todo!()
      // if let Err(Error::NoConnection) = model.send(user, stream) {
      //   println!("Not connected...")
      // }
      // Task::none()
    }
    Message::Stream(server_message) => match server_message {
      ServerText::JoinedRoom { from } => {
        println!("User joined room: {from:?}");
        Task::none()
      }
      ServerText::LeftRoom { from } => {
        println!("User left room: {from:?}");

        Task::none()
      } // ServerText::ChatMessage { from, text } => {
      //   // model.receive(text, from);

      //   // Task::none()
      // }
      ServerText::Post(post) => todo!(),
    },
  }
}
