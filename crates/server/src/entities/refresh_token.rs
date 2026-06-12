use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(DeriveEntityModel, Clone, Debug)]
#[sea_orm(table_name = "refresh_token")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub id: Uuid,
  pub refresh_token_jti: Uuid,
  pub user_id: Uuid,
  #[sea_orm(belongs_to, from = "user_id", to = "id")]
  pub user: HasOne<super::user::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
