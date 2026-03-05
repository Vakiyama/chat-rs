use std::sync::Arc;

use iced::widget::container;
use iced::{Element, Subscription};
use resend_rs::Resend;

mod message;
mod websocket;

use crate::model::{Auth, Screen};
use crate::screens::{auth, chat};

mod library;
mod model;
mod screens;
mod types;

const SPACE_GRID: u16 = 8;

fn main() -> iced::Result {
  let _env = dotenvy::dotenv().unwrap();
  let resend = Arc::new(Resend::default());

  iced::application(new, make_update(resend), view)
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

fn make_update(resend: Arc<Resend>) -> impl Fn(&mut model::Model, Message) -> iced::Task<Message> {
  move |model: &mut model::Model, message: Message| match message {
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user
        && let Screen::Chat(chat_model) = &mut model.screen
      {
        chat::update(chat_model, msg, user);
        iced::Task::none()
      } else {
        iced::Task::none()
      }
    }
    Message::Auth(msg) => {
      if let Auth::NotLoggedIn = &model.user
        && let Screen::Auth(auth_model) = &mut model.screen
      {
        auth::update(auth_model, msg, resend.clone()).map(Message::Auth)
      } else {
        iced::Task::none()
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
