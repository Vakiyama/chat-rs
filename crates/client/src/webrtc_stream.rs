use std::sync::Arc;

use chat_shared::convert::stream::proto::ClientVoiceMessage;
use chat_shared::convert::{IntoProto, TryIntoDomain};
use chat_shared::domain::stream::{ClientVoice, ServerVoice};
use cpal::StreamConfig;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::channel::mpsc;
use futures::stream::StreamExt;
use iced::task::{Never, Sipper, sipper};
use iced::{Task, futures};
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

use crate::client;
use crate::mixer::Mixer;
use crate::model::{Model, Stream};

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
        Err(e) => {
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

pub fn handle(model: &mut Model, msg: Box<ServerVoice>) -> Task<crate::Message> {
  let Some(client) = &model.webrtc_client else {
    eprintln!("No webrtc client found when receiving server offer/answer");
    return Task::none();
  };

  let client = client.clone();
  let webrtc_stream = model.webrtc_stream.clone();

  match *msg {
    ServerVoice::Offer(rtcsession_description) => Task::future(async move {
      let Stream::Connected(mut connection) = webrtc_stream else {
        eprintln!("No webrtc connection found when receiving server offer");
        return crate::Message::None;
      };

      let _ = {
        async || -> anyhow::Result<()> {
          client
            .set_remote_description(rtcsession_description)
            .await?;
          let reneg_answer = client.create_answer(None).await?;
          client.set_local_description(reneg_answer).await?;

          let answer = client
            .local_description()
            .await
            .ok_or(anyhow::anyhow!("No local description"))?;

          connection.send(ClientVoice::Answer(answer));
          Ok(())
        }
      }()
      .await
      .map_err(|e| eprintln!("Error handling server offer: {e:?}"));

      crate::Message::None
    }),
    ServerVoice::Answer(rtcsession_description) => Task::future(async move {
      for line in rtcsession_description
        .sdp
        .lines()
        .filter(|l| l.contains("candidate"))
      {
        println!("server candidate: {line}");
      }
      let _ = client
        .set_remote_description(rtcsession_description)
        .await
        .map_err(|e| eprintln!("Error setting remote desc from server asnwer: {e:?}"));

      crate::Message::None
    }),
  }
}

pub fn start() -> Task<crate::Message> {
  Task::future(async move {
    let client = setup_client()
      .await
      .map_err(|e| eprintln!("Error setting up initial client! {e:?}"));

    match client {
      Ok((client, offer, mic_stream, output_stream)) => crate::Message::WebRTCClientCreated(
        client.into(),
        offer.into(),
        mic_stream.into(),
        output_stream.into(),
      ),
      Err(_) => crate::Message::None,
    }
  })
}

async fn spawn_mic(track: Arc<TrackLocalStaticSample>) -> anyhow::Result<cpal::Stream> {
  let host = cpal::default_host();
  let device = host
    .default_input_device()
    .ok_or_else(|| anyhow::anyhow!("no input device"))?;
  let config = device.default_input_config()?;
  let sample_rate: u32 = config.sample_rate();
  let sample_format = config.sample_format();

  assert!(sample_rate == 48000);
  assert!(sample_format == cpal::SampleFormat::F32);
  let channels = config.channels();

  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
  let stream_config: StreamConfig = config.into();

  let stream = device.build_input_stream(
    stream_config,
    move |data: &[f32], _| {
      let mono: Vec<f32> = data
        .chunks(channels.into())
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

      let _ = tx.send(mono);
    },
    |err| eprintln!("cpal input error: {err}"),
    None,
  )?;

  stream.play()?;

  // encoder task accumulates 20ms frames, encodes and writes
  tokio::spawn(async move {
    // assuming 48K sample rate
    let frame_size = sample_rate as usize / 50;
    let encoder = audiopus::coder::Encoder::new(
      audiopus::SampleRate::Hz48000,
      audiopus::Channels::Mono,
      audiopus::Application::Voip,
    )
    .expect("opus encoder");

    let mut pcm: Vec<f32> = Vec::with_capacity(frame_size * 2);
    let mut out = vec![0u8; 1500];
    while let Some(chunk) = rx.recv().await {
      pcm.extend_from_slice(&chunk);
      while pcm.len() >= frame_size {
        let frame: Vec<f32> = pcm.drain(..frame_size).collect();
        match encoder.encode_float(&frame, &mut out) {
          Ok(n) => {
            let _ = track
              .write_sample(&Sample {
                data: bytes::Bytes::copy_from_slice(&out[..n]),
                duration: std::time::Duration::from_millis(20),
                ..Default::default()
              })
              .await;
          }
          Err(e) => eprintln!("opus encode error {e}"),
        }
      }
    }
  });

  Ok(stream) // when this stream drops, the mic stops.
}

fn spawn_speaker(mixer: Mixer) -> anyhow::Result<cpal::Stream> {
  let host = cpal::default_host();
  let device = host
    .default_output_device()
    .ok_or_else(|| anyhow::anyhow!("no output device"))?;
  let config = device.default_output_config()?;
  let sample_rate: u32 = config.sample_rate();
  let sample_format = config.sample_format();

  assert!(sample_rate == 48000);
  assert!(sample_format == cpal::SampleFormat::F32);

  // let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
  let stream_config: StreamConfig = config.into();
  let channels = stream_config.channels as usize;

  let stream = device.build_output_stream(
    stream_config,
    move |data: &mut [f32], _| {
      mixer.mix_into(data, channels);
    },
    |err| eprintln!("cpal input error: {err}"),
    None,
  )?;

  stream.play()?;

  Ok(stream) // when this stream drops, the output stops.
}

pub async fn setup_client() -> anyhow::Result<(
  RTCPeerConnection,
  RTCSessionDescription,
  cpal::Stream,
  cpal::Stream,
)> {
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

  let mixer = Mixer::default();

  let cpal_stream_input = spawn_mic(mic_track).await?;
  let cpal_stream_output = spawn_speaker(mixer.clone())?;

  let mixer_for_tracks = mixer.clone();
  client.on_track(Box::new(move |track, _, _| {
    let mixer = mixer_for_tracks.clone();
    Box::pin(async move {
      println!("on track for client fired");
      let src = track.ssrc();
      let mut decoder =
        audiopus::coder::Decoder::new(audiopus::SampleRate::Hz48000, audiopus::Channels::Mono)
          .expect("opus decoder");
      let mut pcm = vec![0f32; 960]; // one 20ms frame at 48k
      while let Ok((pkt, _)) = track.read_rtp().await {
        if pkt.payload.is_empty() {
          continue;
        } // e.g. padding
        match decoder.decode_float(Some(&pkt.payload[..]), &mut pcm, false) {
          Ok(n) => mixer.push(src, &pcm[..n]),
          Err(e) => eprintln!("opus decode error: {e}"),
        }
      }
      mixer.remove(src); // track ended (peer left)
    })
  }));

  client.on_peer_connection_state_change(Box::new(move |new_state| {
    println!("{new_state:?}");

    Box::pin(async {})
  }));

  let offer = client.create_offer(None).await?;
  let mut gather = client.gathering_complete_promise().await;
  client.set_local_description(offer).await?;
  let _ = gather.recv().await;
  let offer = client.local_description().await.unwrap();

  // connection.send(ClientVoice::Offer(offer));

  Ok((client, offer, cpal_stream_input, cpal_stream_output))
}
