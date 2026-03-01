use crate::screens::chat::Model as ChatModel;
use chat_rs::schema::user::Model as User;
use uuid::Uuid;

use crate::websocket;

pub enum WebSocket {
  Connected(websocket::Connection),
  Disconnected,
}
pub enum Auth {
  LoggedIn(User),
  NotLoggedIn,
}

pub struct Model {
  pub screen: Screen,
  pub user: Auth,
}

pub enum Screen {
  Login,
  Register,
  ConfirmCode,
  Chat(ChatModel),
}

impl Default for Model {
  // fn default() -> Self {
  //   Model {
  //     screen: Screen::Login,
  //     user: Auth::NotLoggedIn,
  //   }
  // }
  //
  // debug version vvvv
  fn default() -> Self {
    Model {
      screen: Screen::Chat(Default::default()),
      user: Auth::LoggedIn(User {
        id: Uuid::new_v4(),
        name: "RootPoison".into(),
      }),
    }
  }
}
