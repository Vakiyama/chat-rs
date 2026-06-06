use chat_rs::shared::domain::stream::User;

use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;

use crate::stream;

#[derive(Clone)]
pub enum Stream {
  Connected(stream::Connection),
  Disconnected,
}

pub enum Auth {
  LoggedIn(User),
  NotLoggedIn,
}

pub struct Model {
  pub screen: Screen,
  pub user: Auth,
  pub stream: Stream,
}

pub enum Screen {
  Auth(AuthModel),
  Chat(ChatModel),
}

impl Default for Model {
  fn default() -> Self {
    Model {
      screen: Screen::Auth(AuthModel::new()),
      user: Auth::NotLoggedIn,
      stream: Stream::Disconnected,
    }
  }
}
