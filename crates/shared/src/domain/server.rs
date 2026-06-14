use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Channel {
  pub id: Uuid,
  pub name: String,
  pub r#type: ChannelType,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChannelType {
  Unspecified,
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
