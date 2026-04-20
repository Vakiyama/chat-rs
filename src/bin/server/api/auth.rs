use futures_util::future::BoxFuture;
use http::{Request, Response, StatusCode};
use std::{
  collections::{HashMap, HashSet},
  sync::{Arc, Mutex},
  time::Duration,
};

use axum::{Json, Router, extract::State, routing::post};
use chat_rs::shared::auth::{
  EmailCodePair, JWTKey, LoginBody, LoginResponse, RefreshBody, RefreshError, RefreshResponse,
  Token, TokenPair, UserId, VerifyBody, VerifyError, VerifyResponse,
};
use jwt_simple::{
  claims::{Claims, DEFAULT_TIME_TOLERANCE_SECS, NoCustomClaims},
  prelude::{HS256Key, MACLike},
};
use tower_http::auth::AsyncAuthorizeRequest;
use uuid::Uuid;

use crate::library::resend;

pub fn router() -> Router {
  Router::new()
    .route(
      "/login",
      post(login_handler::<InMemoryStore, InMemoryTokenStore>),
    )
    .with_state(RouterState::default())
}

// ------------------------- Stores -------------------------

#[derive(Default, Clone)]
pub struct InMemoryStore {
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

pub trait CodeStore {
  /// given an uuid, gets the pending
  async fn get_email_code_pair(&self, uuid: &Uuid) -> Option<&EmailCodePair>;

  async fn insert(&mut self, uuid: Uuid, email_code: EmailCodePair) -> Option<EmailCodePair>;
}

#[derive(Debug)]
pub enum RefreshTokenStoreError {
  InvalidToken,
  TokenRotatedOut, // leads to a signout on all tokens due to potential malicious actor
  TokenExpired,
  NoUserWithSuchToken,
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

/// This struct has no guarantees about token validity across server restarts
/// the intention is to re-implement the refresh token store trait for either a redis or db based solution.
#[derive(Default, Clone)]
struct InMemoryTokenStore {
  lookups: Arc<Mutex<TokenUserIdReverseLookup>>,
  key: JWTKey,
  time_tolerance: jwt_simple::prelude::Duration,
}

#[derive(Default, Clone)]
struct TokenUserIdReverseLookup {
  tokens: HashMap<Token, UserId>,
  user_ids: HashMap<UserId, HashSet<Token>>,
}

impl RefreshTokenStore for InMemoryTokenStore {
  fn new(time_tolerance: jwt_simple::prelude::Duration) -> Self {
    Self {
      time_tolerance,
      lookups: Default::default(),
      key: Default::default(),
    }
  }

