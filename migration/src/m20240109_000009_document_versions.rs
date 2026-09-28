use sea_orm_migration::prelude::*;

/// Records each distinct version of a document found in the provider's git history.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Creates the version table; it is backfilled from git on the next sync.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE document_versions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    url TEXT NOT NULL,
                    commit_id TEXT NOT NULL,
                    timestamp BIGINT NOT NULL,
                    message TEXT NOT NULL,
                    blob_id TEXT NOT NULL,
                    status TEXT,
                    version TEXT,
                    current_release_date TEXT
                );
                CREATE UNIQUE INDEX idx_document_versions_url_commit ON document_versions(url, commit_id);
                CREATE INDEX idx_document_versions_url_id ON document_versions(url, id)",
            )
            .await?;
        Ok(())
    }

    /// Removes recorded versions; git history remains the source of truth.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX idx_document_versions_url_id;
                DROP INDEX idx_document_versions_url_commit;
                DROP TABLE document_versions",
            )
            .await?;
        Ok(())
    }
}
