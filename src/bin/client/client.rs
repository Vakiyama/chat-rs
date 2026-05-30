//

use http_body_util::{BodyExt, Full};
use std::{
  io::Bytes,
  pin::Pin,
  sync::{Arc, Mutex},
};
use tokio::sync::OnceCell;
use tonic::{IntoRequest, body::Body, transport::Channel};
use tonic_middleware::{Middleware, ServiceBound};

use crate::client::proto::auth::{RefreshRequest, auth_service_client::AuthServiceClient};

use chat_rs::{
  SERVER_URL, SERVER_URL_HTTP,
  shared::{
    convert::proto::auth::RefreshResponse,
    domain::auth::{RefreshCommand, Token},
  },
};
use http::{Extensions, Request, Response};

#[derive(Default)]
struct TokenStore {
  access_token: Option<Token>,
  refresh_token: Option<Token>,
}
//
// #[derive(Clone)]
// pub struct ApiClient {
//   pub_client: reqwest::Client,
//   private_client: reqwest_middleware::ClientWithMiddleware,
//   tokens: Arc<Mutex<TokenStore>>,
// }
//
// impl ApiClient {
//   pub async fn login(self, body: LoginBody) -> Result<LoginResponse> {
//     let req = self
//       .pub_client
//       .post(format!("http://{}{}", SERVER_URL, "/api/auth/login"))
//       .json(&body)
//       .send()
//       .await?;
//
//     println!("{req:?}");
//     if req.status() == 200 {
//       let res = req.json::<LoginResponse>().await?;
//
//       Ok(res)
//     } else {
//       todo!()
//     }
//   }
//
//   pub async fn verify(self, body: VerifyBody) -> Result<()> {
//     let req = self
//       .pub_client
//       .post(format!("{}{}", SERVER_URL, "/api/auth/verify"))
//       .json(&body)
//       .send()
//       .await?;
//
//     let VerifyResponse {
//       access_token,
//       refresh_token,
//       ..
//     } = req.json::<VerifyResponse>().await?;
//
//     {
//       let mut token_store = self.tokens.lock().unwrap();
//       token_store.access_token = Some(access_token);
//       token_store.refresh_token = Some(refresh_token);
//     }
//
//     Ok(())
//   }
// }
//
// impl Default for ApiClient {
//   fn default() -> Self {
//     let client = reqwest::Client::new();
//     let auth_middleware = AuthMiddleware::default();
//     let private_client = ClientBuilder::new(client).with(auth_middleware).build();
//
//     Self {
//       pub_client: Default::default(),
//       private_client,
//       tokens: Default::default(),
//     }
//   }
// }
//
// // // need some auth middleware between client requests
// // // 1. place to store auth/refresh tokens in memory
// // // 2. handle refresh token semantics on unauthed reqs
// // // 3. some teardown
// //
// // use std::sync::{Arc, Mutex};
// //
// // use tokio::sync::Mutex as AMutex;
// //
// // use chat_rs::{
// //   SERVER_URL,
// //   spec::{
// //     Api,
// //     auth::{AuthClient, RefreshBody, Token},
// //   },
// // };
// // use http::Extensions;
// // use reqwest::{Client, Request, Response};
// // use reqwest_middleware::{ClientBuilder, Middleware, Next, Result};
// // use spec_derive_core::RequestError;
// //
// // // ----------------------------- ApiClient ----------------------------
// //
// // pub struct ApiClient {
// //   pub auth_client: Api, // this "Api" should be given a different name
// //   pub tokens: Arc<Mutex<TokenStore>>,
// // }
// //
// // impl Default for ApiClient {
// //   fn default() -> Self {
// //     let tokens: Arc<Mutex<TokenStore>> = Arc::default();
// //
// //     let middleware = AuthMiddleware {
// //       tokens: tokens.clone(),
// //       auth_client: Arc::new(AMutex::new(Api::new(
// //         format!("http://{}/api/auth", SERVER_URL),
// //         ClientBuilder::new(Client::new()).build(),
// //       ))),
// //     };
// //
// //     let client = ClientBuilder::new(Client::new()).with(middleware).build();
// //
// //     let auth_client = Api::new(format!("http://{}/api/auth", SERVER_URL), client);
// //
// //     Self {
// //       auth_client,
// //       tokens,
// //     }
// //   }
// // }
// //
// // // impl ApiClient {}
// //
// // // ---------------------------- Middleware ---------------------------
// //
// //

// use tonic::body::BoxBody;
use tower::{Service, ServiceBuilder};

#[derive(Clone)]
pub struct AuthService {
  inner: tonic::transport::Channel,
  tokens: Arc<Mutex<TokenStore>>,
}

impl AuthService {
  pub fn new(inner: tonic::transport::Channel) -> Self {
    Self {
      inner,
      tokens: Arc::new(Mutex::new(TokenStore::default())),
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

  fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
    let clone = self.inner.clone();
    let mut inner = std::mem::replace(&mut self.inner, clone);
    let tokens = self.tokens.clone();

    // Capture parts we need to replay before consuming req
    let (parts, body) = req.into_parts();

    Box::pin(async move {
      // Buffer the body bytes so we can replay
      use http_body_util::BodyExt;
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

      // let tokens = Arc::new(Mutex::new(TokenStore::default()));

      // how to create a service:
      // let auth_channel = ;

      HTTPClient {
        auth: AuthServiceClient::new(channel),
      }
    })
    .await
    .clone()
}

fn private_channel(channel: tonic::transport::Channel) -> AuthService {
  tower::ServiceBuilder::new()
    .layer_fn(AuthService::new)
    .service(channel)
}
