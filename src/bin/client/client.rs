use chat_rs::config::CONFIG;
use chat_rs::shared::convert::IntoProto;
use chat_rs::shared::convert::auth::proto::auth_service_client::AuthServiceClient;
use chat_rs::shared::convert::stream::proto::stream_service_client::StreamServiceClient;
use chat_rs::shared::domain::auth::{RefreshCommand, VerifyReturn};
use chat_rs::shared::domain::auth::Token;
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
pub struct TokenStore {
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
        .refresh(RefreshCommand { refresh_token }.into_proto())
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

#[non_exhaustive]
#[derive(Clone)]
pub struct GrpcClient {
  pub auth: AuthServiceClient<Channel>,
  pub stream: StreamServiceClient<AuthService>,
  tokens: Arc<Mutex<TokenStore>>,
}

impl GrpcClient {
  pub fn insert_tokens(&self, response: VerifyReturn) {
    let mut tokens = self.tokens.lock().unwrap();
    tokens.access_token = Some(response.access_token);
    tokens.refresh_token = Some(response.refresh_token);
  }
}

static GRPC_CLIENT: tokio::sync::OnceCell<GrpcClient> = OnceCell::const_new();

pub async fn get() -> GrpcClient {
  GRPC_CLIENT
    .get_or_init(|| async {
      let server_url = format!("http://{}", CONFIG.server.grpc_address);

      let channel = tonic::transport::Endpoint::new(server_url)
        .unwrap()
        .connect()
        .await
        .unwrap();

      let tokens: Arc<Mutex<TokenStore>> = Default::default();

      let auth_channel = AuthService::new(channel.clone(), tokens.clone());
      let stream = StreamServiceClient::new(auth_channel);

      GrpcClient {
        auth: AuthServiceClient::new(channel.clone()),
        stream,
        tokens,
      }
    })
    .await
    .clone()
  }
