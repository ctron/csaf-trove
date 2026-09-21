pub use sea_orm_migration::prelude::*;

mod m20240101_000001_create_initial_schema;
mod m20240102_000002_add_retrieval_error;

/// Migrator that applies all schema migrations in order.
pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240101_000001_create_initial_schema::Migration),
            Box::new(m20240102_000002_add_retrieval_error::Migration),
        ]
    }
}
