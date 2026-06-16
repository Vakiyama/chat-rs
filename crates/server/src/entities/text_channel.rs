use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(DeriveEntityModel, Clone, Debug)]
#[sea_orm(table_name = "text_channel")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub id: Uuid,
  pub name: String,
  #[sea_orm(has_one)]
  pub channel: HasOne<super::channel::Entity>,
  pub server_id: Option<Uuid>,
  #[sea_orm(belongs_to, from = "server_id", to = "id")]
  pub server: HasOne<super::server::Entity>,
  pub is_default: bool,
}

impl ActiveModelBehavior for ActiveModel {}
