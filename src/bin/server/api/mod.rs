use axum::Router;
use bytes::Bytes;
use chat_rs::spec::{
  self,
  auth::{JWTAuthorized, JWTKey},
};
use jwt_simple::prelude::HS256Key;
use tower_http::auth::AsyncRequireAuthorizationLayer;

mod auth;

pub fn router() -> Router {
  let key_bytes: Bytes = hex::decode(std::env::var("JWT_KEY").expect("Missing JWT_KEY env var"))
    .expect("Invalid JWT_KEY, decode failed")
    .into();

  let key = HS256Key::from_bytes(&key_bytes);

  Router::new()
    // ------ public api routes --------
    .nest("/auth", spec::auth::auth_handler())
    .layer(
      tower::ServiceBuilder::new().layer(AsyncRequireAuthorizationLayer::new(JWTAuthorized {
        key: JWTKey { key }.into(),
      })),
    )
  // ------ private api routes --------
}
