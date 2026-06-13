use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(DeriveEntityModel, Debug, Clone)]
#[sea_orm(table_name = "channel")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub id: Uuid,
  pub text_channel_id: Option<Uuid>,
  #[sea_orm(belongs_to, from = "text_channel_id", to = "id")]
  pub text_channel: HasOne<super::text_channel::Entity>,
  pub user_user_id: Option<Uuid>,
  #[sea_orm(belongs_to, from = "user_user_id", to = "id")]
  pub user_user: HasOne<super::user_user::Entity>,
  #[sea_orm(has_many)]
  pub post: HasMany<super::post::Entity>,
}
//
// can belong to a text_channel or user_user

impl ActiveModelBehavior for ActiveModel {}
