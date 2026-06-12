use sea_orm::entity::prelude::*;

#[derive(Debug, Clone, EnumIter, DeriveActiveEnum, PartialEq)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Status {
  #[sea_orm(string_value = "Online")]
  Online,
  #[sea_orm(string_value = "Away")]
  Away,
  #[sea_orm(string_value = "DoNotDisturb")]
  DoNotDisturb,
  #[sea_orm(string_value = "Offline")]
  Offline,
}

#[sea_orm::model]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "user")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub id: Uuid,
  pub username: String,
  pub email: String,
  pub status: Status,
  #[sea_orm(has_many, via = "user_server")]
  pub servers: HasMany<super::server::Entity>,
  #[sea_orm(has_many)]
  pub refresh_tokens: HasMany<super::refresh_token::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
