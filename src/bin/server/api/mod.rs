use axum::Router;

mod auth;

pub fn router() -> Router {
  Router::new().nest("/auth", auth::router())
}

