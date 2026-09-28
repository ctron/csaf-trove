use sea_orm_migration::prelude::*;

/// Records how long each sync run took.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Adds a nullable duration, leaving existing runs without one.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SyncRuns::Table)
                    .add_column(ColumnDef::new(SyncRuns::DurationMs).big_integer())
                    .to_owned(),
            )
            .await
    }

    /// Removes the duration field.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SyncRuns::Table)
                    .drop_column(SyncRuns::DurationMs)
                    .to_owned(),
            )
            .await
    }
}

/// Identifiers for the sync run duration migration.
#[derive(DeriveIden)]
enum SyncRuns {
    /// Recorded sync runs.
    Table,
    /// Run duration in milliseconds.
    DurationMs,
}
