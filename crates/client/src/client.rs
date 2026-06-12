use crate::config::CONFIG;
use chat_shared::convert::IntoProto;
use chat_shared::convert::auth::proto::auth_service_client::AuthServiceClient;
use chat_shared::convert::stream::proto::stream_service_client::StreamServiceClient;
use chat_shared::convert::user::proto::user_service_client::UserServiceClient;
use chat_shared::domain::auth::RefreshCommand;
use chat_shared::domain::auth::Token;
use http_body_util::BodyExt;
use http_body_util::Full;
use keyring::Entry;
use std::{pin::Pin, sync::Arc};
use tokio::sync::{Mutex, OnceCell};
use tonic::transport::Channel;
use tower::Service;

pub async fn store_refresh_token(refresh_token: String) -> keyring::Result<()> {
  let service = CONFIG.keyring_service_name.clone();
  let user = CONFIG.keyring_user.clone();
  on_keyring_thread(move || Entry::new(&service, &user)?.set_password(&refresh_token)).await
}

pub async fn load_refresh_token() -> keyring::Result<String> {
  let service = CONFIG.keyring_service_name.clone();
  let user = CONFIG.keyring_user.clone();
  on_keyring_thread(move || Entry::new(&service, &user)?.get_password()).await
}

pub async fn clear_refresh_token() -> keyring::Result<()> {
  let service = CONFIG.keyring_service_name.clone();
  let user = CONFIG.keyring_user.clone();
  on_keyring_thread(move || Entry::new(&service, &user)?.delete_credential()).await
}

async fn on_keyring_thread<F, T>(f: F) -> T
where
  F: FnOnce() -> T + Send + 'static,
  T: Send + 'static,
{
  let (tx, rx) = tokio::sync::oneshot::channel();
  std::thread::spawn(move || {
    let _ = tx.send(f());
  });
  rx.await.expect("keyring thread panicked")
}

#[derive(Default)]
pub struct TokenStore {
  access_token: Option<Token>,
  refresh_token: Option<Token>,
}

#[derive(Clone)]
pub struct AuthService {
  inner: tonic::transport::Channel,
  tokens: Arc<Mutex<TokenStore>>,
  with_buffer_replay: bool,
}

