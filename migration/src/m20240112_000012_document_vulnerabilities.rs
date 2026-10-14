use sea_orm_migration::prelude::*;

/// Indexes the vulnerabilities of each document for cross-provider CVE lookups.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Creates the vulnerability table; it is filled by the next revalidation.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE document_vulnerabilities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    document_id INTEGER NOT NULL,
                    cve TEXT,
                    ids TEXT NOT NULL DEFAULT '[]',
                    title TEXT,
                    score REAL,
                    severity TEXT
                );
                CREATE INDEX idx_document_vulnerabilities_cve ON document_vulnerabilities(cve);
                CREATE INDEX idx_document_vulnerabilities_document ON document_vulnerabilities(document_id)",
            )
            .await?;
        Ok(())
    }

    /// Removes the vulnerability index.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX idx_document_vulnerabilities_document;
                DROP INDEX idx_document_vulnerabilities_cve;
                DROP TABLE document_vulnerabilities",
            )
            .await?;
        Ok(())
    }
}
