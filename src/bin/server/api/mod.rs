use utoipa_axum::router::OpenApiRouter;

mod auth;

pub fn router() -> OpenApiRouter {
  OpenApiRouter::new().nest("/auth", auth::router())
}
