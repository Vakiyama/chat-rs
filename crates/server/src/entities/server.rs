use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(DeriveEntityModel, Clone, Debug)]
#[sea_orm(table_name = "server")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub id: Uuid,
  pub name: String,
  #[sea_orm(has_many, via = "user_server")]
  pub members: HasMany<super::user::Entity>,
  #[sea_orm(has_many)]
  pub text_channel: HasMany<super::text_channel::Entity>,
  #[sea_orm(has_many)]
  pub voice_channel: HasMany<super::voice_channel::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
