use crate::audio_processing::mixer::Mixer;
use crate::audio_processing::resampler::Resampler;
use crate::client;
use chat_shared::convert::stream::proto::ClientVoiceMessage;
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleRate, SupportedStreamConfig};
use futures::channel::mpsc;
use futures::stream::StreamExt;
use iced::futures;
use iced::task::{Never, Sipper, sipper};
use sonora::config::{
  AdaptiveDigital, EchoCanceller, FixedDigital, GainController2, NoiseSuppression,
};
use sonora::{AudioProcessing, Config};
use std::sync::Arc;
use uuid::Uuid;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_OPUS, MediaEngine};
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

#[derive(Debug, Clone)]
pub struct WebRTCConnection(mpsc::Sender<ClientVoiceMessage>);

impl WebRTCConnection {
  pub fn send(&mut self, message: ClientVoice) {
    self
      .0
      .try_send(message.into_proto())
      .expect("Send message to server");
  }
}

#[derive(Debug, Clone)]
pub enum Event {
  Connected(WebRTCConnection),
  Disconnected,
  MessageReceived(Box<ServerVoice>),
}

impl From<Event> for crate::Message {
  fn from(val: Event) -> Self {
    match val {
      Event::Connected(connection) => crate::Message::WebRTCSignalStreamConnected(connection),
      Event::Disconnected => crate::Message::WebRTCSignalStreamDisconnected,
      Event::MessageReceived(server_message) => crate::Message::WebRTC(server_message),
    }
  }
}

pub fn connect() -> impl Sipper<Never, Event> {
  sipper(async |mut output| {
    // reconnect loop; awaits 1 second on disconnect/error and retries
    loop {
      let (tx, rx) = mpsc::channel::<ClientVoiceMessage>(100);
      let mut receiver = match client::get().await.stream.connect_voice_stream(rx).await {
        Ok(response) => {
          println!("Firing webrtc connect event.");
          output.send(Event::Connected(WebRTCConnection(tx))).await;

          response.into_inner().fuse()
        }
        Err(_) => {
          tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
          continue;
        }
      };

      loop {
        match receiver.select_next_some().await {
          Ok(server_msg) => match server_msg.try_into_domain().map(Box::new) {
            Ok(msg) => output.send(Event::MessageReceived(msg)).await,
            Err(e) => println!("Error parsing incoming bidi server msg. {e}"),
          },
          Err(_) => {
            output.send(Event::Disconnected).await;
            break;
          }
        }
      }
    }
  })
}

// handle messages

pub const TARGET_RATE: u32 = 48_000;

/// What the mic stream sends to the audio processor. `Rate` is emitted once when
/// the input device (and thus its native sample rate) changes live, so the
/// processor can rebuild its capture resampler before the new samples arrive.
pub enum CaptureEvent {
  Samples(Vec<f32>),
  Rate(u32),
}

// cpal 0.18 exposes the device name through `Display` (`to_string()`), not a
// `name()` accessor — that's our stable identifier for a saved device choice.
fn resolve_input_device(host: &cpal::Host, name: Option<&str>) -> anyhow::Result<Device> {
  if let Some(name) = name {
    for dev in host.input_devices()? {
      if dev.to_string() == name {
        return Ok(dev);
      }
    }
    eprintln!("input device {name:?} not found; falling back to default");
  }
  host
    .default_input_device()
    .ok_or_else(|| anyhow::anyhow!("no input device"))
}

fn resolve_output_device(host: &cpal::Host, name: Option<&str>) -> anyhow::Result<Device> {
  if let Some(name) = name {
    for dev in host.output_devices()? {
      if dev.to_string() == name {
        return Ok(dev);
      }
    }
    eprintln!("output device {name:?} not found; falling back to default");
  }
  host
    .default_output_device()
    .ok_or_else(|| anyhow::anyhow!("no output device"))
}

