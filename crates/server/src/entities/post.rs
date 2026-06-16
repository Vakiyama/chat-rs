use chrono::Utc;
use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[sea_orm::model]
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "post")]
pub struct Model {
  #[sea_orm(primary_key)]
  pub id: Uuid,
  pub content: String,

  pub author_id: Option<Uuid>,
  #[sea_orm(belongs_to, from = "author_id", to = "id")]
  pub author: HasOne<super::user::Entity>,

  pub channel_id: Option<Uuid>,
  #[sea_orm(belongs_to, from = "channel_id", to = "id")]
  pub channel: HasOne<super::channel::Entity>,

  pub created_at: chrono::DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
