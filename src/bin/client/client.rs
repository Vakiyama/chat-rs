use std::sync::{Arc, Mutex};

use chat_rs::{
  SERVER_URL,
  shared::auth::{
    LoginBody, LoginResponse, RefreshBody, RefreshResponse, Token, VerifyBody, VerifyResponse,
  },
};
use http::Extensions;
use reqwest::{Client, Request, Response};
use reqwest_middleware::{ClientBuilder, Middleware, Next, Result};
use uuid::Uuid;

#[derive(Default)]
struct TokenStore {
  access_token: Option<Token>,
  refresh_token: Option<Token>,
}

pub struct ApiClient {
  pub_client: reqwest::Client,
  private_client: reqwest_middleware::ClientWithMiddleware,
  tokens: Arc<Mutex<TokenStore>>,
}

impl ApiClient {
  pub async fn login(self, email: String) -> Result<Uuid> {
    let req = self
      .pub_client
      .post(format!("{}{}", SERVER_URL, "/api/auth/login"))
      .json(&LoginBody { email })
      .send()
      .await?;

    let LoginResponse { identifier } = req.json::<LoginResponse>().await?;

    Ok(identifier)
  }

  pub async fn verify(self, code: String, identifier: Uuid, email: String) -> Result<()> {
    let req = self
      .pub_client
      .post(format!("{}{}", SERVER_URL, "/api/auth/verify"))
      .json(&VerifyBody {
        identifier,
        email,
        code,
      })
      .send()
      .await?;

    let VerifyResponse {
      access_token,
      refresh_token,
      ..
    } = req.json::<VerifyResponse>().await?;

    {
      let mut token_store = self.tokens.lock().unwrap();
      token_store.access_token = Some(access_token);
      token_store.refresh_token = Some(refresh_token);
    }

    Ok(())
  }
}

impl Default for ApiClient {
  fn default() -> Self {
    let client = reqwest::Client::new();
    let auth_middleware = AuthMiddleware::default();
    let private_client = ClientBuilder::new(client).with(auth_middleware).build();

    Self {
      pub_client: Default::default(),
      private_client,
      tokens: Default::default(),
    }
  }
}

// // need some auth middleware between client requests
// // 1. place to store auth/refresh tokens in memory
// // 2. handle refresh token semantics on unauthed reqs
// // 3. some teardown
//
// use std::sync::{Arc, Mutex};
//
// use tokio::sync::Mutex as AMutex;
//
// use chat_rs::{
//   SERVER_URL,
//   spec::{
//     Api,
//     auth::{AuthClient, RefreshBody, Token},
//   },
// };
// use http::Extensions;
// use reqwest::{Client, Request, Response};
// use reqwest_middleware::{ClientBuilder, Middleware, Next, Result};
// use spec_derive_core::RequestError;
//
// // ----------------------------- ApiClient ----------------------------
//
// pub struct ApiClient {
//   pub auth_client: Api, // this "Api" should be given a different name
//   pub tokens: Arc<Mutex<TokenStore>>,
// }
//
// impl Default for ApiClient {
//   fn default() -> Self {
//     let tokens: Arc<Mutex<TokenStore>> = Arc::default();
//
//     let middleware = AuthMiddleware {
//       tokens: tokens.clone(),
//       auth_client: Arc::new(AMutex::new(Api::new(
//         format!("http://{}/api/auth", SERVER_URL),
//         ClientBuilder::new(Client::new()).build(),
//       ))),
//     };
//
//     let client = ClientBuilder::new(Client::new()).with(middleware).build();
//
//     let auth_client = Api::new(format!("http://{}/api/auth", SERVER_URL), client);
//
//     Self {
//       auth_client,
//       tokens,
//     }
//   }
// }
//
// // impl ApiClient {}
//
// // ---------------------------- Middleware ---------------------------
//
//

#[derive(Clone, Default)]
struct AuthMiddleware {
  tokens: Arc<Mutex<TokenStore>>,
  client: Arc<Client>,
}

#[async_trait::async_trait]
impl Middleware for AuthMiddleware {
  async fn handle(
    &self,
    mut req: Request,
    extensions: &mut Extensions,
    next: Next<'_>,
  ) -> Result<Response> {
    println!("middleware handle fired");
    if let Some(token) = &self.tokens.lock().unwrap().access_token {
      req.headers_mut().insert(
        http::header::AUTHORIZATION,
        format!("Bearer {}", token).parse().unwrap(),
      );
    }

    let cloned = req.try_clone();
    let res = next.clone().run(req, extensions).await?;

    if res.status() == http::StatusCode::UNAUTHORIZED
      && let Some(cloned_req) = cloned
    {
      self.refresh().await?;
      return next.run(cloned_req, extensions).await;
    }

    Ok(res)
  }
}

impl AuthMiddleware {
  async fn refresh(&self) -> reqwest_middleware::Result<()> {
    let refresh_token = { self.tokens.lock().unwrap().refresh_token.clone() };

    let Some(token) = refresh_token else {
      return Ok(());
    };

    let res = self
      .client
      .post(format!("{}{}", SERVER_URL, "/api/auth/refresh"))
      .json(&RefreshBody {
        refresh_token: token,
      })
      .send()
      .await
      .map_err(|e| reqwest_middleware::Error::Reqwest(e))?;

    let new_tokens = res.json::<RefreshResponse>().await.unwrap();

    {
      let mut store = self.tokens.lock().unwrap();

      store.access_token = Some(new_tokens.access_token);
      store.refresh_token = Some(new_tokens.refresh_token);
    }

    Ok(())
  }
}
