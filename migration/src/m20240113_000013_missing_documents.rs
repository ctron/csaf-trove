use sea_orm_migration::prelude::*;

/// Tracks documents the provider reports as not found.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Adds a nullable per-run count and reclassifies stored 404 retrieval failures as missing.
    ///
    /// Incremental syncs do not retry failed documents, so existing failures would otherwise
    /// keep their old classification.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SyncRuns::Table)
                    .add_column(ColumnDef::new(SyncRuns::DocumentsMissing).big_integer())
                    .to_owned(),
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE documents
                 SET document_checks = json_set(document_checks, '$.retrieval.status', 'missing')
                 WHERE json_extract(document_checks, '$.retrieval.status') = 'failed'
                   AND retrieval_error LIKE '%Client error: 404%'",
            )
            .await?;
        Ok(())
    }

    /// Restores missing documents to plain retrieval failures and removes the per-run count.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE documents
                 SET document_checks = json_set(document_checks, '$.retrieval.status', 'failed')
                 WHERE json_extract(document_checks, '$.retrieval.status') = 'missing'",
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(SyncRuns::Table)
                    .drop_column(SyncRuns::DocumentsMissing)
                    .to_owned(),
            )
            .await
    }
}

/// Identifiers for the missing documents migration.
#[derive(DeriveIden)]
enum SyncRuns {
    /// Recorded sync runs.
    Table,
    /// Number of documents reported as not found during the run.
    DocumentsMissing,
}
