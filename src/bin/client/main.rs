use iced::widget::container;
use iced::{Element, Subscription};

mod message;
mod websocket;

use message::Message;

use crate::model::{Auth, Screen};
use crate::screens::chat;

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

pub fn update(model: &mut model::Model, message: Message) {
  match message {
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user
        && let Screen::Chat(chat_model) = &mut model.screen
      {
        chat::update(chat_model, msg, user);
      }
    }
  }
}

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let view = match &model.screen {
    model::Screen::Login => todo!(),
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
