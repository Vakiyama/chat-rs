use chat_rs::config::CONFIG;
use chat_rs::shared::convert::auth::proto::{RegisterRequest, RegisterResponse};
use chat_rs::shared::domain::auth::RegisterReturn;
use chat_rs::shared::{
  convert::{
    IntoProto, IntoStatus, TryIntoDomain,
    auth::proto::{
      LoginRequest, LoginResponse, RefreshRequest, RefreshResponse, VerifyRequest, VerifyResponse,
      auth_service_server::AuthService,
    },
  },
  domain::auth::{
    LoginReturn, RefreshCommand, RefreshError, RefreshReturn, Token, TokenPair, UserId,
    VerifyCommand, VerifyError, VerifyReturn,
  },
};
use http::Request;
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, ModelTrait, QueryFilter};
use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};
use tokio_rate_limit::RateLimiter;
use tonic::body::Body;
use tonic::transport::server::TcpConnectInfo;
use tonic_middleware::RequestInterceptor;

use crate::entities;
use crate::library::{database, resend};
use jwt_simple::{
  claims::{Claims, DEFAULT_TIME_TOLERANCE_SECS, NoCustomClaims},
  prelude::{HS256Key, MACLike},
};
use uuid::Uuid;

#[derive(Clone)]
pub enum VerifyType {
  Login {
    email: String,
    user_id: Uuid,
    username: String,
  },
  Register {
    email: String,
    username: String,
  },
}

#[derive(Clone)]
pub struct EmailCodePair {
  pub code: String,
  pub info: VerifyType,
  pub verify_attempts: i8,
}

#[derive(Clone)]
pub struct JWTKey {
  pub key: HS256Key,
}

impl Default for JWTKey {
  fn default() -> Self {
    let key_bytes: bytes::Bytes = hex::decode(&CONFIG.auth.jwt_key_hex)
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

type JWTId = Uuid;

// ------------------------- Stores -------------------------

#[derive(Default, Clone)]
pub struct InMemoryCodeStore {
  pending: Arc<Mutex<HashMap<JWTId, EmailCodePair>>>,
}

impl CodeStore for InMemoryCodeStore {
  async fn get_email_code_pair(&self, uuid: &Uuid) -> Option<EmailCodePair> {
    let mut pending = self.pending.lock().unwrap();

    let code_pair = pending.get_mut(uuid);

    if let Some(pending_code) = code_pair {
      pending_code.verify_attempts += 1;

      if pending_code.verify_attempts >= CONFIG.auth.max_verify_code_attempts {
        pending.remove(uuid);
        return None;
      }
    }

    pending.get(uuid).cloned()
  }

  async fn insert(&mut self, uuid: JWTId, email_code: EmailCodePair) -> Option<EmailCodePair> {
    let store = self.pending.clone();

    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_mins(15)).await;
      store.lock().unwrap().remove_entry(&uuid);
    });

    self.pending.lock().unwrap().insert(uuid, email_code)
  }

  async fn delete(&mut self, uuid: &Uuid) {
    self.pending.lock().unwrap().remove_entry(uuid);
  }
}

pub trait CodeStore {
  /// given an uuid, gets the pending
  async fn get_email_code_pair(&self, uuid: &Uuid) -> Option<EmailCodePair>;

  async fn insert(&mut self, uuid: Uuid, email_code: EmailCodePair) -> Option<EmailCodePair>;

