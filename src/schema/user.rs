use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: Uuid,
  pub username: String,
}

impl Model {
  pub fn new(username: &str) -> Self {
    Model {
      id: Uuid::new_v4(),
      username: username.into(),
    }
  }
}

impl ActiveModelBehavior for ActiveModel {}
