use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Channel {
  pub id: Uuid,
  pub name: String,
  pub r#type: ChannelType,
  pub muted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChannelType {
  Text,
  Voice,
}

#[derive(Clone, Debug)]
pub struct Server {
  pub id: Uuid,
  pub name: String,
  pub channels: Vec<Channel>,
}

#[derive(Clone, Debug)]
pub struct ServersResponse {
  pub servers: Vec<Server>,
}

#[derive(Clone, Debug)]
pub struct SetChannelMuteRequest {
  pub text_channel_id: Uuid,
  pub muted: bool,
}
