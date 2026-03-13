use std::collections::HashMap;

use axum::{Router, extract::Query, response::IntoResponse, routing::get};

type JWT = String;

#[derive(Default, Clone)]
struct PendingTokenStore {
  pending: HashMap<String, JWT>,
}

pub fn router() -> Router {
  let token_store = PendingTokenStore::default();

  Router::new()
    .route("/login", get(login_handler))
    .with_state(token_store)
}

enum Error {
  NoEmailQueryParam,
}

// get the email from the qparam, send to resend, create a stateful "email:code -> JWT" pending
// resolution store
async fn login_handler(Query(params): Query<HashMap<String, String>>) -> Result<String, ()> {
  todo!()
}
