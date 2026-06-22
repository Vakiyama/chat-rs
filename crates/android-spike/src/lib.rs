// CHA-33 android cross-compile spike. The deliverable is whether the dependency
// graph (opus, ring, sonora, webrtc, tonic) builds and links for
// aarch64-linux-android under the ndk, so this code only needs to exist.
pub fn spike() -> u32 {
  48_000
}