// ALSA enumerates a flood of virtual/plugin PCMs (and outright duplicate names).
// Drop the obvious plugin entries and de-duplicate so the picker is readable.
// "default"/"pulse"/"pipewire" and real hardware survive; if the filter would
// empty the list we fall back to the merely-deduped one as a safety net.
fn clean_device_list(names: Vec<String>) -> Vec<String> {
  const NOISE_PREFIXES: &[&str] = &[
    "sysdefault",
    "samplerate",
    "speexrate",
    "upmix",
    "vdownmix",
    "dmix",
    "dsnoop",
    "surround",
    "iec958",
    "spdif",
    "modem",
    "phoneline",
    "usbstream",
    "null",
    "oss",
  ];

  let dedup = |iter: &mut dyn Iterator<Item = String>| -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    iter.filter(|n| seen.insert(n.clone())).collect()
  };

  let filtered = dedup(
    &mut names
      .iter()
      .filter(|n| {
        let lower = n.to_ascii_lowercase();
        !NOISE_PREFIXES.iter().any(|p| lower.starts_with(p))
      })
      .cloned(),
  );

  if filtered.is_empty() {
    dedup(&mut names.into_iter())
  } else {
    filtered
  }
}

/// Enumerate input device names for the settings picker. Best-effort: returns an
/// empty list rather than erroring so the UI can still render.
pub fn list_input_devices() -> Vec<String> {
  clean_device_list(
    cpal::default_host()
      .input_devices()
      .map(|devs| devs.map(|d| d.to_string()).collect())
      .unwrap_or_default(),
  )
}

/// Enumerate output device names for the settings picker.
pub fn list_output_devices() -> Vec<String> {
  clean_device_list(
    cpal::default_host()
      .output_devices()
      .map(|devs| devs.map(|d| d.to_string()).collect())
      .unwrap_or_default(),
  )
}

/// A lightweight standalone mic-level meter for the settings screen, independent
/// of any call. Runs its own capture stream on a dedicated thread (cpal streams
/// aren't `Send`) and publishes a smoothed RMS level (f32 bits) that the UI
/// polls. Dropping it stops the stream.
pub struct MicMonitor {
  stop: Arc<std::sync::atomic::AtomicBool>,
  level: Arc<std::sync::atomic::AtomicU32>,
}

impl MicMonitor {
  pub fn start(device_name: Option<String>) -> Self {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let stop_thread = stop.clone();
    let level_thread = level.clone();

    std::thread::spawn(move || {
      let host = cpal::default_host();
      let dev = match resolve_input_device(&host, device_name.as_deref()) {
        Ok(d) => d,
        Err(e) => {
          eprintln!("mic monitor: no input device: {e:?}");
          return;
        }
      };
      let cfg = match (dev.default_input_config(), dev.supported_input_configs()) {
        (Ok(default), Ok(ranges)) => match pick_config(default, ranges) {
          Ok(c) => c,
          Err(e) => {
            eprintln!("mic monitor: pick config: {e:?}");
            return;
          }
        },
        (Err(e), _) | (_, Err(e)) => {
          eprintln!("mic monitor: query config: {e:?}");
          return;
        }
      };
      let stream_cfg: cpal::StreamConfig = cfg.into();

      // peak-hold meter: snaps up to transients, decays smoothly.
      let mut smoothed = 0f32;
      let stream = dev.build_input_stream(
        stream_cfg,
        move |data: &[f32], _| {
          let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len().max(1) as f32).sqrt();
          smoothed = (smoothed * 0.85).max(rms);
          level_thread.store(smoothed.to_bits(), Ordering::Relaxed);
        },
        |err| eprintln!("mic monitor input error: {err}"),
        None,
      );
      let stream = match stream {
        Ok(s) => s,
        Err(e) => {
          eprintln!("mic monitor: build stream failed: {e}");
          return;
        }
      };
      if let Err(e) = stream.play() {
        eprintln!("mic monitor: play failed: {e}");
        return;
      }

      while !stop_thread.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
      }
      // stream dropped here → capture stops.
    });

    MicMonitor { stop, level }
  }

  /// Latest smoothed input RMS (0.0..~1.0).
  pub fn level(&self) -> f32 {
    f32::from_bits(self.level.load(std::sync::atomic::Ordering::Relaxed))
  }
}

