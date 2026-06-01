use chat_rs::shared::domain::stream::User;

use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;

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
  Auth(AuthModel),
  Register,
  ConfirmCode,
  Chat(ChatModel),
}

impl Default for Model {
  fn default() -> Self {
    Model {
      screen: Screen::Auth(AuthModel::new()),
      user: Auth::NotLoggedIn,
    }
  }
}
