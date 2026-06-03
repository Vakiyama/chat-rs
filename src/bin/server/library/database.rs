use chat_rs::config::CONFIG;
use sea_orm::{Database, DatabaseConnection};
use tokio::sync::OnceCell;

static DB_CLIENT: OnceCell<DatabaseConnection> = OnceCell::const_new();

pub async fn get() -> &'static DatabaseConnection {
  DB_CLIENT
    .get_or_init(async || {
      let db = Database::connect(CONFIG.server.db_connection.clone())
        .await
        .unwrap();

      db.get_schema_registry(module_path!().split("::").next().unwrap())
        .sync(&db)
        .await
        .expect("Failed to sync db schema");

      db
    })
    .await
}
