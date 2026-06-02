use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "post")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: Uuid,
  pub author_name: String,
  pub content: String,
}

impl ActiveModelBehavior for ActiveModel {}
