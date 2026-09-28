use sea_orm_migration::prelude::*;

/// Separates essential check outcomes while conservatively converting legacy results.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Adds outcomes and recovers only facts recorded by the legacy pipeline.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE documents ADD COLUMN document_checks TEXT NOT NULL DEFAULT '{}'",
        )
        .await?;
        db.execute_unprepared(r#"
            UPDATE documents SET document_checks = json_object(
                'retrieval', json_object('status', CASE WHEN retrieval_error IS NOT NULL THEN 'failed' ELSE 'passed' END, 'message', retrieval_error),
                'parsing', json_object('status', CASE
                    WHEN retrieval_error IS NOT NULL THEN 'not_evaluated'
                    WHEN signature_error LIKE 'Document error: document parsing error:%' THEN 'failed'
                    WHEN signature_error LIKE 'Document error: %' THEN 'not_evaluated'
                    WHEN basic_passed IS NOT NULL OR csaf_version IS NOT NULL THEN 'passed'
                    ELSE 'not_evaluated' END,
                    'message', CASE WHEN retrieval_error IS NULL AND signature_error LIKE 'Document error: %' THEN signature_error END),
                'signature', json_object('status', CASE
                    WHEN retrieval_error IS NOT NULL OR signature_error LIKE 'Document error: %' THEN 'not_evaluated'
                    WHEN signature_error LIKE 'Invalid signature: %' THEN 'failed'
                    WHEN signature_present = 0 THEN 'missing'
                    ELSE 'not_evaluated' END,
                    'message', CASE
                        WHEN retrieval_error IS NOT NULL OR signature_error LIKE 'Document error: %' THEN NULL
                        WHEN signature_error LIKE 'Invalid signature: %' THEN signature_error
                        WHEN signature_present = 1 THEN 'Revalidate to determine the signature outcome. Previous combined result: ' || COALESCE(signature_error, 'no error recorded') END),
                'digest', json_object('status', CASE
                    WHEN retrieval_error IS NOT NULL OR signature_error LIKE 'Document error: %' THEN 'not_evaluated'
                    WHEN signature_error LIKE '%SHA-256 mismatch:%' OR signature_error LIKE '%SHA-512 mismatch:%' THEN 'failed'
                    WHEN signature_warning IS NOT NULL THEN 'warning'
                    WHEN signature_present = 0 THEN 'missing'
                    ELSE 'not_evaluated' END,
                    'message', CASE WHEN retrieval_error IS NULL AND signature_error NOT LIKE 'Document error: %'
                        AND (signature_error LIKE '%SHA-256 mismatch:%' OR signature_error LIKE '%SHA-512 mismatch:%') THEN signature_error
                        WHEN retrieval_error IS NULL THEN signature_warning END)
            );
            UPDATE processing_checkpoint SET summary_dirty = 1 WHERE id = 1;
        "#).await?;
        Ok(())
    }

    /// Removes the independent outcomes, preserving legacy fields for older binaries.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE documents DROP COLUMN document_checks")
            .await?;
        Ok(())
    }
}
