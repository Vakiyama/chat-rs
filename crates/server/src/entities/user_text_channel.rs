use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(DeriveEntityModel, Clone, Debug)]
#[sea_orm(table_name = "user_text_channel")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub user_id: Uuid,
  #[sea_orm(primary_key, auto_increment = false)]
  pub text_channel_id: Uuid,
  #[sea_orm(belongs_to, from = "user_id", to = "id")]
  pub user: Option<super::user::Entity>,
  #[sea_orm(belongs_to, from = "text_channel_id", to = "id")]
  pub text_channel: Option<super::text_channel::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