  async fn has(&self, token: &Token) -> Result<UserId, RefreshTokenStoreError> {
    let token_uuid = get_uuid_from_token(self.key.get(), token, self.time_tolerance);

    let has_token = {
      let lookup = self.lookups.lock().unwrap();
      lookup.tokens.contains_key(token)
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
    let uuid = get_uuid_from_token(self.key.get(), &token, self.time_tolerance)
      .map(Ok)
      .unwrap_or(Err(RefreshTokenStoreError::InvalidToken))?;

    let mut lookup = self.lookups.lock().unwrap();

    lookup.tokens.insert(token.clone(), uuid);

    match lookup.user_ids.get_mut(&uuid) {
      Some(set) => {
        set.insert(token);
      }
      None => {
        let mut set = HashSet::new();
        set.insert(token);
        let _ = lookup.user_ids.insert(uuid, set);
      }
    }

    Ok(uuid)
  }

  async fn remove(&self, token: &Token) -> Result<Option<()>, RefreshTokenStoreError> {
    let mut lookup = self.lookups.lock().unwrap();

    let uuid = get_uuid_from_token(self.key.get(), token, self.time_tolerance)
      .map(Ok)
      .unwrap_or_else(|| {
        // token is invalid, but we may be able to remove it as it may just be expired

        if let Some(identifier) = lookup.tokens.get(token) {
          Ok(*identifier)
        } else {
          Err(RefreshTokenStoreError::InvalidToken)
        }
      })?;

    let tokens = lookup
      .user_ids
      .get_mut(&uuid)
      .map(Ok)
      .unwrap_or(Err(RefreshTokenStoreError::NoUserWithSuchToken))?;

    if tokens.len() == 1 {
      let _ = lookup.user_ids.remove(&uuid);
      let _ = lookup.tokens.remove(token);

      Ok(Some(()))
    } else {
      match tokens.remove(token) {
        true => {
          lookup.tokens.remove(token);
          Ok(Some(()))
        }
        false => Ok(None),
      }
    }
  }

  async fn remove_all_for_user(&self, user_id: &UserId) -> Result<(), RefreshTokenStoreError> {
    let tokens = {
      let mut lookup = self.lookups.lock().unwrap();

      lookup
        .user_ids
        .get_mut(user_id)
        .map(|tokens| tokens.clone())
    }
    .map(Ok)
    .unwrap_or(Err(RefreshTokenStoreError::NoUserWithSuchToken))?;

    for token in &tokens {
      self.remove(token).await?;
    }

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
    }
  }
}

// ---------------------------------- Middleware -------------------------------------

#[derive(Clone)]
pub struct JWTAuthorized
where
  JWTKey: 'static,
{
  pub key: Arc<JWTKey>,
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

  get_uuid_from_token(key.get(), token, DEFAULT_TIME_TOLERANCE_SECS.into())
}

fn get_uuid_from_token(
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

  Uuid::parse_str(&subject).ok()
}

// ------------------------ Config ------------------------

const JWT_ACCESS_DURATION: Duration = Duration::from_hours(8);
const JWT_REFRESH_DURATION: Duration = Duration::from_hours(24 * 30);

#[derive(Default, Clone)]
struct RouterState<CodeStore, RefreshStore> {
  code_store: CodeStore,
  refresh_store: RefreshStore,
  resend: Arc<resend_rs::Resend>,
  jwt_key: JWTKey,
}

// ------------------------ Routes ------------------------

async fn login_handler<C, R>(
  State(mut state): State<RouterState<C, R>>,
  Json(payload): Json<LoginBody>,
) -> Result<LoginResponse, resend::Error>
where
  C: CodeStore + Send + Sync + 'static,
  R: RefreshTokenStore + Send + Sync + 'static,
{
  let identifier = Uuid::new_v4();
  let code = resend::send_auth_email(&payload.email, state.resend).await?;

  state
    .code_store
    .insert(
      identifier,
      EmailCodePair {
        code,
        email: payload.email,
      },
    )
    .await;

  Ok(LoginResponse { identifier })
}

async fn verify_handler<C, R>(
  State(state): State<RouterState<C, R>>,
  Json(payload): Json<VerifyBody>,
) -> Result<VerifyResponse, VerifyError>
where
  C: CodeStore + Send + Sync + 'static,
  R: RefreshTokenStore + Send + Sync + 'static,
{
  let VerifyBody {
    identifier,
    email: incoming_email,
    code: code_attempt,
  } = payload;

  let EmailCodePair { code, email } = state
    .code_store
    .get_email_code_pair(&identifier)
    .await
    .map(Ok)
    .unwrap_or(Err(VerifyError::UnknownIdentifier))?;

  let TokenPair {
    access_token,
    refresh_token,
  } = generate_tokens(identifier, state.jwt_key.get())
    .await
    .map_err(|_| VerifyError::Internal)?;

  if &code_attempt == code && email == &incoming_email {
    Ok(VerifyResponse {
      access_token,
      refresh_token,
      duration_milliseconds: JWT_ACCESS_DURATION.as_millis(),
    })
  } else {
    Err(VerifyError::InvalidCode)
  }
}

async fn refresh_handler<C, R>(
  State(state): State<RouterState<C, R>>,
  Json(body): Json<RefreshBody>,
) -> Result<RefreshResponse, RefreshError>
where
  C: CodeStore + Send + Sync + 'static,
  R: RefreshTokenStore + Send + Sync + 'static,
{
  let RefreshBody {
    refresh_token: incoming_refresh_token,
  } = body;

  let user_id = state
    .refresh_store
    .has(&incoming_refresh_token)
    .await
    .map_err(|err| -> RefreshError { err.into() })?;

  let TokenPair {
    access_token,
    refresh_token,
  } = generate_tokens(user_id, state.jwt_key.get())
    .await
    .map_err(|_| RefreshError::Internal)?;

  state
    .refresh_store
    .rotate(&incoming_refresh_token, refresh_token.clone())
    .await
    .map_err(|err| -> RefreshError { err.into() })?;

  Ok(RefreshResponse {
    access_token,
    refresh_token,
  })
}

/// expects the user id as identifier
async fn generate_tokens(identifier: UserId, key: &HS256Key) -> Result<TokenPair, anyhow::Error> {
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

// ---------------------------- (mostly) LLM generated unit tests for RefreshTokenStore ---------------------------

#[cfg(test)]
mod tests {
  use bytes::Bytes;

  use super::*;
  use std::time::Duration;

  fn test_key() -> HS256Key {
    let _env = dotenvy::dotenv();

    let key_bytes: Bytes = hex::decode(std::env::var("JWT_KEY").expect("Missing JWT_KEY env var"))
      .expect("Invalid key, decode failed")
      .into();
    HS256Key::from_bytes(&key_bytes)
  }

  fn make_valid_token(key: &HS256Key, user_id: UserId) -> Token {
    let claims = Claims::create(JWT_REFRESH_DURATION.into())
      .with_subject(user_id.to_string())
      .with_jwt_id(Uuid::new_v4().to_string());

    key.authenticate(claims).unwrap()
  }

  fn make_expired_token(key: &HS256Key, user_id: UserId) -> Token {
    let claims = Claims::create(Duration::from_secs(1).into()).with_subject(user_id.to_string());
    key.authenticate(claims).unwrap()
  }

  async fn test_insert_and_has(store: &impl RefreshTokenStore, key: &HS256Key) {
    let user_id = Uuid::new_v4();
    let token = make_valid_token(key, user_id);

    let inserted_id = store.insert(token.clone()).await.unwrap();
    assert_eq!(inserted_id, user_id);

    let has_id = store.has(&token).await.unwrap();
    assert_eq!(has_id, user_id);
  }

  async fn test_has_garbage_token_returns_invalid(store: &impl RefreshTokenStore, _key: &HS256Key) {
    let result = store.has(&"not-a-token".to_string()).await;
    assert!(matches!(result, Err(RefreshTokenStoreError::InvalidToken)));
  }

  async fn test_has_unknown_valid_token_detects_rotation(
    store: &impl RefreshTokenStore,
    key: &HS256Key,
  ) {
    let user_id = Uuid::new_v4();
    // valid token that was never inserted — simulates a rotated-out token
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
  }

  async fn test_has_expired_token_removes_and_returns_expired(
    store: &impl RefreshTokenStore,
    key: &HS256Key,
  ) {
    let user_id = Uuid::new_v4();
    let token = make_expired_token(key, user_id);

    store.insert(token.clone()).await.unwrap();

    store
      .has(&token)
      .await
      .expect("Token should be valid at this point");

    tokio::time::sleep(Duration::from_secs(1)).await;

    let result = store.has(&token).await;
    assert!(matches!(result, Err(RefreshTokenStoreError::TokenExpired)));

    // should also be removed from the store after expiry is detected
    let result = store.has(&token).await;
    assert!(matches!(result, Err(RefreshTokenStoreError::InvalidToken)));
  }

  async fn test_remove_cleans_up(store: &impl RefreshTokenStore, key: &HS256Key) {
    let user_id = Uuid::new_v4();
    let token = make_valid_token(key, user_id);

    store.insert(token.clone()).await.unwrap();
    let result = store.remove(&token).await.unwrap();
    assert!(result.is_some());

    let token2 = make_valid_token(key, user_id);
    store.insert(token2).await.unwrap();

    // token should now be gone — valid JWT not in store means rotation detected
    let result = store.has(&token).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::TokenRotatedOut)
    ));
  }

