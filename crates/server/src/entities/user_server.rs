use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[derive(Debug, Clone, EnumIter, DeriveActiveEnum, PartialEq)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Role {
  #[sea_orm(string_value = "Owner")]
  Owner,
  #[sea_orm(string_value = "Admin")]
  Admin,
  #[sea_orm(string_value = "Member")]
  Member,
}

#[sea_orm::model]
#[derive(DeriveEntityModel, Clone, Debug)]
#[sea_orm(table_name = "user_server")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub user_id: Uuid,
  #[sea_orm(primary_key, auto_increment = false)]
  pub server_id: Uuid,
  #[sea_orm(belongs_to, from = "user_id", to = "id")]
  pub user: Option<super::user::Entity>,
  #[sea_orm(belongs_to, from = "server_id", to = "id")]
  pub server: Option<super::server::Entity>,

  pub role: Role,
}

impl ActiveModelBehavior for ActiveModel {}
