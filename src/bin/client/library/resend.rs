use dotenvy::dotenv;
use rand::RngExt;
use rand::distr::Alphanumeric;
use resend_rs::types::CreateEmailBaseOptions;
use resend_rs::{Resend, Result};

const FROM: &str = "ChatRS <chatrs@resend.dev>";
const SUBJECT: &str = "Login Code";

/// sends an authentication email to the address
/// returns the auth code for confirmation that the user recieved it
pub async fn send_auth_email(to: &str, resend: &Resend) -> Result<String, resend_rs::Error> {
  let _env = dotenv().unwrap();

  let mut rng = rand::rng();
  // need to generate a 5 digit alphanumeric code
  let chars: String = (0..5).map(|_| rng.sample(Alphanumeric) as char).collect();

  let email = CreateEmailBaseOptions::new(FROM, [to], SUBJECT)
    .with_html(&format!("Your login code is: {}", chars));

  let _result = resend.emails.send(email).await?;

  Ok(chars)
}
