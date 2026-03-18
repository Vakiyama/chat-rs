use bytes::Bytes;
use jwt_simple::prelude::HS256Key;
use tower_http::auth::AsyncRequireAuthorizationLayer;
use utoipa_axum::router::OpenApiRouter;

use crate::api::auth::{JWTAuthorized, JWTKey};

mod auth;

pub fn router() -> OpenApiRouter {
  let key_bytes: Bytes = hex::decode(std::env::var("JWT_KEY").expect("Missing JWT_KEY env var"))
    .expect("Invalid JWT_KEY, decode failed")
    .into();

  let key = HS256Key::from_bytes(&key_bytes);

  OpenApiRouter::new()
    // ------ public api routes --------
    .nest("/auth", auth::router())
    .layer(
      tower::ServiceBuilder::new().layer(AsyncRequireAuthorizationLayer::new(JWTAuthorized {
        key: JWTKey { key }.into(),
      })),
    )
  // ------ private api routes --------
  // hemingway: setup progenitor for app client, with semantics for
  // refresh token requesting on unautorhized reqs
}
