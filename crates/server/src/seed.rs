use sea_orm::{EntityTrait, IntoActiveModel, QueryFilter};

use crate::{
  entities::{self, user_server::Role},
  library::database,
};

// create a single, global server,
// with two text chats and two voice calls,
// all existing and new users are assigned automatically to this server
pub async fn seed() {
  let db = database::get().await;

  let mut server = entities::server::Entity::find()
    .filter(
      entities::server::Entity::COLUMN
        .name
        .eq("Intergalactic Federation"),
    )
    .one(db)
    .await
    .unwrap();

  if server.is_none() {
    server = Some(
      entities::server::Entity::insert(
        entities::server::Model {
          id: uuid::Uuid::new_v4(),
          name: "Intergalactic Federation".to_string(),
        }
        .into_active_model(),
      )
      .exec_with_returning(db)
      .await
      .unwrap(),
    );
  };

  let server = server.unwrap();

  let text_channels = entities::text_channel::Entity::find()
    .all(db)
    .await
    .unwrap();

  if text_channels.is_empty() {
    let channels = entities::text_channel::Entity::insert_many([
      entities::text_channel::Model {
        id: uuid::Uuid::new_v4(),
        server_id: server.id.into(),
        name: "mess-hall".into(),
        is_default: true,
      }
      .into_active_model(),
      entities::text_channel::Model {
        id: uuid::Uuid::new_v4(),
        server_id: server.id.into(),
        name: "forge".into(),
        is_default: false,
      }
      .into_active_model(),
    ])
    .exec_with_returning_keys(db)
    .await
    .unwrap();

    let [first, second] = channels[0..2] else {
      panic!()
    };

    entities::channel::Entity::insert_many([
      entities::channel::Model {
        id: uuid::Uuid::new_v4(),
        text_channel_id: Some(first),
        user_user_id: None,
      }
      .into_active_model(),
      entities::channel::Model {
        id: uuid::Uuid::new_v4(),
        text_channel_id: Some(second),
        user_user_id: None,
      }
      .into_active_model(),
    ])
    .exec(db)
    .await
    .unwrap();
  };

  let voice_channels = entities::voice_channel::Entity::find()
    .all(db)
    .await
    .unwrap();

  if voice_channels.is_empty() {
    entities::voice_channel::Entity::insert_many([
      entities::voice_channel::Model {
        id: uuid::Uuid::new_v4(),
        server_id: server.id.into(),
        name: "Augmentation Chambers".into(),
      }
      .into_active_model(),
      entities::voice_channel::Model {
        id: uuid::Uuid::new_v4(),
        server_id: server.id.into(),
        name: "Academy Network".into(),
      }
      .into_active_model(),
    ])
    .exec(db)
    .await
    .unwrap();
  };

  let users = entities::user::Entity::find()
    .find_also_related(entities::server::Entity)
    .all(db)
    .await
    .unwrap();

  // find users without a server and create the relationship
  let new_user_servers = users
    .into_iter()
    .filter(|(_, server)| server.is_none())
    .map(|(user, _)| {
      entities::user_server::Model {
        user_id: user.id,
        server_id: server.id,
        role: Role::Admin,
      }
      .into_active_model()
    });

  entities::user_server::Entity::insert_many(new_user_servers)
    .exec(db)
    .await
    .unwrap();
}
