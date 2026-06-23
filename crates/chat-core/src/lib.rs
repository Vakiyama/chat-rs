// chat-core: the non-UI client logic shared by the desktop iced client and the
// android native UI. Pure rust (grpc, webrtc voice, audio pipeline) with the
// platform couplings (token storage, config paths) behind injected traits.
pub mod mixer;
pub mod resampler;
pub mod voice_settings;
