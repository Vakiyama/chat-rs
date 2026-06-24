use crate::config::CONFIG;
use chat_shared::convert::auth::proto::auth_service_server::AuthServiceServer;
use chat_shared::convert::post::proto::posts_service_server::PostsServiceServer;
use chat_shared::convert::server::proto::server_service_server::ServerServiceServer;
use chat_shared::convert::stream::proto::stream_service_server::StreamServiceServer;
use chat_shared::convert::user::proto::user_service_server::UserServiceServer;
use tokio_rate_limit::{RateLimiter, RateLimiterConfig};
use tonic::transport::Server;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::api::auth::{
  AuthServer, ClientRateLimitInterceptor, DbTokenStore, InMemoryCodeStore, JWTAuthorizedInterceptor,
};

use crate::api::post::PostsServer;
use crate::api::server::ServerServer;
use crate::api::stream::StreamServer;
use crate::api::user::UserServer;

mod api;
mod config;
mod entities;
mod library;
mod seed;

#[tokio::main]
async fn main() {
  tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::from_default_env())
    .init();

  seed::seed().await;

  let auth_service: AuthServer<InMemoryCodeStore, DbTokenStore> = AuthServer::default();

  let jwt_interceptor = JWTAuthorizedInterceptor::default();
  let per_client_rate_limit_interceptor = ClientRateLimitInterceptor {
    limiter: RateLimiter::new(RateLimiterConfig {
      requests_per_second: 1,
      burst: 5,
    })
    .into(),
  };

  let auth_service = ServiceBuilder::new()
    .layer(tonic_middleware::RequestInterceptorLayer::new(
      per_client_rate_limit_interceptor,
    ))
    .service(AuthServiceServer::new(auth_service));

  let private_stream_service = ServiceBuilder::new()
    .layer(tonic_middleware::RequestInterceptorLayer::new(
      jwt_interceptor.clone(),
    ))
    .service(StreamServiceServer::new(StreamServer::default()));

  let private_user_service = ServiceBuilder::new()
    .layer(tonic_middleware::RequestInterceptorLayer::new(
      jwt_interceptor.clone(),
    ))
    .service(UserServiceServer::new(UserServer));

  let private_server_service = ServiceBuilder::new()
    .layer(tonic_middleware::RequestInterceptorLayer::new(
      jwt_interceptor.clone(),
    ))
    .service(ServerServiceServer::new(ServerServer));

  let private_posts_service = ServiceBuilder::new()
    .layer(tonic_middleware::RequestInterceptorLayer::new(
      jwt_interceptor,
    ))
    .service(PostsServiceServer::new(PostsServer));

  let grpc = Server::builder()
    // Detect dead client connections (network drops) promptly so the voice
    // stream's task ends and runs its presence cleanup, instead of leaving the
    // disconnected user as a ghost in the room. Without keepalive an ungraceful
    // drop isn't noticed until the OS TCP timeout, which can be minutes.
    .http2_keepalive_interval(Some(std::time::Duration::from_secs(10)))
    .http2_keepalive_timeout(Some(std::time::Duration::from_secs(20)))
    .tcp_keepalive(Some(std::time::Duration::from_secs(10)))
    .layer(
      TraceLayer::new_for_grpc().make_span_with(|request: &http::Request<_>| {
        tracing::info_span!(
            "grpc_request",
            method = %request.uri().path(),
        )
      }),
    )
    .add_service(private_stream_service)
    .add_service(private_user_service)
    .add_service(private_server_service)
    .add_service(private_posts_service)
    .add_service(auth_service)
    .serve(CONFIG.server.grpc_address.parse().unwrap());

  println!("gRPC listening on: {}", CONFIG.server.grpc_address);

  grpc
    .await
    .map_err(|e| eprintln!("gRPC exited: {:?}", e))
    .unwrap();
}
