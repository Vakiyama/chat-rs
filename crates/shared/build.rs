fn main() -> Result<(), Box<dyn std::error::Error>> {
  if std::env::var_os("PROTOC").is_none() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();
    unsafe {
      std::env::set_var("PROTOC", protoc_path);
    }
  }

  tonic_prost_build::configure()
    .type_attribute(".", "#[non_exhaustive]")
    .build_server(true)
    .build_client(true)
    .compile_protos(
      &[
        "./src/proto/auth/v1/auth.proto",
        "./src/proto/stream/v1/stream.proto",
        "./src/proto/user/v1/user.proto",
        "./src/proto/server/v1/server.proto",
        "./src/proto/post/v1/post.proto",
      ],
      &["./src/proto/"],
    )?;
  Ok(())
}
