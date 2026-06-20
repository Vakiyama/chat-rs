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
  Ping {
    timestamp: u64,
  },
}

#[derive(Clone, Debug)]
pub enum ServerText {
  JoinedRoom {
    from: User,
  },
  LeftRoom {
    from: User,
  },
  Post(Post),
  Pong {
    timestamp: u64,
    server_received_at: u64,
  },
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
  Speaking {
    speaking: bool,
    voice_channel_id: Uuid,
  },
  SubscribeServer {
    server_id: Uuid,
  },
  SetMuted {
    muted: bool,
    voice_channel_id: Uuid,
  },
  SetDeafened {
    deafened: bool,
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
  PresenceSnapshot {
    voice_channel_id: Uuid,
    server_id: Uuid,
    peers: Vec<DisplayVoiceUser>,
  },
}

#[derive(Clone, Debug)]
pub struct Speaking {
  pub speaking: bool,
}

#[derive(Clone, Debug)]
pub struct DisplayVoiceUser {
  pub user: User,
  pub muted: bool,
  pub deafened: bool,
  pub speaking: bool,
}
