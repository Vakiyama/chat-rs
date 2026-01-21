use crate::schema::user::Model as User;
use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "post")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: Uuid,
  // pub author_id: Uuid,
  // #[sea_orm(belongs_to, from = "author_id", to = "id")]
  // pub author: HasOne<super::user::Entity>,
  pub author_name: String,
  pub content: String,
}

impl Model {
  pub fn new(content: &str, author: &str) -> Self {
    Self {
      id: Uuid::new_v4(),
      author_name: author.into(),
      content: content.to_string(),
    }
  }
}

impl ActiveModelBehavior for ActiveModel {}
