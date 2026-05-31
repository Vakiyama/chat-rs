fn main() -> Result<(), Box<dyn std::error::Error>> {
  let protoc_path = protoc_bin_vendored::protoc_bin_path().unwrap();
  unsafe {
    std::env::set_var("PROTOC", protoc_path);
  }

  tonic_prost_build::configure()
    .type_attribute(".", "#[non_exhaustive]")
    .build_server(true)
    .build_client(true)
    .compile_protos(
      &[
        "./src/shared/proto/auth/v1/auth.proto",
        "./src/shared/proto/stream/v1/stream.proto",
      ],
      &["./src/shared/proto/"],
    )?;
  Ok(())
}
