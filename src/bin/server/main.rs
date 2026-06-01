use chat_rs::shared::convert::stream::proto::stream_service_server::StreamServiceServer;
use chat_rs::{SERVER_URL, shared::convert::auth::proto::auth_service_server::AuthServiceServer};
use tonic::transport::Server;
use tower::ServiceBuilder;

use crate::api::auth::{
  AuthServer, InMemoryCodeStore, InMemoryTokenStore, JWTAuthorizedInterceptor,
};

use crate::api::stream::StreamServer;

mod api;
mod library;

#[tokio::main]
async fn main() {
  let _env = dotenvy::dotenv();

  let auth_service: AuthServer<InMemoryCodeStore, InMemoryTokenStore> = AuthServer::default();
  let jwt_interceptor = JWTAuthorizedInterceptor::default();

  let public_service = ServiceBuilder::new().service(AuthServiceServer::new(auth_service));

  let private_stream_service = ServiceBuilder::new()
    .layer(tonic_middleware::RequestInterceptorLayer::new(
      jwt_interceptor,
    ))
    .service(StreamServiceServer::new(StreamServer::default()));

  let grpc = Server::builder()
    .add_service(private_stream_service)
    .add_service(public_service)
    .serve(SERVER_URL.parse().unwrap());

  println!("gRPC listening on: {SERVER_URL}");

  grpc
    .await
    .map_err(|e| eprintln!("gRPC exited: {:?}", e))
    .unwrap();
}
