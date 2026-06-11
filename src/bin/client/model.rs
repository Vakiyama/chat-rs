use std::sync::Arc;

use chat_rs::shared::domain::stream::User;
use webrtc::peer_connection::RTCPeerConnection;

use crate::screens::auth::Model as AuthModel;
use crate::screens::chat::Model as ChatModel;

use crate::{chat_stream, webrtc_stream};

#[derive(Clone)]
pub enum Stream<T> {
  Connected(T),
  Disconnected,
}

pub enum Auth {
  LoggedIn(User),
  NotLoggedIn,
}

pub struct Model {
  pub screen: Screen,
  pub user: Auth,
  pub chat_stream: Stream<chat_stream::ChatConnection>,
  pub webrtc_stream: Stream<webrtc_stream::WebRTCConnection>,
  pub webrtc_client: Option<Arc<RTCPeerConnection>>,
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
      chat_stream: Stream::Disconnected,
      webrtc_stream: Stream::Disconnected,
      webrtc_client: None,
    }
  }
}
