use iced::Element;
use iced::Subscription;
use iced::keyboard;
use iced::keyboard::Event;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::container;
use iced::widget::{Column, column, row, text, text_input};

mod message;
mod websocket;

use message::Message;

use crate::model::WebSocket;

mod model;

const SPACE_GRID: u16 = 8;

fn main() -> iced::Result {
  iced::application(new, update, view)
    .subscription(subscription)
    .run()
}

fn new() -> model::Model {
  model::Model::default()
}

fn subscription(_model: &model::Model) -> Subscription<Message> {
  Subscription::batch([
    keyboard::listen().map(Message::Keyboard),
    Subscription::run(websocket::connect).map(|event| event.into()),
  ])
}

fn update(model: &mut model::Model, message: Message) {
  match message {
    Message::ContentChanged(new) => {
      model.input = new;
    }
    Message::Keyboard(event) => {
      if let Event::KeyPressed {
        key: Key::Named(Named::Enter),
        ..
      } = event
        && let Err(model::Error::NoConnection) = model.send()
      {
        println!("Not connected...")
      }
    }
    Message::Disconnected => {
      model.websocket = WebSocket::Disconnected;
    }
    Message::Connected(connection) => {
      model.websocket = WebSocket::Connected(connection);
    }
    Message::Websocket(server_message) => match server_message {
      chat_rs::ServerMessage::JoinedRoom { from } => {
        println!("User joined room: {from:?}")
      }
      chat_rs::ServerMessage::LeftRoom { from } => println!("User left room: {from:?}"),
      chat_rs::ServerMessage::Chat { from, text } => model.receive(&text, &from.name),
      chat_rs::ServerMessage::Ping => {
        println!("Server ping received.")
      }
    },
  }
}

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  container(
    container(view_chat(model, "#general"))
      .padding(SPACE_GRID)
      .style(container::rounded_box),
  )
  .padding(SPACE_GRID)
  .into()
}

fn view_chat<'a>(model: &'_ model::Model, chat_title: &'a str) -> Element<'a, Message> {
  let posts = model
    .posts
    .iter()
    .map(|post| {
      let element: Element<Message> =
        row![text(post.author_name.clone()), text(post.content.clone())]
          .spacing(SPACE_GRID as u32)
          .into();
      element
    })
    .collect::<Vec<_>>();

  let children: Element<'_, Message> = column![
    container(Column::with_children(posts))
      .padding([SPACE_GRID, 0])
      .height(iced::Fill),
    text_input("Send message", &model.input)
      .on_input(Message::ContentChanged)
      .padding(SPACE_GRID)
  ]
  .into();

  column![container(chat_title), children]
    .spacing({ SPACE_GRID } as u32)
    .into()
}
