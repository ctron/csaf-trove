pub use sea_orm_migration::prelude::*;

mod m20240101_000001_create_initial_schema;

/// Migrator that applies all schema migrations in order.
pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20240101_000001_create_initial_schema::Migration)]
    }
}
