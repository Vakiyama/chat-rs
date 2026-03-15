use std::str::FromStr;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use dotenvy::dotenv;
use rand::RngExt;
use rand::distr::Alphanumeric;
use resend_rs::types::CreateEmailBaseOptions;
use resend_rs::{Resend, Result};

const FROM: &str = "ChatRS <chatrs@resend.dev>";
const SUBJECT: &str = "Login Code";

#[derive(Debug)]
pub enum Error {
  Api(resend_rs::Error),
  EmailValidation(email_address::Error),
}

impl IntoResponse for Error {
  fn into_response(self) -> axum::response::Response {
    match self {
      Error::Api(_) => (StatusCode::BAD_REQUEST, "Resend API Error").into_response(),
      Error::EmailValidation(_error) => {
        (StatusCode::UNPROCESSABLE_ENTITY, "Invalid Email").into_response()
      }
    }
  }
}

/// sends an authentication email to the address
/// returns the auth code for confirmation that the user received it
pub async fn send_auth_email(to: String, resend: Arc<Resend>) -> Result<String, Error> {
  let _env = dotenv().unwrap();

  let _valid = email_address::EmailAddress::from_str(&to).map_err(Error::EmailValidation)?;

  let chars = {
    let mut rng = rand::rng();

    let chars: String = (0..5)
      .map(|_| { rng.sample(Alphanumeric) as char }.to_ascii_uppercase())
      .collect();
    chars
  };

  let email = CreateEmailBaseOptions::new(FROM, [to], SUBJECT)
    .with_html(&format!("Your login code is: {}", chars));

  let _result = resend.emails.send(email).await.map_err(Error::Api)?;

  Ok(chars)
}
