use sea_orm_migration::prelude::*;

/// Essential check stages stored in the `document_checks` JSON.
const STAGES: [&str; 4] = ["retrieval", "parsing", "signature", "digest"];

/// Indexes essential check and overall outcomes for document filtering.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Exposes each stage status and the overall outcome as virtual columns, each indexed
    /// together with the listing order, so filters no longer parse the JSON of every document.
    ///
    /// Overall outcomes are exclusive and checked in order: `failing` (a failed essential check,
    /// a missing document, or profile errors), `warning` (a digest warning or profile warnings),
    /// `passing` (retrieved and parsed, with passed or absent integrity inputs), otherwise `NULL`.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for stage in STAGES {
            db.execute_unprepared(&format!(
                "ALTER TABLE documents ADD COLUMN {stage}_status TEXT GENERATED ALWAYS AS
                    (COALESCE(json_extract(document_checks, '$.{stage}.status'), 'not_evaluated')) VIRTUAL"
            ))
            .await?;
            db.execute_unprepared(&format!(
                "CREATE INDEX idx_documents_{stage}_status ON documents({stage}_status, tracking_id)"
            ))
            .await?;
        }
        db.execute_unprepared(
            "ALTER TABLE documents ADD COLUMN overall_status TEXT GENERATED ALWAYS AS (CASE
                WHEN retrieval_status IN ('failed', 'missing') OR parsing_status = 'failed'
                    OR signature_status = 'failed' OR digest_status = 'failed'
                    OR (retrieval_status = 'passed' AND parsing_status = 'passed'
                        AND (basic_error_count > 0 OR extended_error_count > 0 OR full_error_count > 0))
                    THEN 'failing'
                WHEN digest_status = 'warning'
                    OR (retrieval_status = 'passed' AND parsing_status = 'passed'
                        AND (basic_warning_count > 0 OR extended_warning_count > 0 OR full_warning_count > 0))
                    THEN 'warning'
                WHEN retrieval_status = 'passed' AND parsing_status = 'passed'
                    AND signature_status IN ('passed', 'missing') AND digest_status IN ('passed', 'missing')
                    THEN 'passing'
            END) VIRTUAL",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_documents_overall_status ON documents(overall_status, tracking_id)",
        )
        .await?;
        Ok(())
    }

    /// Removes the indexes and their virtual columns.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP INDEX idx_documents_overall_status")
            .await?;
        db.execute_unprepared("ALTER TABLE documents DROP COLUMN overall_status")
            .await?;
        for stage in STAGES {
            db.execute_unprepared(&format!("DROP INDEX idx_documents_{stage}_status"))
                .await?;
            db.execute_unprepared(&format!("ALTER TABLE documents DROP COLUMN {stage}_status"))
                .await?;
        }
        Ok(())
    }
}