impl Drop for MicMonitor {
  fn drop(&mut self) {
    self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
  }
}

/// Build (and start) a mic capture stream on the named device (or the default),
/// returning the stream and its native sample rate. Used both at call setup and
/// when the user switches input device live.
pub fn build_mic(
  cap_tx: tokio::sync::mpsc::UnboundedSender<CaptureEvent>,
  device_name: Option<&str>,
) -> anyhow::Result<(cpal::Stream, u32)> {
  let host = cpal::default_host();
  let dev = resolve_input_device(&host, device_name)?;
  let cfg = pick_config(dev.default_input_config()?, dev.supported_input_configs()?)?;
  let rate = cfg.sample_rate();
  let stream = spawn_mic(cap_tx, cfg, dev)?;
  Ok((stream, rate))
}

/// Build (and start) a speaker stream on the named device (or the default),
/// reusing the given mixer and render tee, returning the stream and its native
/// sample rate. A live swap is only seamless when the rate matches the mixer's.
pub fn build_speaker(
  mixer: Mixer,
  render_tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>,
  device_name: Option<&str>,
) -> anyhow::Result<(cpal::Stream, u32)> {
  let host = cpal::default_host();
  let dev = resolve_output_device(&host, device_name)?;
  let cfg = pick_config(dev.default_output_config()?, dev.supported_output_configs()?)?;
  let rate = cfg.sample_rate();
  let stream = spawn_speaker(mixer, render_tx, cfg, dev)?;
  Ok((stream, rate))
}

fn pick_config(
  default: cpal::SupportedStreamConfig,
  ranges: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
) -> anyhow::Result<cpal::SupportedStreamConfig> {
  // shared-mode WASAPI only grants the device's current mix format, so trust the default first
  if default.sample_format() == cpal::SampleFormat::F32 {
    return Ok(default);
  }
  // default isn't f32: look for any f32 range that contains the default's rate
  let rate = default.sample_rate();
  let mut f32_ranges: Vec<_> = ranges
    .filter(|r| {
      r.sample_format() == cpal::SampleFormat::F32
        && r.min_sample_rate() <= rate
        && r.max_sample_rate() >= rate
    })
    .collect();
  if let Some(range) = f32_ranges.pop() {
    return Ok(range.with_sample_rate(rate));
  }
  anyhow::bail!("no f32 config at device rate {rate}")
}

fn spawn_mic(
  tx: tokio::sync::mpsc::UnboundedSender<CaptureEvent>,
  config: SupportedStreamConfig,
  device: Device,
) -> anyhow::Result<cpal::Stream> {
  let channels = config.channels();

  let stream_config: cpal::StreamConfig = config.into();

  let stream = device.build_input_stream(
    stream_config,
    move |data: &[f32], _| {
      // cpal invokes this from the OS audio thread through a C boundary; a panic
      // unwinding back into it is UB and aborts the process on Windows. Contain
      // any panic here. A zero channel count would make `chunks(0)` panic.
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if channels == 0 {
          return;
        }
        let mono: Vec<f32> = data
          .chunks(channels.into())
          .map(|frame| frame.iter().sum::<f32>() / channels as f32)
          .collect();

        let _ = tx.send(CaptureEvent::Samples(mono));
      }));
      if result.is_err() {
        eprintln!("cpal input callback panicked; dropping buffer");
      }
    },
    |err| eprintln!("cpal input error: {err}"),
    None,
  )?;

  stream.play()?;

  Ok(stream) // when this stream drops, the mic stops.
}