impl AuthService {
  pub fn new(
    inner: tonic::transport::Channel,
    tokens: Arc<Mutex<TokenStore>>,
    with_buffer_replay: bool,
  ) -> Self {
    Self {
      inner,
      tokens,
      with_buffer_replay,
    }
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

  fn call(&mut self, mut req: http::Request<tonic::body::Body>) -> Self::Future {
    let clone = self.inner.clone();
    let mut inner = std::mem::replace(&mut self.inner, clone);
    let tokens = self.tokens.clone();
    let with_replay = self.with_buffer_replay;

    // Capture parts we need to replay before consuming req

    Box::pin(async move {
      // Buffer the body bytes so we can replay

      if with_replay {
        let (parts, body) = req.into_parts();
        let body_bytes = body.collect().await?.to_bytes();
        let make_req = async |tokens: &Arc<Mutex<TokenStore>>| {
          let mut builder = http::Request::builder()
            .method(parts.method.clone())
            .uri(parts.uri.clone())
            .version(parts.version);
          // Clone headers
          *builder.headers_mut().unwrap() = parts.headers.clone();

          let body = tonic::body::Body::new(Full::new(body_bytes.clone()).map_err(|e| match e {}));

          let mut req = builder.body(body).unwrap();
          // Attach current token
          if let Some(token) = &tokens.lock().await.access_token {
            req.headers_mut().insert(
              http::header::AUTHORIZATION,
              format!("Bearer {}", token).parse().unwrap(),
            );
          }
          req
        };

        let res = inner.call(make_req(&tokens).await).await?;

        if res.status() != http::StatusCode::UNAUTHORIZED {
          return Ok(res);
        }

        let refresh_token = tokens.lock().await.refresh_token.clone();
        let Some(refresh_token) = refresh_token else {
          return Ok(res);
        };

        let token_res = get()
          .await
          .auth
          .refresh(RefreshCommand { refresh_token }.into_proto())
          .await
          .map(|req| req.into_inner());

        match token_res {
          Ok(token_res) => {
            {
              let mut store = tokens.lock().await;
              store.access_token = Some(token_res.access_token);
              store.refresh_token = Some(token_res.refresh_token);
            }

            inner
              .call(make_req(&tokens).await)
              .await
              .map_err(Into::into)
          }
          Err(e) => {
            {
              let mut store = tokens.lock().await;
              store.access_token = None;
              store.refresh_token = None;

              let _ = clear_refresh_token()
                .await
                .map_err(|e| eprintln!("Failed to clear tokens from credential store: {e}"));
            }

            Err(e)?
          }
        }
        // Retry with fresh token
      } else if let Some(token) = &tokens.lock().await.access_token {
        req.headers_mut().insert(
          http::header::AUTHORIZATION,
          format!("Bearer {}", token).parse().unwrap(),
        );

        let res = inner.call(req).await?;

        Ok(res)
      } else {
        let res = inner.call(req).await?;
        eprintln!("Warning: request being made with no tokens in private channel.");

        Ok(res)
      }
    })
  }
}

#[non_exhaustive]
#[derive(Clone)]
pub struct GrpcClient {
  pub auth: AuthServiceClient<Channel>,
  pub stream: StreamServiceClient<AuthService>,
  pub user: UserServiceClient<AuthService>,
  tokens: Arc<Mutex<TokenStore>>,
}

impl GrpcClient {
  async fn load_from_credential_store(&self) -> Result<(), ()> {
    let refresh_token = match load_refresh_token().await {
      Ok(t) => t,
      Err(_) => {
        println!("No creds to load");
        return Err(());
      }
    };

    let mut auth = self.auth.clone();
    let resp = match auth
      .refresh(RefreshCommand { refresh_token }.into_proto())
      .await
    {
      Ok(r) => r.into_inner(),
      Err(_) => {
        let _ = clear_refresh_token().await;
        println!("Stored refresh token rejected; cleared");
        return Err(());
      }
    };

    if let Err(e) = store_refresh_token(resp.refresh_token.clone()).await {
      eprintln!("Failed to persist rotated refresh token: {e}");
    }
    let mut store = self.tokens.lock().await;
    store.access_token = Some(resp.access_token);
    store.refresh_token = Some(resp.refresh_token);
    println!("Loaded refresh token from system credential store.");

    Ok(())
  }

  pub async fn insert_tokens(&self, refresh_token: String, access_token: String) {
    let mut tokens = self.tokens.lock().await;

    let result = store_refresh_token(refresh_token.clone()).await;
    match result {
      Ok(_) => println!("Success storing refresh token"),
      Err(e) => println!("Failed to store token: {:?}", e),
    }

    tokens.access_token = Some(access_token);
    tokens.refresh_token = Some(refresh_token);
  }

  pub async fn has_tokens(&self) -> bool {
    let tokens = self.tokens.lock().await;

    tokens.access_token.is_some() && tokens.refresh_token.is_some()
  }
}

static GRPC_CLIENT: tokio::sync::OnceCell<GrpcClient> = OnceCell::const_new();

pub async fn get() -> GrpcClient {
  GRPC_CLIENT
    .get_or_init(|| async {
      let channel: Channel;

      loop {
        let server_url = CONFIG.server_url.to_string();
        let channel_connect = tonic::transport::Endpoint::new(server_url.clone())
          .unwrap_or_else(|_| panic!("Failed to parse server url {}", &server_url))
          .connect()
          .await;

        match channel_connect {
          Ok(connected) => {
            channel = connected;
            println!("Connected to grpc server.",);
            break;
          }
          Err(_) => {
            println!("Could not connect to grpc server at {}", CONFIG.server_url);
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            println!("Retrying...");
            continue;
          }
        }
      }

      let tokens: Arc<Mutex<TokenStore>> = Default::default();

      let auth_channel_no_replay = AuthService::new(channel.clone(), tokens.clone(), false);
      let auth_channel = AuthService::new(channel.clone(), tokens.clone(), true);

      let stream = StreamServiceClient::new(auth_channel_no_replay);
      let user = UserServiceClient::new(auth_channel);

      let client = GrpcClient {
        auth: AuthServiceClient::new(channel.clone()),
        stream,
        tokens,
        user,
      };

      let _ = client.load_from_credential_store().await;

      client
    })
    .await
    .clone()
}
