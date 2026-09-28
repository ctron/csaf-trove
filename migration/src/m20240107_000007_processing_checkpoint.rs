use sea_orm_migration::prelude::*;

/// Records the last completely processed provider snapshot.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Creates a singleton checkpoint alongside the document database.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared("CREATE TABLE processing_checkpoint (id INTEGER PRIMARY KEY CHECK (id = 1), commit_id TEXT NOT NULL, fingerprint TEXT NOT NULL, summary_dirty INTEGER NOT NULL DEFAULT 0)").await?;
        manager.get_connection().execute_unprepared("CREATE INDEX idx_documents_url ON documents(url); CREATE INDEX idx_documents_retrieval_errors ON documents(url) WHERE retrieval_error IS NOT NULL").await?;
        Ok(())
    }

    /// Removes processing checkpoints without changing documents.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared("DROP INDEX idx_documents_retrieval_errors; DROP INDEX idx_documents_url; DROP TABLE processing_checkpoint").await?;
        Ok(())
    }
}
