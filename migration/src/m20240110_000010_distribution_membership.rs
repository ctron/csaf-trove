use sea_orm_migration::prelude::*;

/// Records exact distribution membership independently of document retrieval.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Creates a unique mapping usable by distribution health queries.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE distribution_membership (
                distribution_url TEXT NOT NULL,
                document_url TEXT NOT NULL,
                PRIMARY KEY (distribution_url, document_url)
            )",
            )
            .await?;
        Ok(())
    }

    /// Removes the mapping without affecting stored advisories.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE distribution_membership")
            .await?;
        Ok(())
    }
}
