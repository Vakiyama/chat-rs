use rand::RngExt;

/// sends an authentication email to the address
/// returns the auth code for confirmation that the user recieved it
pub async fn send_auth_email(to: &str) {
  // need to generate a 5 digit alphanumeric code
}
