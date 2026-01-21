use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: Uuid,
  pub name: String,
}

impl Model {
  pub fn new(name: &str) -> Self {
    Model {
      id: Uuid::new_v4(),
      name: name.into(),
    }
  }
}

impl ActiveModelBehavior for ActiveModel {}
