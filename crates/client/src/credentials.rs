//! Desktop refresh-token persistence: the OS keyring (Secret Service / macOS
//! Keychain / Windows Credential Manager) via the `keyring` crate. The
//! [`CredentialStore`] trait and the grpc client live in `chat_core::client`;
//! android provides a keystore-backed impl instead.

use chat_core::client::CredentialStore;
use keyring::Entry;

pub struct KeyringCredentialStore {
  service: String,
  user: String,
}

impl KeyringCredentialStore {
  pub fn new(service: String, user: String) -> Self {
    Self { service, user }
  }
}

impl CredentialStore for KeyringCredentialStore {
  fn store_refresh_token(&self, token: &str) -> anyhow::Result<()> {
    Entry::new(&self.service, &self.user)?.set_password(token)?;
    Ok(())
  }

  fn load_refresh_token(&self) -> anyhow::Result<String> {
    Ok(Entry::new(&self.service, &self.user)?.get_password()?)
  }

  fn clear_refresh_token(&self) -> anyhow::Result<()> {
    Entry::new(&self.service, &self.user)?.delete_credential()?;
    Ok(())
  }
}
