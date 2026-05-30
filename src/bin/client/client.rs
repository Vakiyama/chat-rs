use crate::client::proto::auth::{RefreshRequest, auth_service_client::AuthServiceClient};
use chat_rs::{SERVER_URL_HTTP, shared::domain::auth::Token};
use http_body_util::BodyExt;
use http_body_util::Full;
use std::{
  pin::Pin,
  sync::{Arc, Mutex},
};
use tokio::sync::OnceCell;
use tonic::transport::Channel;
use tower::Service;

#[derive(Default)]
struct TokenStore {
  access_token: Option<Token>,
  refresh_token: Option<Token>,
}

#[derive(Clone)]
pub struct AuthService {
  inner: tonic::transport::Channel,
  tokens: Arc<Mutex<TokenStore>>,
}

impl AuthService {
  pub fn new(inner: tonic::transport::Channel, tokens: Arc<Mutex<TokenStore>>) -> Self {
    Self { inner, tokens }
  }
}

impl Service<http::Request<tonic::body::Body>> for AuthService {
  type Response = http::Response<tonic::body::Body>;
  type Error = Box<dyn std::error::Error + Send + Sync>;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(
    &mut self,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Result<(), Self::Error>> {
    self.inner.poll_ready(cx).map_err(Into::into)
  }

  fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
    let clone = self.inner.clone();
    let mut inner = std::mem::replace(&mut self.inner, clone);
    let tokens = self.tokens.clone();

    // Capture parts we need to replay before consuming req
    let (parts, body) = req.into_parts();

    Box::pin(async move {
      // Buffer the body bytes so we can replay
      let body_bytes = body.collect().await?.to_bytes();

      let make_req = |tokens: &Arc<Mutex<TokenStore>>| {
        let mut builder = http::Request::builder()
          .method(parts.method.clone())
          .uri(parts.uri.clone())
          .version(parts.version);
        // Clone headers
        *builder.headers_mut().unwrap() = parts.headers.clone();

        let body = tonic::body::Body::new(Full::new(body_bytes.clone()).map_err(|e| match e {}));

        let mut req = builder.body(body).unwrap();
        // Attach current token
        if let Some(token) = &tokens.lock().unwrap().access_token {
          req
            .headers_mut()
            .insert(http::header::AUTHORIZATION, token.parse().unwrap());
        }
        req
      };

      let res = inner.call(make_req(&tokens)).await?;

      if res.status() != http::StatusCode::UNAUTHORIZED {
        return Ok(res);
      }

      let refresh_token = tokens.lock().unwrap().refresh_token.clone();
      let Some(refresh_token) = refresh_token else {
        return Ok(res);
      };

      let token_res = get()
        .await
        .auth
        .refresh(RefreshRequest { refresh_token })
        .await?
        .into_inner();

      {
        let mut store = tokens.lock().unwrap();
        store.access_token = Some(token_res.access_token);
        store.refresh_token = Some(token_res.refresh_token);
      }

      // Retry with fresh token
      inner.call(make_req(&tokens)).await.map_err(Into::into)
    })
  }
}

pub mod proto {
  pub mod auth {
    include!(concat!(env!("OUT_DIR"), "/auth.v1.rs"));
  }
}

#[non_exhaustive]
#[derive(Clone)]
pub struct HTTPClient {
  pub auth: proto::auth::auth_service_client::AuthServiceClient<Channel>,
}

static HTTP_CLIENT: tokio::sync::OnceCell<HTTPClient> = OnceCell::const_new();

pub async fn get() -> HTTPClient {
  HTTP_CLIENT
    .get_or_init(|| async {
      let channel = tonic::transport::Channel::from_static(SERVER_URL_HTTP)
        .connect()
        .await
        .unwrap();

      HTTPClient {
        auth: AuthServiceClient::new(channel),
      }
    })
    .await
    .clone()
}

fn private_channel(
  channel: tonic::transport::Channel,
  tokens: Arc<Mutex<TokenStore>>,
) -> AuthService {
  tower::ServiceBuilder::new()
    .layer_fn(|channel| AuthService::new(channel, tokens.clone()))
    .service(channel)
}
