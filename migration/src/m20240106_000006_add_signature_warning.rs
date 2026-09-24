use sea_orm_migration::prelude::*;

/// Stores tolerated checksum failures separately from errors.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Adds nullable warnings without changing existing validation results.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Documents::Table)
                    .add_column(ColumnDef::new(Documents::SignatureWarning).string())
                    .to_owned(),
            )
            .await
    }

    /// Removes the warning field.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Documents::Table)
                    .drop_column(Documents::SignatureWarning)
                    .to_owned(),
            )
            .await
    }
}

/// Identifiers for the document warning migration.
#[derive(DeriveIden)]
enum Documents {
    /// Stored document results.
    Table,
    /// Tolerated integrity failures.
    SignatureWarning,
}
