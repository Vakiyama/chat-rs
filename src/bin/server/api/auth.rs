use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use axum::{Extension, Json, extract::State, response::IntoResponse};
use bytes::Bytes;
use futures_util::{TryFutureExt, future::BoxFuture};
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use jwt_simple::{
  claims::{Claims, NoCustomClaims},
  prelude::{HS256Key, MACLike},
};
use tower_http::auth::{AsyncAuthorizeRequest, AsyncRequireAuthorizationLayer};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::library::resend;
use uuid::Uuid;

#[derive(Default, Clone)]
struct EmailCodePair {
  code: String,
  email: String,
}

#[derive(Default, Clone)]
struct InMemoryStore {
  pending: HashMap<Uuid, EmailCodePair>,
}

impl CodeStore for InMemoryStore {
  async fn get_email_code_pair(&self, uuid: &Uuid) -> Option<&EmailCodePair> {
    self.pending.get(uuid)
  }

  async fn insert(&mut self, uuid: Uuid, email_code: EmailCodePair) -> Option<EmailCodePair> {
    self.pending.insert(uuid, email_code)
  }
}

trait CodeStore {
  /// given an uuid, gets the pending
  async fn get_email_code_pair(&self, uuid: &Uuid) -> Option<&EmailCodePair>;

  async fn insert(&mut self, uuid: Uuid, email_code: EmailCodePair) -> Option<EmailCodePair>;
}

#[derive(Default, Clone)]
struct RouterState<Store> {
  store: Store,
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

const JWT_ACCESS_DURATION: Duration = Duration::from_hours(8);
const JWT_REFRESH_DURATION: Duration = Duration::from_hours(24 * 30);

// ------------------------ Router ------------------------
pub fn router() -> OpenApiRouter {
  let key_bytes: Bytes = hex::decode(std::env::var("JWT_KEY").expect("Missing JWT_KEY env var"))
    .expect("Invalid JWT_KEY, decode failed")
    .into();

  let key = HS256Key::from_bytes(&key_bytes);

  OpenApiRouter::new()
    .routes(routes!(login_handler))
    .routes(routes!(verify_handler))
    .layer(
      tower::ServiceBuilder::new().layer(AsyncRequireAuthorizationLayer::new(JWTAuthorized {
        key: JWTKey { key: key.into() }.into(),
      })),
    )
    .routes(routes!(refresh_handler))
    .with_state(RouterState::<InMemoryStore>::default())
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

async fn login_handler<Store>(
  State(mut state): State<RouterState<Store>>,
  Json(payload): Json<LoginBody>,
) -> Result<Json<LoginResponse>, resend::Error>
where
  Store: CodeStore + Send + Sync + 'static,
{
  let identifier = Uuid::new_v4();
  let code = resend::send_auth_email(&payload.email, state.resend).await?;

  state
    .store
    .insert(
      identifier,
      EmailCodePair {
        code,
        email: payload.email,
      },
    )
    .await;

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
  access_token: String,
  refresh_token: String,
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
async fn verify_handler<Store>(
  State(state): State<RouterState<Store>>,
  Json(payload): Json<VerifyBody>,
) -> Result<Json<VerifyResponse>, VerifyError>
where
  Store: CodeStore + Send + Sync + 'static,
{
  let VerifyBody {
    identifier,
    email: incoming_email,
    code: code_attempt,
  } = payload;

  let EmailCodePair { code, email } = state
    .store
    .get_email_code_pair(&identifier)
    .await
    .map(Ok)
    .unwrap_or(Err(VerifyError::UnknownIdentifier))?;

  let TokenPair {
    access_token,
    refresh_token,
  } = generate_tokens(identifier, state.jwt_key.get())
    .map_err(|_| VerifyError::Internal)
    .await?;

  if &code_attempt == code && email == &incoming_email {
    Ok(Json(VerifyResponse {
      access_token,
      refresh_token,
      duration_milliseconds: JWT_ACCESS_DURATION.as_millis(),
    }))
  } else {
    Err(VerifyError::InvalidCode)
  }
}

struct TokenPair {
  access_token: String,
  refresh_token: String,
}

/// expects the user id as identifier
async fn generate_tokens(identifier: Uuid, key: &HS256Key) -> Result<TokenPair, anyhow::Error> {
  let claims = Claims::create(JWT_ACCESS_DURATION.into()).with_subject(identifier.to_string());
  let access_token = key
    .authenticate(claims)
    .map_err(|_| anyhow::anyhow!("Internal"))?;

  let claims = Claims::create(JWT_REFRESH_DURATION.into()).with_subject(identifier.to_string());
  let refresh_token = key
    .authenticate(claims)
    .map_err(|_| anyhow::anyhow!("Internal"))?;

  Ok(TokenPair {
    access_token,
    refresh_token,
  })
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
struct RefreshBody {
  refresh_token: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
struct RefreshResponse {
  access_token: String,
  refresh_token: String,
}

pub enum RefreshError {
  InvalidCode,
  UnknownIdentifier,
  Internal,
}

impl IntoResponse for RefreshError {
  fn into_response(self) -> axum::response::Response {
    todo!()
  }
}

/// given a valid refresh token, a new token is issued
#[utoipa::path(
    post,
    path = "/refresh",
    request_body(content = RefreshBody, description = "Verification details for token exchange", content_type = "application/json"),
    responses(
      (status = 200, body = RefreshResponse),
      (status = 400, description = "Missing bearer token"),
      (status = 401, description = "Unauthorized"),
      (status = 500, description = "Internal server error"),
     )
   )
]
// hemingway: refresh handler should return a refresh body, generate all new tokens, replace the
// old refresh token in the store and give the both new tokens back
// #[axum::debug_handler]
async fn refresh_handler<Store>(
  Extension(user_id): Extension<Uuid>,
  State(mut state): State<RouterState<Store>>,
  Json(body): Json<RefreshBody>,
) -> Result<Json<RefreshResponse>, RefreshError>
where
  Store: CodeStore + Send + Sync + 'static,
{
  todo!()
}

// Middleware

#[derive(Clone)]
struct JWTAuthorized
where
  JWTKey: 'static,
{
  key: Arc<JWTKey>,
}

impl AsyncAuthorizeRequest<axum::body::Body> for JWTAuthorized {
  type RequestBody = axum::body::Body;
  type ResponseBody = axum::body::Body;
  type Future = BoxFuture<'static, Result<Request<axum::body::Body>, Response<Self::ResponseBody>>>;

  fn authorize(&mut self, mut request: Request<axum::body::Body>) -> Self::Future {
    let key = self.key.clone();

    Box::pin(async {
      if let Some(user_id) = check_auth(&request, key) {
        request.extensions_mut().insert(user_id);
        Ok(request)
      } else {
        let unauthorized_response = Response::builder()
          .status(StatusCode::UNAUTHORIZED)
          .body(axum::body::Body::default())
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