fn spawn_speaker(
  mixer: Mixer,
  render_tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>,
  config: SupportedStreamConfig,
  device: Device,
) -> anyhow::Result<cpal::Stream> {
  let stream_config: cpal::StreamConfig = config.into();
  let channels = stream_config.channels as usize;
  let mut scratch: Vec<f32> = Vec::new();

  let stream = device.build_output_stream(
    stream_config,
    move |data: &mut [f32], _| {
      // cpal invokes this from the OS audio thread through a C boundary; a panic
      // unwinding back into it is UB and aborts the process on Windows. Contain
      // any panic here. `channels == 0` would make `data.len() / channels` and
      // `chunks_mut(0)` panic — guard it and emit silence instead.
      let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if channels == 0 {
          data.fill(0.0);
          return;
        }
        let frames = data.len() / channels;
        scratch.resize(frames, 0.0);
        mixer.mix_mono(&mut scratch); // mono mix, no channel logic
        for (frame, s) in data.chunks_mut(channels).zip(scratch.iter()) {
          frame.fill(*s); // upmix to device channels
        }
        let _ = render_tx.send(scratch.clone()); // tee to the APM
      }));
      if result.is_err() {
        data.fill(0.0); // keep the stream alive; output silence this cycle
        eprintln!("cpal output callback panicked; silencing buffer");
      }
    },
    |err| eprintln!("cpal output error: {err}"),
    None,
  )?;
  stream.play()?;
  Ok(stream)
}

/// Everything the voice actor needs to own and drive a call's audio path. The
/// channels and rates are kept so the actor can rebuild the mic or speaker
/// stream live when the user switches device.
pub struct CallSetup {
  pub pc: RTCPeerConnection,
  pub offer: RTCSessionDescription,
  // `None` means that device couldn't be opened — we still join the call, but
  // that direction is silent until the user fixes/switches the device. A missing
  // mic means nobody hears us; a missing speaker means we hear nobody.
  pub mic: Option<cpal::Stream>,
  pub speaker: Option<cpal::Stream>,
  pub mixer: Mixer,
  pub in_rate: u32,
  pub out_rate: u32,
  pub cap_tx: tokio::sync::mpsc::UnboundedSender<CaptureEvent>,
  pub rnd_tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>,
  // monotonically counts mic capture buffers that carried real signal (peak
  // above a digital-silence floor). A poller watches it for staleness to detect
  // a mic that opened but delivers nothing — either no frames at all, or frames
  // of pure silence (unplugged / OS-muted / wrong input). Survives live device
  // swaps since it's tied to the processor, not the cpal stream.
  pub capture_signal_frames: Arc<std::sync::atomic::AtomicU64>,
}

