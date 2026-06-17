use chat_shared::convert::TryIntoDomain;
use chat_shared::domain::stream::{ServerVoice, User};
use chat_shared::domain::user::MeReturn;
use iced::Theme::CatppuccinFrappe;
use iced::widget::{Text, container, text};
use iced::{Element, Font, Subscription, Task};
use material_icons::{Icon, icon_to_char};

pub mod audio_processing;
mod chat_stream;
pub mod config;
pub mod webrtc_stream;

use crate::audio_processing::call_handler::spawn_voice;
use crate::chat_stream::ChatConnection;
use crate::model::{Auth, Screen, Stream};
use crate::screens::{auth, chat};
use crate::webrtc_stream::WebRTCConnection;

mod client;
mod model;
mod screens;
mod types;

const SPACE_GRID: u16 = 8;

fn main() -> iced::Result {
  iced::application(new, update, view)
    .theme(CatppuccinFrappe)
    .font(material_icons::FONT)
    .subscription(subscription)
    .run()
}

// for icon loading

pub const MATERIAL: Font = Font::with_name("Material Icons");

pub fn icon<'a>(i: Icon) -> Text<'a> {
  text(icon_to_char(i).to_string()).font(MATERIAL)
}

fn new() -> (model::Model, Task<Message>) {
  (
    model::Model::default(),
    Task::perform(
      async {
        let mut client = client::get().await;

        if client.has_tokens().await {
          client
            .user
            .me(())
            .await
            .unwrap()
            .into_inner()
            .try_into_domain()
            .ok()
        } else {
          None
        }
      },
      Message::Loaded,
    ),
  )
}

fn subscription(model: &model::Model) -> Subscription<Message> {
  match model.user {
    Auth::LoggedIn(_) => Subscription::batch([
      Subscription::run(chat_stream::connect).map(|event| event.into()),
      Subscription::run(webrtc_stream::connect).map(|event| event.into()),
      iced::event::listen_with(|event, status, _window| match (event, status) {
        (
          iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(_),
            text: Some(text),
            modifiers,
            ..
          }),
          iced::event::Status::Ignored,
        ) if !modifiers.command() && !modifiers.control() && !modifiers.alt() => {
          Some(Message::Chat(chat::Message::TypeAhead(text.into())))
        }
        _ => None,
      }),
    ]),
    Auth::NotLoggedIn => Subscription::none(),
  }
}

#[derive(Clone)]
pub enum Message {
  Chat(chat::Message),
  Auth(auth::Message),
  Loaded(Option<MeReturn>),
  ChatStreamConnected(ChatConnection),
  ChatStreamDisconnected,
  WebRTC(Box<ServerVoice>),
  WebRTCSignalStreamConnected(WebRTCConnection),
  WebRTCSignalStreamDisconnected,
  JoinVoice,
  LeaveVoice,
  None,
  LoggedIn(User),
}

fn update(model: &mut model::Model, message: Message) -> iced::Task<Message> {
  match message {
    Message::Chat(msg) => {
      if let Auth::LoggedIn(user) = &model.user
        && let Screen::Chat(chat_model) = &mut model.screen
      {
        chat::update(chat_model, msg, user, model.chat_stream.clone()).map(Message::Chat)
      } else {
        iced::Task::none()
      }
    }
    Message::LoggedIn(user) => {
      model.screen = Screen::Chat(Default::default());
      model.user = Auth::LoggedIn(user);
      iced::Task::done(Message::Chat(chat::Message::Init))
    }
    Message::Auth(msg) => {
      if let Auth::NotLoggedIn = &model.user
        && let Screen::Auth(auth_model) = &mut model.screen
      {
        match msg {
          auth::Message::ApiVerifiedCode(Ok(response)) => {
            let user = User {
              id: response.user_id,
              name: response.username.clone(),
            };

            Task::future(async move {
              client::get()
                .await
                .insert_tokens(response.refresh_token, response.access_token)
                .await;
              Message::LoggedIn(user)
            })
          }
          msg => auth::update(auth_model, msg).map(Message::Auth),
        }
      } else if let Auth::LoggedIn(_) = model.user {
        model.screen = Screen::Chat(Default::default());
        iced::Task::done(Message::Chat(chat::Message::Init))
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

        iced::Task::done(Message::Chat(chat::Message::Init))
      }
      None => Task::none(),
    },
    Message::ChatStreamDisconnected => {
      model.chat_stream = Stream::Disconnected;
      Task::none()
    }
    Message::ChatStreamConnected(connection) => {
      model.chat_stream = Stream::Connected(connection);

      Task::none()
    }
    Message::JoinVoice => {
      if let Some(v) = &model.voice {
        v.join();
      }
      Task::none()
    }
    Message::LeaveVoice => {
      if let Some(v) = &model.voice {
        v.leave();
      }
      Task::none()
    }
    Message::WebRTC(msg) => {
      if let Some(v) = &model.voice {
        v.signal(*msg);
      }
      Task::none()
    }
    Message::WebRTCSignalStreamConnected(conn) => {
      model.voice = Some(spawn_voice(conn));

      Task::done(Message::JoinVoice)
    }
    Message::WebRTCSignalStreamDisconnected => {
      model.voice = None; // drops the handle → actor loop ends
      model.webrtc_stream = Stream::Disconnected;
      Task::none()
    }
  }
}

fn view(model: &'_ model::Model) -> Element<'_, Message> {
  let view = match &model.screen {
    model::Screen::Auth(model) => auth::view(model).map(Message::Auth),
    model::Screen::Chat(model) => screens::chat::view(model).map(Message::Chat),
  };

  container(
    container(view)
      .padding(SPACE_GRID)
      .style(container::rounded_box),
  )
  .padding(SPACE_GRID)
  .into()
}
