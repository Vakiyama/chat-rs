use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "post")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: Uuid,
  pub content: String,
}

impl Model {
  pub fn new(content: &str) -> Self {
    Self {
      id: Uuid::new_v4(),
      content: content.to_string(),
    }
  }
}

impl ActiveModelBehavior for ActiveModel {}
