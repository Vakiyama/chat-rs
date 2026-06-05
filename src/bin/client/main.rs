use chat_rs::shared::convert::TryIntoDomain;
use chat_rs::shared::convert::user::proto::MeResponse;
use chat_rs::shared::domain::stream::User;
use chat_rs::shared::domain::user::MeReturn;
use iced::widget::container;
use iced::{Element, Subscription, Task};

mod stream;

use crate::model::{Auth, Screen};
use crate::screens::{auth, chat};

mod client;
mod model;
mod screens;
mod types;

const SPACE_GRID: u16 = 8;

fn main() -> iced::Result {
  iced::application(new, update, view)
    .subscription(subscription)
    .run()
}

fn new() -> (model::Model, Task<Message>) {
  (
    model::Model::default(),
    Task::perform(
      async {
        client::get()
          .await
          .user
          .me(())
          .await
          .unwrap()
          .into_inner()
          .try_into_domain()
          .ok()
      },
      Message::Loaded,
    ),
  )
}

fn subscription(_model: &model::Model) -> Subscription<Message> {
  Subscription::run(stream::connect)
    .map(|event| event.into())
    .map(Message::Chat)
}

#[derive(Debug, Clone)]
enum Message {
  Chat(chat::Message),
  Auth(auth::Message),
  Loaded(Option<MeReturn>),
  None,
}

fn update(model: &mut model::Model, message: Message) -> iced::Task<Message> {
  match message {
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user
        && let Screen::Chat(chat_model) = &mut model.screen
      {
        chat::update(chat_model, msg, user).map(Message::Chat)
      } else {
        iced::Task::none()
      }
    }
    Message::Auth(msg) => {
      if let Auth::NotLoggedIn = &model.user
        && let Screen::Auth(auth_model) = &mut model.screen
      {
        match msg {
          auth::Message::ApiVerifiedCode(Ok(response)) => {
            model.screen = Screen::Chat(Default::default());
            model.user = Auth::LoggedIn(User {
              id: response.user_id,
              name: response.username.clone(),
            });

            Task::future(async {
              client::get()
                .await
                .insert_tokens(response.refresh_token, response.access_token)
                .await;
              // entry.delete_credential()?;
              Message::None
            })
          }
          msg => auth::update(auth_model, msg).map(Message::Auth),
        }
      } else if let Auth::LoggedIn(_) = model.user {
        model.screen = Screen::Chat(Default::default());
        iced::Task::none()
      } else {
        iced::Task::none()
      }
    }
    Message::None => iced::Task::none(),
    Message::Loaded(me_return) => match me_return {
      Some(response) => {
        model.screen = Screen::Chat(Default::default());
        model.user = Auth::LoggedIn(User {
          id: response.user_id,
          name: response.username.clone(),
        });

        Task::none()
      }
      None => Task::none(),
    },
  }
}

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let view = match &model.screen {
    model::Screen::Auth(model) => auth::view(model).map(Message::Auth),
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