  async fn test_remove_nonexistent_returns_none(store: &impl RefreshTokenStore, key: &HS256Key) {
    let user_id = Uuid::new_v4();
    let token = make_valid_token(key, user_id);

    // never inserted, valid JWT — hits rotation path in has, but remove
    // should return NoUserWithSuchToken since there's no user entry
    let result = store.remove(&token).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::NoUserWithSuchToken)
    ));
  }

  async fn test_remove_garbage_token_returns_invalid(
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
    let user_id = Uuid::new_v4();
    let token_a = make_valid_token(key, user_id);
    let token_b = make_valid_token(key, user_id);
    let token_c = make_valid_token(key, user_id);

    store.insert(token_a.clone()).await.unwrap();
    store.insert(token_b.clone()).await.unwrap();

    store.remove_all_for_user(&user_id).await.unwrap();

    store.insert(token_c.clone()).await.unwrap();

    // both tokens should now look like rotated-out tokens
    let result = store.has(&token_a).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::TokenRotatedOut)
    ));

    // should now not find this token for this user, as we've detected a rotated-out token being
    // used
    let result = store.has(&token_b).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::NoUserWithSuchToken)
    ));

    let result = store.has(&token_c).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::NoUserWithSuchToken)
    ));
  }

  async fn test_remove_all_for_unknown_user(store: &impl RefreshTokenStore, _key: &HS256Key) {
    let unknown = Uuid::new_v4();
    let result = store.remove_all_for_user(&unknown).await;
    assert!(matches!(
      result,
      Err(RefreshTokenStoreError::NoUserWithSuchToken)
    ));
  }

  async fn run_all(store: &impl RefreshTokenStore, key: &HS256Key) {
    test_insert_and_has(store, key).await;
    test_has_garbage_token_returns_invalid(store, key).await;
    test_has_unknown_valid_token_detects_rotation(store, key).await;
    test_has_expired_token_removes_and_returns_expired(store, key).await;
    test_remove_cleans_up(store, key).await;
    test_remove_nonexistent_returns_none(store, key).await;
    test_remove_garbage_token_returns_invalid(store, key).await;
    test_remove_all_for_user_cleans_up_everything(store, key).await;
    test_remove_all_for_unknown_user(store, key).await;
  }

  #[tokio::test]
  async fn in_memory_token_store() {
    let key = test_key();
    let store = InMemoryTokenStore::new(jwt_simple::prelude::Duration::from_secs(0));
    run_all(&store, &key).await;
  }
}
