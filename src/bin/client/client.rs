struct ApiClient {
  private: PrivateClient,
  public: PubClient,
}

struct PrivateClient {}

struct PubClient {}

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
// #[derive(Default)]
// struct TokenStore {
//   access_token: Option<Token>,
//   refresh_token: Option<Token>,
// }
//
// #[derive(Clone)]
// struct AuthMiddleware {
//   tokens: Arc<Mutex<TokenStore>>,
//   auth_client: Arc<AMutex<Api>>,
// }
//
// #[async_trait::async_trait]
// impl Middleware for AuthMiddleware {
//   async fn handle(
//     &self,
//     mut req: Request,
//     extensions: &mut Extensions,
//     next: Next<'_>,
//   ) -> Result<Response> {
//     println!("middleware handle fired");
//     if let Some(token) = &self.tokens.lock().unwrap().access_token {
//       req.headers_mut().insert(
//         http::header::AUTHORIZATION,
//         format!("Bearer {}", token).parse().unwrap(),
//       );
//     }
//
//     let cloned = req.try_clone();
//     let res = next.clone().run(req, extensions).await?;
//
//     if res.status() == http::StatusCode::UNAUTHORIZED
//       && let Some(cloned_req) = cloned
//     {
//       self.refresh().await?;
//       return next.run(cloned_req, extensions).await;
//     }
//
//     Ok(res)
//   }
// }
//
// impl AuthMiddleware {
//   async fn refresh(&self) -> reqwest_middleware::Result<()> {
//     let refresh_token = { self.tokens.lock().unwrap().refresh_token.clone() }; // lock dropped here
//
//     let Some(token) = refresh_token else {
//       return Ok(());
//     };
//
//     let res = self
//       .auth_client
//       .lock()
//       .await
//       .refresh_handler(RefreshBody {
//         refresh_token: token,
//       })
//       .await;
//
//     match res {
//       Ok(new_tokens) => {
//         let mut store = self.tokens.lock().unwrap();
//         store.access_token = Some(new_tokens.access_token);
//         store.refresh_token = Some(new_tokens.refresh_token);
//
//         Ok(())
//       }
//       Err(RequestError::Server(e)) => {
//         eprintln!("{e:?}");
//         Err(reqwest_middleware::Error::Middleware(anyhow::anyhow!(
//           "Server error when hitting refresh"
//         )))
//       }
//       Err(RequestError::Network(e)) => Err(e),
//       Err(RequestError::Decode(e)) => {
//         eprintln!("{e:?}");
//         Err(reqwest_middleware::Error::Middleware(anyhow::anyhow!(
//           "Decode error when hitting refresh"
//         )))
//       }
//     }
//   }
// }
