//! The uniffi facade: a thin, kotlin-facing wrapper over the core. The rest of
//! the crate speaks rust types (tonic/webrtc/cpal) that don't cross uniffi, so
//! the boundary lives here in simple records, enums, and callback interfaces.
//!
//! Thin on purpose (CHA-34): this proves the uniffi toolchain end to end
//! (callback interfaces both ways, an enum, an error, and an async export). The
//! full surface (auth, stream, voice handle) layers onto these mechanisms with
//! the android app.

use crate::client::CredentialStore;
use std::sync::Arc;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
  #[error("chat-core not initialized")]
  NotInitialized,
  #[error("network error: {message}")]
  Network { message: String },
}

/// A status update pushed from rust to the kotlin UI. Mirrors the kinds of voice
/// events the actor emits; the full mapping lands with the voice surface.
#[derive(uniffi::Enum)]
pub enum CoreEvent {
  VoiceJoined,
  MicActivity { receiving: bool },
  MediaHealthy { healthy: bool },
}

/// Refresh-token persistence implemented on the kotlin side (android keystore).
/// Bridged to the core [`CredentialStore`] so the grpc client uses it unchanged.
#[uniffi::export(callback_interface)]
pub trait Credentials: Send + Sync {
  fn store_refresh_token(&self, token: String);
  fn load_refresh_token(&self) -> Option<String>;
  fn clear_refresh_token(&self);
}

/// Sink for [`CoreEvent`]s, implemented on the kotlin side.
#[uniffi::export(callback_interface)]
pub trait EventListener: Send + Sync {
  fn on_event(&self, event: CoreEvent);
}

struct CredentialsBridge(Box<dyn Credentials>);

impl CredentialStore for CredentialsBridge {
  fn store_refresh_token(&self, token: &str) -> anyhow::Result<()> {
    self.0.store_refresh_token(token.to_string());
    Ok(())
  }
  fn load_refresh_token(&self) -> anyhow::Result<String> {
    self
      .0
      .load_refresh_token()
      .ok_or_else(|| anyhow::anyhow!("no refresh token stored"))
  }
  fn clear_refresh_token(&self) -> anyhow::Result<()> {
    self.0.clear_refresh_token();
    Ok(())
  }
}

/// Wire the grpc client to the android-provided config + keystore. Call once at
/// app startup before any other call.
#[uniffi::export]
pub fn init(server_url: String, credentials: Box<dyn Credentials>) {
  crate::client::init(server_url, Arc::new(CredentialsBridge(credentials)));
}

/// The single internal sample rate the audio pipeline runs at (48 kHz).
#[uniffi::export]
pub fn target_sample_rate() -> u32 {
  crate::rtc::TARGET_RATE
}

/// Async + event-push smoke test: proves the tokio-backed async bridge and the
/// rust to kotlin callback both generate and run. Yields, then pushes one event.
#[uniffi::export(async_runtime = "tokio")]
pub async fn ffi_async_smoke(listener: Box<dyn EventListener>) -> Result<(), CoreError> {
  tokio::task::yield_now().await;
  listener.on_event(CoreEvent::MicActivity { receiving: true });
  Ok(())
}