#[allow(clippy::too_many_arguments)]
pub async fn setup_client(
  voice_channel_id: Uuid,
  conn: WebRTCConnection,
  muted: Arc<std::sync::atomic::AtomicBool>,
  deafened: Arc<std::sync::atomic::AtomicBool>,
  gate_threshold: Arc<std::sync::atomic::AtomicU32>,
  input_name: Option<String>,
  output_name: Option<String>,
) -> anyhow::Result<CallSetup> {
  let mut media_engine = MediaEngine::default();

  media_engine.register_default_codecs()?;
  let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
  let api = APIBuilder::new()
    .with_media_engine(media_engine)
    .with_interceptor_registry(registry)
    .build();

  let config = RTCConfiguration {
    ice_servers: vec![RTCIceServer {
      urls: vec!["stun:stun.l.google.com:19302".to_owned()],
      ..Default::default()
    }],
    ..Default::default()
  };

  let client = api
    .new_peer_connection(config)
    .await
    .map_err(|e| anyhow::anyhow!("Error setting up new peer conn {e:?}"))?;

  let mic_track = Arc::new(TrackLocalStaticSample::new(
    RTCRtpCodecCapability {
      mime_type: MIME_TYPE_OPUS.to_owned(),
      ..Default::default()
    },
    "audio".into(),
    "mic".into(),
  ));
  let sender = client.add_track(mic_track.clone()).await?;

  tokio::spawn(async move {
    let mut buf = vec![0u8; 1500];
    while sender.read(&mut buf).await.is_ok() {}
  });

  let (cap_tx, cap_rx) = tokio::sync::mpsc::unbounded_channel::<CaptureEvent>();
  let (rnd_tx, rnd_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();

  // The output rate is fixed here for the life of the call: the mixer's jitter
  // buffer, every per-peer resampler, and the APM render config are all sized to
  // it. A live speaker swap is only seamless when the new device shares this rate.
  //
  // A dead output device is non-fatal: we still join (so the user hears the rest
  // of the UI react and can fix devices from Settings), just with no speaker.
  // The mixer self-trims each source to 200ms, so leaving it undrained can't grow
  // unbounded. When no device is resolvable we size the mixer/APM render at the
  // target rate so the rest of the pipeline still has a coherent rate to use.
  let (cpal_stream_output, out_rate, mixer) = match resolve_output_device(
    &cpal::default_host(),
    output_name.as_deref(),
  )
  .and_then(|dev| {
    let cfg = pick_config(dev.default_output_config()?, dev.supported_output_configs()?)?;
    Ok((dev, cfg))
  }) {
    Ok((out_dev, out_cfg)) => {
      let out_rate = out_cfg.sample_rate();
      let mixer = Mixer::new(out_rate, deafened.clone()); // jitter buffer sized in device-rate samples
      match spawn_speaker(mixer.clone(), rnd_tx.clone(), out_cfg, out_dev) {
        Ok(stream) => (Some(stream), out_rate, mixer), // playback + render tee
        Err(e) => {
          eprintln!("output device build failed; joining without speaker: {e:?}");
          (None, out_rate, mixer)
        }
      }
    }
    Err(e) => {
      eprintln!("no usable output device; joining without speaker: {e:?}");
      let mixer = Mixer::new(TARGET_RATE, deafened.clone());
      (None, TARGET_RATE, mixer)
    }
  };

  // A dead input device is likewise non-fatal: we join muted-at-the-source
  // (nobody hears us) until the mic is fixed. With no capture stream the audio
  // processor simply never receives samples and publishes nothing.
  let (cpal_stream_input, in_rate) = match build_mic(cap_tx.clone(), input_name.as_deref()) {
    Ok((stream, rate)) => (Some(stream), rate),
    Err(e) => {
      eprintln!("input device build failed; joining without mic: {e:?}");
      (None, TARGET_RATE)
    }
  };
  let capture_signal_frames = Arc::new(std::sync::atomic::AtomicU64::new(0));
  spawn_audio_processor(
    cap_rx,
    rnd_rx,
    mic_track.clone(),
    in_rate,
    out_rate,
    voice_channel_id,
    conn,
    muted,
    deafened,
    gate_threshold,
    capture_signal_frames.clone(),
  )?; // APM + Opus, owns the middle

  let offer = client.create_offer(None).await?;
  let mut gather = client.gathering_complete_promise().await;
  client.set_local_description(offer).await?;
  let _ = gather.recv().await;
  let offer = client
    .local_description()
    .await
    .ok_or(anyhow::anyhow!("Client has no local description"))?;

  Ok(CallSetup {
    pc: client,
    offer,
    mic: cpal_stream_input,
    speaker: cpal_stream_output,
    mixer,
    in_rate,
    out_rate,
    cap_tx,
    rnd_tx,
    capture_signal_frames,
  })
}

#[allow(clippy::too_many_arguments)]
fn spawn_audio_processor(
  mut cap_rx: tokio::sync::mpsc::UnboundedReceiver<CaptureEvent>,
  mut rnd_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<f32>>,
  track: Arc<TrackLocalStaticSample>,
  in_rate: SampleRate,
  out_rate: SampleRate,
  voice_channel_id: Uuid,
  mut conn: WebRTCConnection,
  muted: Arc<std::sync::atomic::AtomicBool>,
  deafened: Arc<std::sync::atomic::AtomicBool>,
  gate_threshold: Arc<std::sync::atomic::AtomicU32>,
  capture_signal_frames: Arc<std::sync::atomic::AtomicU64>,
) -> anyhow::Result<()> {
  let mut cap_rs = Resampler::new(in_rate, TARGET_RATE)?;

  tokio::spawn(async move {
    let config = Config {
      echo_canceller: Some(EchoCanceller::default()),
      noise_suppression: Some(NoiseSuppression::default()),
      // adaptive digital AGC: normalizes quiet vs. loud mics toward a target
      // loudness post-AEC/NS. The noise cap (max_output_noise_level_dbfs, -50
      // default) keeps it from pumping the silent-room floor up to speech level.
      gain_controller2: Some(GainController2 {
        input_volume_controller: false,
        adaptive_digital: Some(AdaptiveDigital::default()),
        fixed_digital: FixedDigital::default(),
      }),
      ..Default::default()
    };
    let mut apm = AudioProcessing::builder()
      .config(config)
      .capture_config(sonora::StreamConfig::new(TARGET_RATE, 1))
      .render_config(sonora::StreamConfig::new(out_rate, 1))
      .build();

    let mut encoder = audiopus::coder::Encoder::new(
      audiopus::SampleRate::Hz48000,
      audiopus::Channels::Mono,
      audiopus::Application::Voip,
    )
    .expect("opus encoder");

    encoder
      .set_inband_fec(true)
      .expect("Failed to set Inband FEC");

    let mut cap_buf: Vec<f32> = Vec::new();
    let mut rnd_buf: Vec<f32> = Vec::new();
    let mut clean = vec![0f32; 480]; // one 10ms APM frame
    let mut rnd_sink = vec![0f32; 480];
    let mut pcm_20ms: Vec<f32> = Vec::with_capacity(960);
    let mut out = vec![0u8; 1500];
    // noise gate: closes (mutes the published frame) once the post-NS RMS stays
    // below the live-tunable threshold for GATE_HANGOVER frames. The threshold is
    // read each frame from a shared atomic (f32 bits); 0.0 disables the gate.
    let mut gate_hang = 0u32;
    const GATE_HANGOVER: u32 = 30; // ~300ms tail so word gaps don't clip

    // state above the loop:
    let mut speaking = false;
    let mut hang = 0u32;
    let mut last_sent = false;
    const VAD_THRESHOLD: f32 = 0.01; // tune by ear, post-NS so it can be low
    const VAD_HANGOVER: u32 = 25; // ~25 * 10ms = 250ms tail, avoids word-gap flicker

    loop {
      tokio::select! {
        Some(chunk) = rnd_rx.recv() => {
          rnd_buf.extend_from_slice(&chunk);
          while rnd_buf.len() >= 480 {
            let frame: Vec<f32> = rnd_buf.drain(..480).collect();
            if let Err(e) = apm.process_render_f32(&[&frame], &mut [&mut rnd_sink[..]]) {
              eprintln!("apm render error: {e:?}");
            }
          }
        }
        Some(event) = cap_rx.recv() => {
          let chunk = match event {
            // input device switched live: rebuild the resampler for the new
            // device's native rate and drop any half-converted tail.
            CaptureEvent::Rate(rate) => {
              match Resampler::new(rate, TARGET_RATE) {
                Ok(rs) => { cap_rs = rs; cap_buf.clear(); }
                Err(e) => eprintln!("cap resampler rebuild ({rate}->48k): {e:?}"),
              }
              continue;
            }
            CaptureEvent::Samples(chunk) => chunk,
          };
          // mic liveness: count this buffer only if it carries real signal.
          // Measured on the RAW device chunk (before APM/gain/gate/mute), so
          // neither noise suppression nor self-mute can zero it. A working mic —
          // even a quiet one — has a noise floor that peaks above this tiny
          // epsilon within a buffer; a dead/muted/disconnected device that still
          // fires callbacks delivers exact-zero (digital silence) samples and so
          // never advances the counter. The poller flags sustained no-advance.
          const SILENCE_EPS: f32 = 1e-4;
          if chunk.iter().any(|s| s.abs() > SILENCE_EPS) {
            capture_signal_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
          }
          let mut at48k = Vec::new();
          if let Err(e) = cap_rs.push(&chunk, &mut at48k) { eprintln!("cap resample: {e:?}"); continue; }
          cap_buf.extend_from_slice(&at48k);     // now genuinely 48k


          while cap_buf.len() >= 480 {
            let frame: Vec<f32> = cap_buf.drain(..480).collect();

            // to prevent internal err, we catch unwind and rebuild the apm:
            // thread 'tokio-rt-worker' (2506228) panicked at /home/user/.local/share/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sonora-aec3-0.1.0/src/adaptive_fir_filter.rs:136:22:
            // slice index starts at 13 but ends at 12
            // note: run with RUST_BACKTRACE=1 environment variable to display a backtrace
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
              apm.process_capture_f32(&[&frame], &mut [&mut clean[..]])
            }));

            // leave the 10ms frame to publish in `clean`; the raw branches fall
            // back to the un-processed frame (raw > silence). Gating + encode
            // happen uniformly below.
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => { eprintln!("apm capture err: {e:?}"); clean.copy_from_slice(&frame); }
                Err(_) => {
                    eprintln!("AEC3 PANICKED — rebuilding APM, passing frame raw");
                    let config = Config {
                      echo_canceller: Some(EchoCanceller::default()),
                      noise_suppression: Some(NoiseSuppression::default()),
                      // adaptive digital AGC: normalizes quiet vs. loud mics toward a target
      // loudness post-AEC/NS. The noise cap (max_output_noise_level_dbfs, -50
      // default) keeps it from pumping the silent-room floor up to speech level.
      gain_controller2: Some(GainController2 {
        input_volume_controller: false,
        adaptive_digital: Some(AdaptiveDigital::default()),
        fixed_digital: FixedDigital::default(),
      }),
                      ..Default::default()
                    };
                    apm = AudioProcessing::builder()
                      .config(config)
                      .capture_config(sonora::StreamConfig::new(TARGET_RATE, 1))
                      .render_config(sonora::StreamConfig::new(out_rate, 1))
                      .build();

                    clean.copy_from_slice(&frame);   // raw > silence
                }
            }


            // mic is gated while muted or deafened (deafen implies mute): we stop
            // publishing audio and report not-speaking regardless of input level.
            let gated = muted.load(std::sync::atomic::Ordering::Relaxed)
              || deafened.load(std::sync::atomic::Ordering::Relaxed);

            // after process_capture_f32 produces `clean`:
            let rms = (clean.iter().map(|s| s*s).sum::<f32>() / clean.len() as f32).sqrt();
            if gated { speaking = false; hang = 0; }
            else if rms > VAD_THRESHOLD { speaking = true; hang = VAD_HANGOVER; }
            else if hang > 0 { hang -= 1; } else { speaking = false; }

            if speaking != last_sent {
                conn.send(ClientVoice::Speaking{speaking, voice_channel_id});   // delta only, on transition
                last_sent = speaking;
            }

            // noise gate: keep the frame while above threshold (or within the
            // hangover tail), otherwise mute it. Read live so the settings slider
            // takes effect mid-call; 0.0 disables the gate entirely.
            let gate_t = f32::from_bits(gate_threshold.load(std::sync::atomic::Ordering::Relaxed));
            let gate_open = if gate_t <= 0.0 {
              true
            } else if rms > gate_t {
              gate_hang = GATE_HANGOVER;
              true
            } else if gate_hang > 0 {
              gate_hang -= 1;
              true
            } else {
              false
            };
            if !gate_open {
              clean.fill(0.0); // gate closed → publish silence
            }
            pcm_20ms.extend_from_slice(&clean);

            if pcm_20ms.len() >= 960 {
              // while gated we still drain the accumulator (so it can't grow
              // unbounded) but publish nothing to the track.
              if !gated {
                match encoder.encode_float(&pcm_20ms[..960], &mut out) {
                  Ok(n) => {
                    let _ = track.write_sample(&Sample {
                      data: bytes::Bytes::copy_from_slice(&out[..n]),
                      duration: std::time::Duration::from_millis(20),
                      ..Default::default()
                    }).await;
                  }
                  Err(e) => eprintln!("opus encode error {e}"),
                }
              }
              pcm_20ms.clear();
            }
          }
        }
        else => break,   // both channels closed → streams dropped → tear down
      }
    }
  });
  Ok(())
}
