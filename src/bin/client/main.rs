use iced::widget::container;
use iced::{Element, Subscription};

mod message;
mod websocket;

use crate::model::{Auth, Screen};
use crate::screens::{auth, chat};

mod model;
mod screens;
mod types;

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
  Subscription::run(websocket::connect)
    .map(|event| event.into())
    .map(Message::Chat)
}

#[derive(Debug, Clone)]
enum Message {
  Chat(chat::Message),
  Auth(auth::Message),
}

fn update(model: &mut model::Model, message: Message) {
  match message {
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user
        && let Screen::Chat(chat_model) = &mut model.screen
      {
        chat::update(chat_model, msg, user);
      }
    }
    Message::Auth(msg) => {
      if let Auth::NotLoggedIn = &model.user
        && let Screen::Auth(auth_model) = &mut model.screen
      {
        auth::update(auth_model, msg)
      }
    }
  }
}

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let view = match &model.screen {
    model::Screen::Auth(model) => auth::view(model).map(Message::Auth),
    model::Screen::Register => todo!(),
    model::Screen::ConfirmCode => todo!(),
    model::Screen::Chat(model) => screens::chat::view(model, "#general").map(Message::Chat),
  };

  container(
    container(view)
      .padding(SPACE_GRID)
      .style(container::rounded_box),
  )
  .padding(SPACE_GRID)
  .into()
}
