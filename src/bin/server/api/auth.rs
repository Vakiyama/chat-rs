use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
};

use axum::{Json, Router, extract::State, routing::post};
use dotenvy::dotenv;

use crate::library::resend;
use uuid::Uuid;

#[derive(Default, Clone)]
struct PendingTokenStore {
  pending: HashMap<Uuid, String>,
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
    .with_state(RouterState::default())
}

#[derive(utoipa::ToSchema, serde::Serialize)]
struct LoginResponse {
  identifier: Uuid,
}

#[derive(serde::Deserialize)]
pub struct LoginParams {
  pub email: String,
}

// get the email from the qparam, send to resend, create a stateful "email:code -> JWT" pending
// resolution store
#[utoipa::path(
    post,
    path = "/login",
    params(("email" = String, Query, description = "Email to send verification code to")), 
    responses(
      (status = 200, body = LoginResponse),
      (status = 422, description = "Missing or invalid body params"),
      (status = 400, description = "Resend API error"),
    )
 )
]
#[axum::debug_handler]
async fn login_handler(
  State(state): State<RouterState>,
  Json(payload): Json<LoginParams>,
) -> Result<Json<LoginResponse>, resend::Error> {
  let identifier = Uuid::new_v4();
  let code = resend::send_auth_email(payload.email, state.resend).await?;

  let mut state_lock = state.store.lock().unwrap();
  state_lock.pending.insert(identifier, code);

  Ok(Json(LoginResponse { identifier }))
}
