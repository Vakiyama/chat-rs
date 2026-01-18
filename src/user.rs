use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "cake")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: i32,
  pub name: String,
  #[sea_orm(has_one)]
  pub fruit: Option<super::fruit::Entity>,
  #[sea_orm(has_many, via = "cake_filling")]
  pub fillings: Vec<super::filling::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
