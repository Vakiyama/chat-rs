use uuid::Uuid;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::domain::post::Post;

#[derive(Clone, Debug)]
pub struct CreatePostCommand {
  pub content: String,
  pub channel_id: Uuid,
}
#[derive(Clone, Debug)]
pub struct User {
  pub id: Uuid,
  pub name: String,
}

pub enum ClientText {
  CreatePostRequest {
    id: Uuid,
    content: String,
    text_channel_id: Uuid,
  },
}

#[derive(Clone, Debug)]
pub enum ServerText {
  JoinedRoom { from: User },
  LeftRoom { from: User },
  Post(Post),
}

pub enum ClientVoice {
  Offer {
    description: RTCSessionDescription,
    voice_channel_id: Uuid,
  },
  Answer {
    description: RTCSessionDescription,
    voice_channel_id: Uuid,
  },
  LeaveRoom {
    voice_channel_id: Uuid,
  },
}

#[derive(Clone, Debug)]
pub enum ServerVoice {
  Offer {
    description: RTCSessionDescription,
    voice_channel_id: Uuid,
  },
  Answer {
    description: RTCSessionDescription,
    voice_channel_id: Uuid,
  },
}
