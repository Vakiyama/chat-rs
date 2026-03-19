use bytes::Bytes;

#[derive(Debug, thiserror::Error)]
pub enum RequestError<E>
where
  E: std::fmt::Debug + std::fmt::Display,
{
  #[error("network error: {0}")]
  Network(#[from] reqwest::Error),

  #[error("server error: {0}")]
  Server(E),

  #[error("decode error: {0}")]
  Decode(String),
}

pub trait Decode: Sized {
  fn decode(bytes: Bytes) -> Result<Self, String>;
}

impl<T> Decode for T
where
  T: for<'de> serde::Deserialize<'de>,
{
  fn decode(bytes: Bytes) -> Result<Self, String> {
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
  }
}
