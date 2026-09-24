#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub use sea_orm_migration::prelude::*;

mod m20240101_000001_create_initial_schema;
mod m20240102_000002_add_retrieval_error;
mod m20240103_000003_add_version_count;
mod m20240104_000004_add_test_counts;
mod m20240105_000005_fix_provider_info_table;

/// Migrator that applies all schema migrations in order.
pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240101_000001_create_initial_schema::Migration),
            Box::new(m20240102_000002_add_retrieval_error::Migration),
            Box::new(m20240103_000003_add_version_count::Migration),
            Box::new(m20240104_000004_add_test_counts::Migration),
            Box::new(m20240105_000005_fix_provider_info_table::Migration),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    async fn in_memory_db() -> sea_orm::DatabaseConnection {
        Database::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn migration_check() {
        let db = in_memory_db().await;
        Migrator::status(&db).await.unwrap();
    }

    #[tokio::test]
    async fn migration_up() {
        let db = in_memory_db().await;
        Migrator::up(&db, None).await.unwrap();
    }

    #[tokio::test]
    async fn migration_up_down() {
        let db = in_memory_db().await;
        Migrator::up(&db, None).await.unwrap();
        Migrator::down(&db, None).await.unwrap();
    }
}
