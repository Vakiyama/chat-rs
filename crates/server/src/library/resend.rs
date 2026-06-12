use std::str::FromStr;
use std::sync::Arc;

use rand::RngExt;
use rand::distr::Alphanumeric;
use resend_rs::types::CreateEmailBaseOptions;
use resend_rs::{Resend, Result};
use serde::Deserialize;
use tonic::Status;

const FROM: &str = "login@vitorakiyama.com";
const SUBJECT: &str = "Login Code";

#[derive(Debug, Deserialize, Clone)]
pub enum Error {
  Api(String),
  EmailValidation(String),
}

impl From<Error> for tonic::Status {
  fn from(error: Error) -> tonic::Status {
    match error {
      Error::Api(message) => Status::internal("An unknown error ocurred."),
      Error::EmailValidation(_error) => Status::invalid_argument("Invalid email."),
    }
  }
}

/// sends an authentication email to the address
/// returns the auth code for confirmation that the user received it
pub async fn send_auth_email(to: &String, resend: Arc<Resend>) -> Result<String, Error> {
  let _valid =
    email_address::EmailAddress::from_str(to).map_err(|e| Error::EmailValidation(e.to_string()))?;

  let chars = {
    let mut rng = rand::rng();

    let chars: String = (0..6)
      .map(|_| { rng.sample(Alphanumeric) as char }.to_ascii_uppercase())
      .collect();
    chars
  };

  let email = CreateEmailBaseOptions::new(FROM, [to], SUBJECT)
    .with_html(&format!("Your login code is: {}", chars));

  let _result = resend
    .emails
    .send(email)
    .await
    .map_err(|e| Error::Api(e.to_string()))?;

  Ok(chars)
}
