use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use axum::{Json, extract::State, response::IntoResponse};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use jwt_simple::{
  claims::{Claims, NoCustomClaims},
  prelude::{HS256Key, MACLike},
};
use tower_http::auth::AsyncAuthorizeRequest;
use utoipa_axum::{router::OpenApiRouter, routes};

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
  jwt_key: JWTKey,
}

#[derive(Clone)]
struct JWTKey {
  key: HS256Key,
}

impl Default for JWTKey {
  fn default() -> Self {
    let key_bytes: Bytes =
      hex::decode(std::env::var("JWT_SECRET").expect("Missing JWT secret env var"))
        .expect("Invalid key, decode failed")
        .into();

    let key: HS256Key = HS256Key::from_bytes(&key_bytes);

    Self { key }
  }
}

impl JWTKey {
  pub fn get(&self) -> &HS256Key {
    &self.key
  }
}

// ------------------------ Config ------------------------

const JWT_DURATION: Duration = Duration::from_hours(8);

// ------------------------ Router ------------------------
pub fn router() -> OpenApiRouter {
  OpenApiRouter::new()
    .routes(routes!(login_handler))
    .routes(routes!(verify_handler))
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
    path = "/login",
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
    path = "/verify",
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
    let claims = Claims::create(JWT_DURATION.into()).with_subject(identifier.to_string());
    let token = state
      .jwt_key
      .get()
      .authenticate(claims)
      .map_err(|_| VerifyError::Internal)?;

    Ok(Json(VerifyResponse {
      token,
      duration_milliseconds: JWT_DURATION.as_millis(),
    }))
  } else {
    Err(VerifyError::InvalidCode)
  }
}

// Middleware

#[derive(Clone)]
struct JWTAuthorized
where
  JWTKey: 'static,
{
  key: Arc<JWTKey>,
}

impl<B> AsyncAuthorizeRequest<B> for JWTAuthorized
where
  B: Send + Sync + 'static,
{
  type RequestBody = B;
  type ResponseBody = Full<Bytes>;
  type Future = BoxFuture<'static, Result<Request<B>, Response<Self::ResponseBody>>>;

  fn authorize(&mut self, mut request: Request<B>) -> Self::Future {
    let key = self.key.clone();

    Box::pin(async {
      if let Some(user_id) = check_auth(&request, key) {
        request.extensions_mut().insert(user_id);
        Ok(request)
      } else {
        let unauthorized_response = Response::builder()
          .status(StatusCode::UNAUTHORIZED)
          .body(Full::<Bytes>::default())
          .unwrap();

        Err(unauthorized_response)
      }
    })
  }
}

fn check_auth<B>(req: &Request<B>, key: Arc<JWTKey>) -> Option<Uuid> {
  let header = req.headers().get("Authorization")?.to_str().ok()?;
  let token = header.strip_prefix("Bearer ")?;

  let claims = key.get().verify_token::<NoCustomClaims>(token, None).ok()?;
  let subject = claims.subject?;

  Uuid::parse_str(&subject).ok()
}
