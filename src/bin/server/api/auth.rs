use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
use bytes::Bytes;
use dotenvy::dotenv;
use jwt_simple::{
  claims::Claims,
  prelude::{HS256Key, MACLike},
};

use crate::library::resend;
use uuid::Uuid;

#[derive(Default, Clone)]
struct EmailCodePair {
  code: String,
  email: String,
}

#[derive(Default, Clone)]
struct PendingTokenStore {
  pending: HashMap<Uuid, EmailCodePair>,
}

#[derive(Default, Clone)]
struct RouterState {
  store: Arc<Mutex<PendingTokenStore>>,
  resend: Arc<resend_rs::Resend>,
}

pub fn router() -> Router {
  let _env = dotenv();

  Router::new()
    .route("/login", post(login_handler))
    .route("/verify", post(verify_handler))
    .with_state(RouterState::default())
}

#[derive(utoipa::ToSchema, serde::Serialize)]
struct LoginResponse {
  identifier: Uuid,
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
struct LoginBody {
  email: String,
}

#[utoipa::path(
    post,
    path = "/api/auth/login",
    request_body(content = LoginBody, description = "Email to attempt login with", content_type = "application/json"),
    // params(("email" = String, Query, description = "Email to send verification code to")), 
    responses(
      (status = 200, body = LoginResponse),
      (status = 422, description = "Missing or invalid body params"),
      (status = 400, description = "Resend API error"),
    )
 )
]

async fn login_handler(
  State(state): State<RouterState>,
  Json(payload): Json<LoginBody>,
) -> Result<Json<LoginResponse>, resend::Error> {
  let identifier = Uuid::new_v4();
  let code = resend::send_auth_email(&payload.email, state.resend).await?;

  let mut state_lock = state.store.lock().unwrap();
  state_lock.pending.insert(
    identifier,
    EmailCodePair {
      code,
      email: payload.email,
    },
  );

  Ok(Json(LoginResponse { identifier }))
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
struct VerifyBody {
  identifier: Uuid,
  email: String,
  code: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
struct VerifyResponse {
  token: String,
  duration_milliseconds: u128,
}

enum VerifyError {
  InvalidCode,
  UnknownIdentifier,
  Internal,
}

impl IntoResponse for VerifyError {
  fn into_response(self) -> axum::response::Response {
    todo!()
  }
}

#[utoipa::path(
    post,
    path = "/api/auth/verify",
    request_body(content = VerifyBody, description = "Verification details for token exchange", content_type = "application/json"),
    responses(
      (status = 200, body = VerifyResponse),
      (status = 422, description = "Missing or invalid body params"),
      (status = 400, description = "Invalid identifier"),
      (status = 500, description = "Internal server error"),
     )
   )
]
async fn verify_handler(
  State(state): State<RouterState>,
  Json(payload): Json<VerifyBody>,
) -> Result<Json<VerifyResponse>, VerifyError> {
  let VerifyBody {
    identifier,
    email: incoming_email,
    code: code_attempt,
  } = payload;

  let store = state.store.lock().unwrap();

  let EmailCodePair { code, email } = store
    .pending
    .get(&identifier)
    .map(Ok)
    .unwrap_or(Err(VerifyError::UnknownIdentifier))?;

  if &code_attempt == code && email == &incoming_email {
    let key_bytes: Bytes =
      hex::decode(std::env::var("JWT_SECRET").expect("Missing JWT secret env var"))
        .expect("Invalid key, decode failed")
        .into();

    let key: HS256Key = HS256Key::from_bytes(&key_bytes);

    let duration = Duration::from_hours(8);
    let claims = Claims::create(duration.into());
    let token = key
      .authenticate(claims)
      .map_err(|_| VerifyError::Internal)?;

    Ok(Json(VerifyResponse {
      token,
      duration_milliseconds: duration.as_millis(),
    }))
  } else {
    Err(VerifyError::InvalidCode)
  }
}
