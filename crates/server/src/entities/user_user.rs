use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(DeriveEntityModel, Debug, Clone)]
#[sea_orm(table_name = "user_user")]
pub struct Model {
  #[sea_orm(primary_key, auto_increment = false)]
  pub id: Uuid,

  // todo: add user user relationship
  #[sea_orm(has_one)]
  pub channel: HasOne<super::channel::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
