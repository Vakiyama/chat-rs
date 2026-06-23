// chat-core: the non-UI client logic shared by the desktop iced client and the
// android native UI. Pure rust (grpc, webrtc voice, audio pipeline) with the
// platform couplings (token storage, config paths) behind injected traits.
uniffi::setup_scaffolding!();

pub mod client;
pub mod ffi;
pub mod mixer;
pub mod resampler;
pub mod rtc;
pub mod voice;
pub mod voice_settings;
