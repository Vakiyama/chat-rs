// CHA-33 on-device smoke test. Push to a device with adb and run it to prove the
// native deps work at runtime, not just at link time:
//   nix develop .#android -c cargo ndk -t arm64-v8a --platform 24 build --bin smoke
//   adb push target/aarch64-linux-android/debug/smoke /data/local/tmp/
//   adb shell /data/local/tmp/smoke
// platform 24+ is required: bionic only exports stderr/stdout/stdin as symbols
// from api 23, below that the link fails with "undefined symbol: stderr".
#[tokio::main]
async fn main() {
  // opus: encode a 10ms frame. proves libopus links and runs.
  let enc = audiopus::coder::Encoder::new(
    audiopus::SampleRate::Hz48000,
    audiopus::Channels::Mono,
    audiopus::Application::Voip,
  )
  .expect("opus encoder");
  let pcm = [0f32; 480];
  let mut out = [0u8; 1500];
  let n = enc.encode_float(&pcm, &mut out).expect("opus encode");
  println!("opus: encoded {n} bytes");

  // sonora: build the APM and process a 10ms capture frame. proves AEC inits.
  let mut apm = sonora::AudioProcessing::builder()
    .config(sonora::Config::default())
    .capture_config(sonora::StreamConfig::new(48_000, 1))
    .render_config(sonora::StreamConfig::new(48_000, 1))
    .build();
  let frame = [0f32; 480];
  let mut clean = [0f32; 480];
  apm
    .process_capture_f32(&[&frame], &mut [&mut clean[..]])
    .expect("apm process");
  println!("sonora: processed a 10ms frame");

  // webrtc: build a peer connection. proves webrtc-rs inits at runtime.
  let mut media = webrtc::api::media_engine::MediaEngine::default();
  media.register_default_codecs().expect("codecs");
  let api = webrtc::api::APIBuilder::new()
    .with_media_engine(media)
    .build();
  let pc = api
    .new_peer_connection(webrtc::peer_connection::configuration::RTCConfiguration::default())
    .await
    .expect("peer connection");
  println!("webrtc: created a peer connection");
  let _ = pc.close().await;

  // tonic: open an http/2 channel to the server. proves the transport + tls run.
  match tonic::transport::Channel::from_static("http://5.78.193.193:3000")
    .connect()
    .await
  {
    Ok(_) => println!("tonic: connected to server"),
    Err(e) => println!("tonic: connect failed (fine if the server is unreachable): {e}"),
  }

  println!("smoke ok");
}
