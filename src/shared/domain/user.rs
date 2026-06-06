use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct MeReturn {
  pub username: String,
  pub user_id: Uuid,
}