  async fn delete(&mut self, uuid: &Uuid);
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshTokenStoreError {
  #[error("invalid token")]
  InvalidToken,
  #[error("token rotated out")]
  TokenRotatedOut,
  #[error("token expired")]
  TokenExpired,
  #[error("no user with such token")]
  NoUserWithSuchToken,
  #[error(transparent)]
  Internal(#[from] sea_orm::DbErr),
}

impl From<RefreshTokenStoreError> for tonic::Status {
  fn from(val: RefreshTokenStoreError) -> Self {
    match val {
      RefreshTokenStoreError::InvalidToken
      | RefreshTokenStoreError::TokenRotatedOut
      | RefreshTokenStoreError::TokenExpired => tonic::Status::unauthenticated("Invalid token."),
      RefreshTokenStoreError::NoUserWithSuchToken => tonic::Status::not_found("User not found."),
      RefreshTokenStoreError::Internal(_db_err) => {
        tonic::Status::internal("Unknown error occurred.")
      }
    }
  }
}

pub trait RefreshTokenStore {
  #[allow(dead_code)] // used in tests
  fn new(time_tolerance: jwt_simple::prelude::Duration) -> Self;

  async fn has(&self, token: &Token) -> Result<UserId, RefreshTokenStoreError>;

  async fn rotate(&self, old: &Token, new: Token) -> Result<(), RefreshTokenStoreError>;

  async fn insert(&self, token: Token) -> Result<UserId, RefreshTokenStoreError>;

  async fn remove(&self, token: &Token) -> Result<Option<()>, RefreshTokenStoreError>;

  async fn remove_all_for_user(&self, user_id: &UserId) -> Result<(), RefreshTokenStoreError>;
}

#[derive(Clone)]
pub struct DbTokenStore {
  key: JWTKey,
  time_tolerance: jwt_simple::prelude::Duration,
}

impl Default for DbTokenStore {
  fn default() -> Self {
    Self {
      key: Default::default(),
      time_tolerance: jwt_simple::prelude::Duration::from_mins(1),
    }
  }
}

impl RefreshTokenStore for DbTokenStore {
  fn new(time_tolerance: jwt_simple::prelude::Duration) -> Self {
    Self {
      time_tolerance,
      key: Default::default(),
    }
  }

  async fn has(&self, token: &Token) -> Result<UserId, RefreshTokenStoreError> {
    let token_uuid = get_uuid_from_verify_token(self.key.get(), token, self.time_tolerance);

    let jti = get_jti_for_token(self.key.get(), token)
      .ok_or_else(|| RefreshTokenStoreError::InvalidToken)?;

    let db = database::get().await;

    let has_token = {
      entities::refresh_token::Entity::find()
        .filter(entities::refresh_token::COLUMN.refresh_token_jti.eq(jti))
        .one(db)
        .await?
        .is_some()
    };

    match has_token {
      true => match token_uuid {
        Some(user_id) => Ok(user_id),
        None => {
          self.remove(token).await?;
          Err(RefreshTokenStoreError::TokenExpired)
        }
      },
      false => match token_uuid {
        Some(id) => {
          self.remove_all_for_user(&id).await?;
          Err(RefreshTokenStoreError::TokenRotatedOut)
        }
        None => Err(RefreshTokenStoreError::InvalidToken),
      },
    }
  }

  async fn rotate(&self, old: &Token, new: Token) -> Result<(), RefreshTokenStoreError> {
    self.remove(old).await?;
    self.insert(new).await?;

    Ok(())
  }

  async fn insert(&self, token: Token) -> Result<UserId, RefreshTokenStoreError> {
    let user_id = get_uuid_from_verify_token(self.key.get(), &token, self.time_tolerance)
      .map(Ok)
      .unwrap_or(Err(RefreshTokenStoreError::InvalidToken))?;

    let jti = get_jti_for_token(self.key.get(), &token)
      .ok_or_else(|| RefreshTokenStoreError::InvalidToken)?;

    let db = database::get().await;

    entities::refresh_token::Entity::insert(
      entities::refresh_token::Model {
        id: uuid::Uuid::new_v4(),
        refresh_token_jti: jti,
        user_id,
      }
      .into_active_model(),
    )
    .exec(db)
    .await?;

    Ok(user_id)
  }

  async fn remove(&self, token: &Token) -> Result<Option<()>, RefreshTokenStoreError> {
    let db = database::get().await;
    let jti = get_jti_for_token(self.key.get(), token)
      .ok_or_else(|| RefreshTokenStoreError::InvalidToken)?;

    let token_in_db = entities::refresh_token::Entity::find()
      .filter(
        entities::refresh_token::Entity::COLUMN
          .refresh_token_jti
          .eq(jti),
      )
      .one(db)
      .await?;

    if let Some(token_in_db) = token_in_db {
      token_in_db.delete(db).await.map(|_| Ok(Some(())))?
    } else {
      Err(RefreshTokenStoreError::NoUserWithSuchToken)
    }
  }

  async fn remove_all_for_user(&self, user_id: &UserId) -> Result<(), RefreshTokenStoreError> {
    let db = database::get().await;

    entities::refresh_token::Entity::delete_many()
      .filter(entities::refresh_token::COLUMN.user_id.eq(*user_id))
      .exec(db)
      .await?;

    Ok(())
  }
}

impl From<RefreshTokenStoreError> for RefreshError {
  fn from(value: RefreshTokenStoreError) -> Self {
    match value {
      RefreshTokenStoreError::InvalidToken => RefreshError::Unauthorized,
      RefreshTokenStoreError::TokenRotatedOut | RefreshTokenStoreError::TokenExpired => {
        RefreshError::Expired
      }
      RefreshTokenStoreError::NoUserWithSuchToken => RefreshError::UnknownIdentifier,
      RefreshTokenStoreError::Internal(_) => RefreshError::Internal,
    }
  }
}

// ---------------------------------- Middleware -------------------------------------

#[derive(Clone, Default)]
pub struct JWTAuthorizedInterceptor
where
  JWTKey: 'static,
{
  pub key: Arc<JWTKey>,
}

#[tonic::async_trait]
impl RequestInterceptor for JWTAuthorizedInterceptor {
  async fn intercept(
    &self,
    mut request: http::Request<Body>,
  ) -> Result<http::Request<Body>, tonic::Status> {
    let key = self.key.clone();

    if let Some(user_id) = check_auth(&request, key) {
      request.extensions_mut().insert(user_id);

      Ok(request)
    } else {
      Err(tonic::Status::unauthenticated("Unauthenticated"))
    }
  }
}

// auth per client rate limiter
#[derive(Clone)]
pub struct ClientRateLimitInterceptor {
  pub limiter: Arc<RateLimiter>,
}

#[tonic::async_trait]
impl RequestInterceptor for ClientRateLimitInterceptor {
  async fn intercept(
    &self,
    request: http::Request<Body>,
  ) -> Result<http::Request<Body>, tonic::Status> {
    let tcp_info =
      request
        .extensions()
        .get::<TcpConnectInfo>()
        .ok_or(tonic::Status::unavailable(
          "Could not resolve TCP connection info.",
        ))?;

    let address = tcp_info.remote_addr().ok_or(tonic::Status::unavailable(
      "Could not resolve remote address.",
    ))?;

    if self
      .limiter
      .check(&address.ip().to_string())
      .await
      .unwrap()
      .permitted
    {
      Ok(request)
    } else {
      Err(tonic::Status::resource_exhausted("Too many requests."))
    }
  }
}

fn check_auth<B>(req: &Request<B>, key: Arc<JWTKey>) -> Option<Uuid> {
  let headers = req.headers();

  let token = headers
    .get(http::header::AUTHORIZATION)?
    .to_str()
    .ok()?
    .strip_prefix("Bearer ")?;

  get_uuid_from_access_token(key.get(), token, DEFAULT_TIME_TOLERANCE_SECS.into())
}

fn get_jti_for_token(key: &HS256Key, token: &str) -> Option<Uuid> {
  let options = jwt_simple::prelude::VerificationOptions {
    time_tolerance: Some(jwt_simple::prelude::Duration::from_secs(
      60 * 60 * 24 * 365 * 10, // 10y, comfortably > max refresh lifetime
    )),

    ..Default::default()
  };

  let claims = key
    .verify_token::<NoCustomClaims>(token, Some(options))
    .ok()?;

  let jwt_id = claims.jwt_id?;
  let issuer = claims.issuer?;

  if issuer == CONFIG.environment.to_string() {
    return Uuid::parse_str(&jwt_id).ok();
  };

  None
}
fn get_uuid_from_access_token(
  key: &HS256Key,
  token: &str,
  time_tolerance: jwt_simple::prelude::Duration,
) -> Option<Uuid> {
  let options = jwt_simple::prelude::VerificationOptions {
    time_tolerance: Some(time_tolerance),
    ..Default::default()
  };

  let claims = key
    .verify_token::<NoCustomClaims>(token, Some(options))
    .ok()?;

  let subject = claims.subject?;
  let audiences = claims.audiences?;
  let issuer = claims.issuer?;

  if audiences.into_string().ok()? == JWT_AUTH_TOKEN_AUD && issuer == CONFIG.environment.to_string()
  {
    return Uuid::parse_str(&subject).ok();
  };

  None
}

fn get_uuid_from_verify_token(
  key: &HS256Key,
  token: &str,
  time_tolerance: jwt_simple::prelude::Duration,
) -> Option<Uuid> {
  let options = jwt_simple::prelude::VerificationOptions {
    time_tolerance: Some(time_tolerance),
    ..Default::default()
  };

  let claims = key
    .verify_token::<NoCustomClaims>(token, Some(options))
    .ok()?;

  let subject = claims.subject?;
  let audiences = claims.audiences?;
  let issuer = claims.issuer?;

  if audiences.into_string().ok()? == JWT_REFRESH_TOKEN_AUD
    && issuer == CONFIG.environment.to_string()
  {
    return Uuid::parse_str(&subject).ok();
  };

  None
}

// ------------------------ Config ------------------------

fn jwt_access_duration() -> Duration {
  Duration::from_secs(CONFIG.auth.jwt_access_duration_secs)
}

fn jwt_refresh_duration() -> Duration {
  Duration::from_secs(CONFIG.auth.jwt_refresh_duration_secs)
}

#[derive(Default, Clone)]
pub struct RouterState<CodeStore, RefreshStore> {
  code_store: Arc<tokio::sync::Mutex<CodeStore>>,
  refresh_store: RefreshStore,
  resend: Arc<resend_rs::Resend>,
  jwt_key: JWTKey,
}

static JWT_AUTH_TOKEN_AUD: &str = "authorization";
static JWT_REFRESH_TOKEN_AUD: &str = "refresh";

// ------------------------ Routes ------------------------

#[derive(Default, Clone)]
pub struct AuthServer<
  C: CodeStore + Send + Sync + 'static,
  R: RefreshTokenStore + Send + Sync + 'static,
> {
  pub state: RouterState<C, R>,
}

#[tonic::async_trait]
impl AuthService for AuthServer<InMemoryCodeStore, DbTokenStore> {
  async fn register(
    &self,
    request: tonic::Request<RegisterRequest>,
  ) -> Result<tonic::Response<RegisterResponse>, tonic::Status> {
    let jwt_id = Uuid::new_v4();
    let inner_request = request.into_inner();

    let db = database::get().await;

    let user = entities::user::Entity::find()
      .filter(entities::user::Column::Email.eq(&inner_request.email))
      .one(db)
      .await
      .map_err(|e| {
        eprintln!("Error fetching from db:{e}");
        tonic::Status::internal("Unknown error occurred")
      })?;

    if user.is_none() {
      let code = resend::send_auth_email(&inner_request.email, self.state.resend.clone()).await?;

      self
        .state
        .code_store
        .lock()
        .await
        .insert(
          jwt_id,
          EmailCodePair {
            code,
            info: VerifyType::Register {
              email: inner_request.email,
              username: inner_request.username,
            },
            verify_attempts: 0,
          },
        )
        .await;
    };

    Ok(tonic::Response::new(
      RegisterReturn { identifier: jwt_id }.into_proto(),
    ))
  }

  async fn login(
    &self,
    request: tonic::Request<LoginRequest>,
  ) -> Result<tonic::Response<LoginResponse>, tonic::Status> {
    let jwt_id: JWTId = Uuid::new_v4();
    let inner_request = request.into_inner();

    let db = database::get().await;

    let user = entities::user::Entity::find()
      .filter(entities::user::Column::Email.eq(&inner_request.email))
      .one(db)
      .await
      .map_err(|e| {
        eprintln!("Error fetching from db:{e}");
        tonic::Status::internal("Unknown error occurred")
      })?;

    if let Some(user) = user {
      let code = resend::send_auth_email(&inner_request.email, self.state.resend.clone()).await?;

      self
        .state
        .code_store
        .lock()
        .await
        .insert(
          jwt_id,
          EmailCodePair {
            code,
            info: VerifyType::Login {
              email: inner_request.email,
              user_id: user.id,
              username: user.username,
            },
            verify_attempts: 0,
          },
        )
        .await;
    };

    Ok(tonic::Response::new(
      LoginReturn { identifier: jwt_id }.into_proto(),
    ))
  }

  async fn verify(
    &self,
    request: tonic::Request<VerifyRequest>,
  ) -> Result<tonic::Response<VerifyResponse>, tonic::Status> {
    let inner_request = request.into_inner();

    let VerifyCommand {
      identifier,
      email: incoming_email,
      code: code_attempt,
    } = inner_request.try_into_domain()?;

    let EmailCodePair { code, info, .. } = self
      .state
      .code_store
      .lock()
      .await
      .get_email_code_pair(&identifier)
      .await
      .map(Ok)
      .unwrap_or(Err(VerifyError::UnknownIdentifier))
      .map_err(|e| e.into_status())?;

    let (email, user_id, username) = match info {
      VerifyType::Login {
        ref email,
        user_id,
        ref username,
      } => (email.clone(), user_id, username.clone()),
      VerifyType::Register {
        ref email,
        ref username,
      } => (email.clone(), uuid::Uuid::new_v4(), username.clone()),
    };

    let TokenPair {
      access_token,
      refresh_token,
      duration,
    } = generate_tokens(user_id, self.state.jwt_key.get())
      .await
      .map_err(|_| VerifyError::Internal)
      .map_err(|e| e.into_status())?;

    if code_attempt == code && email == incoming_email {
      if let VerifyType::Register { username, email } = info {
        let db = database::get().await;

        let new_user = entities::user::Model {
          id: user_id,
          username,
          email,
          status: entities::user::Status::Online,
        }
        .into_active_model();

        entities::user::Entity::insert(new_user)
          .on_conflict_do_nothing()
          .exec(db)
          .await
          .map_err(|e| {
            eprintln!("Error creating user: {e}");
            tonic::Status::internal("Error while creating user.")
          })?;
      };

      self
        .state
        .refresh_store
        .insert(refresh_token.clone())
        .await?;

      self.state.code_store.lock().await.delete(&identifier).await;

      Ok(tonic::Response::new(
        VerifyReturn {
          access_token,
          refresh_token,
          token_duration: duration,
          user_id,
          username,
        }
        .into_proto(),
      ))
    } else {
      Err(VerifyError::InvalidCode.into_status())
    }
  }

  async fn refresh(
    &self,
    request: tonic::Request<RefreshRequest>,
  ) -> Result<tonic::Response<RefreshResponse>, tonic::Status> {
    let request_inner = request.into_inner();

    let RefreshCommand {
      refresh_token: incoming_refresh_token,
    } = request_inner.try_into_domain()?;

    if get_uuid_from_verify_token(
      self.state.jwt_key.get(),
      &incoming_refresh_token,
      self.state.refresh_store.time_tolerance,
    )
    .is_none()
    {
      return Err(RefreshError::Unauthorized.into_status());
    };

    let user_id = self
      .state
      .refresh_store
      .has(&incoming_refresh_token)
      .await
      .map_err(|err| -> RefreshError { err.into() })
      .map_err(|e| e.into_status())?;

    let TokenPair {
      access_token,
      refresh_token,
      duration: _,
    } = generate_tokens(user_id, self.state.jwt_key.get())
      .await
      .map_err(|_| RefreshError::Internal.into_status())?;

    self
      .state
      .refresh_store
      .rotate(&incoming_refresh_token, refresh_token.clone())
      .await
      .map_err(|err| -> RefreshError { err.into() })
      .map_err(|err| err.into_status())?;

    Ok(
      RefreshReturn {
        access_token,
        refresh_token,
      }
      .into_proto()
      .into(),
    )
  }
}

// /// expects the user id as identifier
async fn generate_tokens(identifier: UserId, key: &HS256Key) -> Result<TokenPair, anyhow::Error> {
  let access_duration = jwt_access_duration();
  let refresh_duration = jwt_refresh_duration();

  let claims = Claims::create(access_duration.into())
    .with_subject(identifier.to_string())
    .with_audience(JWT_AUTH_TOKEN_AUD.to_string())
    .with_issuer(CONFIG.environment.to_string())
    .with_jwt_id(Uuid::new_v4());
  let access_token = key
    .authenticate(claims)
    .map_err(|_| anyhow::anyhow!("Internal"))?;

  let claims = Claims::create(refresh_duration.into())
    .with_subject(identifier.to_string())
    .with_jwt_id(Uuid::new_v4())
    .with_issuer(CONFIG.environment.to_string())
    .with_audience(JWT_REFRESH_TOKEN_AUD.to_string());

  let refresh_token = key
    .authenticate(claims)
    .map_err(|_| anyhow::anyhow!("Internal"))?;

  Ok(TokenPair {
    access_token,
    refresh_token,
    duration: refresh_duration,
  })
}

// ---------------------------- (mostly) LLM generated unit tests for RefreshTokenStore ---------------------------

#[cfg(test)]
mod tests {
  use bytes::Bytes;
  use futures_util::FutureExt;

  use super::*;
  use std::{panic::AssertUnwindSafe, time::Duration};

  fn test_key() -> HS256Key {
    let key_bytes: Bytes = hex::decode(&CONFIG.auth.jwt_key_hex)
      .expect("Invalid key, decode failed")
      .into();
    HS256Key::from_bytes(&key_bytes)
  }

  fn make_valid_token(key: &HS256Key, user_id: UserId) -> Token {
    let refresh_duration = super::jwt_refresh_duration();
    let claims = Claims::create(refresh_duration.into())
      .with_subject(user_id.to_string())
      .with_issuer(CONFIG.environment.to_string())
      .with_audience(JWT_REFRESH_TOKEN_AUD.to_string())
      .with_jwt_id(Uuid::new_v4().to_string());

    key.authenticate(claims).unwrap()
  }

  fn make_expired_token(key: &HS256Key, user_id: UserId) -> Token {
    let claims = Claims::create(Duration::from_secs(1).into())
      .with_subject(user_id.to_string())
      .with_audience(JWT_REFRESH_TOKEN_AUD.to_string())
      .with_issuer(CONFIG.environment.to_string())
      .with_jwt_id(Uuid::new_v4());

    key.authenticate(claims).unwrap()
  }

  /// Inserts a user row so the refresh-token FK is satisfied.
  async fn insert_test_user(user_id: UserId) {
    let db = database::get().await;

    entities::user::Entity::insert(
      entities::user::Model {
        id: user_id,
        username: format!("test_user_{}", user_id.simple()),
        email: format!("{}@test.local", user_id.simple()),
        status: entities::user::Status::Online,
      }
      .into_active_model(),
    )
    .exec(db)
    .await
    .expect("failed to insert test user");
  }

  /// Removes the user and any refresh tokens still pointing at it.
  async fn delete_test_user(user_id: UserId) {
    let db = database::get().await;

    // Delete tokens first in case there's no ON DELETE CASCADE on the FK.
    let _ = entities::refresh_token::Entity::delete_many()
      .filter(entities::refresh_token::COLUMN.user_id.eq(user_id))
      .exec(db)
      .await;

    let _ = entities::user::Entity::delete_by_id(user_id).exec(db).await;
  }

  /// Creates a fresh user, runs the body with its id, then cleans up.
  async fn with_test_user<F, Fut>(f: F)
  where
    F: FnOnce(UserId) -> Fut,
    Fut: std::future::Future<Output = ()>,
  {
    let user_id = Uuid::new_v4();
    insert_test_user(user_id).await;

    // AssertUnwindSafe because the future may hold non-UnwindSafe state;
    // safe here since we re-raise the panic and don't observe broken state.
    let result = AssertUnwindSafe(f(user_id)).catch_unwind().await;

    delete_test_user(user_id).await; // always runs

    if let Err(panic) = result {
      std::panic::resume_unwind(panic);
    }
  }

  async fn test_insert_and_has(store: &impl RefreshTokenStore, key: &HS256Key) {
    with_test_user(|user_id| async move {
      let token = make_valid_token(key, user_id);

      let inserted_id = store.insert(token.clone()).await.unwrap();
      assert_eq!(inserted_id, user_id);

      let has_id = store.has(&token).await.unwrap();
      assert_eq!(has_id, user_id);
    })
    .await;
  }

  async fn test_has_garbage_token_returns_invalid(store: &impl RefreshTokenStore, _key: &HS256Key) {
    let result = store.has(&"not-a-token".to_string()).await;
    assert!(matches!(result, Err(RefreshTokenStoreError::InvalidToken)));
  }

  async fn test_has_unknown_valid_token_detects_rotation(
    store: &impl RefreshTokenStore,
    key: &HS256Key,
  ) {
    with_test_user(|user_id| async move {
      let token1 = make_valid_token(key, user_id);
      let token2 = make_valid_token(key, user_id);

      store.insert(token1.clone()).await.unwrap();

      store
        .rotate(&token1, token2)
        .await
        .expect("Rotation succeeds");

      let result = store.has(&token1).await;
      assert!(matches!(
        result,
        Err(RefreshTokenStoreError::TokenRotatedOut)
      ));
    })
    .await;
  }

  async fn test_has_expired_token_removes_and_returns_expired(
    store: &impl RefreshTokenStore,
    key: &HS256Key,
  ) {
    with_test_user(|user_id| async move {
      let token = make_expired_token(key, user_id);

      store.insert(token.clone()).await.unwrap();

      store
        .has(&token)
        .await
        .expect("Token should be valid at this point");

      tokio::time::sleep(Duration::from_secs(1)).await;

      let result = store.has(&token).await;
      assert!(matches!(result, Err(RefreshTokenStoreError::TokenExpired)));

      let result = store.has(&token).await;
      assert!(matches!(result, Err(RefreshTokenStoreError::InvalidToken)));
    })
    .await;
  }

  async fn test_remove_cleans_up(store: &impl RefreshTokenStore, key: &HS256Key) {
    with_test_user(|user_id| async move {
      let token = make_valid_token(key, user_id);

      store.insert(token.clone()).await.unwrap();
      let result = store.remove(&token).await.unwrap();
      assert!(result.is_some());

      let token2 = make_valid_token(key, user_id);
      store.insert(token2).await.unwrap();

      let result = store.has(&token).await;
      assert!(matches!(
        result,
        Err(RefreshTokenStoreError::TokenRotatedOut)
      ));
    })
    .await;
  }

  async fn test_remove_nonexistent_returns_none(store: &impl RefreshTokenStore, key: &HS256Key) {
    // valid JWT, never inserted, no user — remove filters by token string only.
    let user_id = Uuid::new_v4();
    let token = make_valid_token(key, user_id);

    let result = store.remove(&token).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::NoUserWithSuchToken)
    ));
  }

  async fn test_remove_garbage_token_returns_invalid_token(
    store: &impl RefreshTokenStore,
    _key: &HS256Key,
  ) {
    let result = store.remove(&"not-a-token".to_string()).await;
    assert!(matches!(result, Err(RefreshTokenStoreError::InvalidToken)));
  }

  async fn test_remove_all_for_user_cleans_up_everything(
    store: &impl RefreshTokenStore,
    key: &HS256Key,
  ) {
    with_test_user(|user_id| async move {
      let token_a = make_valid_token(key, user_id);
      let token_b = make_valid_token(key, user_id);
      let token_c = make_valid_token(key, user_id);

      store.insert(token_a.clone()).await.unwrap();
      store.insert(token_b.clone()).await.unwrap();

      store.remove_all_for_user(&user_id).await.unwrap();

      store.insert(token_c.clone()).await.unwrap();

      let result = store.has(&token_a).await;
      assert!(matches!(
        result,
        Err(RefreshTokenStoreError::TokenRotatedOut)
      ));

      let result = store.has(&token_b).await;

      assert!(matches!(
        result,
        Err(RefreshTokenStoreError::TokenRotatedOut)
      ));

      let result = store.has(&token_c).await;

      assert!(matches!(
        result,
        Err(RefreshTokenStoreError::TokenRotatedOut)
      ));
    })
    .await;
  }

  async fn test_remove_all_for_unknown_user(store: &impl RefreshTokenStore, _key: &HS256Key) {
    let unknown = Uuid::new_v4();
    let result = store.remove_all_for_user(&unknown).await;
    assert!(matches!(result, Ok(())));
  }

  async fn run_all(store: &impl RefreshTokenStore, key: &HS256Key) {
    test_insert_and_has(store, key).await;
    test_has_garbage_token_returns_invalid(store, key).await;
    test_has_unknown_valid_token_detects_rotation(store, key).await;
    test_has_expired_token_removes_and_returns_expired(store, key).await;
    test_remove_cleans_up(store, key).await;
    test_remove_nonexistent_returns_none(store, key).await;
    test_remove_garbage_token_returns_invalid_token(store, key).await;
    test_remove_all_for_user_cleans_up_everything(store, key).await;
    test_remove_all_for_unknown_user(store, key).await;
  }

  #[tokio::test]
  async fn in_memory_token_store() {
    let key = test_key();
    let store = DbTokenStore::new(jwt_simple::prelude::Duration::from_secs(0));
    run_all(&store, &key).await;
  }
}
