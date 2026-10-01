//! Durable processing checkpoints and targeted document maintenance.
use super::{Storage, documents, results};
use crate::models::result::ProviderSummary;
use anyhow::{Context, Result};
use csaf_trove_entity::{check_failure, document, revision_history};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Statement, TransactionTrait,
};

/// Inputs of the last successfully completed processing pass.
#[derive(Debug, Clone)]
pub struct ProcessingCheckpoint {
    /// Git snapshot represented by the stored results.
    pub commit_id: String,
    /// Validator, policy and signing-key identity.
    pub fingerprint: String,
    /// Results changed after the cached summary was last successfully published.
    pub summary_dirty: bool,
}

impl Storage {
    /// Loads a checkpoint from the same database as the results.
    pub async fn processing_checkpoint(
        &self,
        domain: &str,
    ) -> Result<Option<ProcessingCheckpoint>> {
        let db = self.db.get(domain).await?;
        db.query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT commit_id, fingerprint, summary_dirty FROM processing_checkpoint WHERE id = 1",
        ))
        .await?
        .map(|row| {
            Ok(ProcessingCheckpoint {
                commit_id: row.try_get("", "commit_id")?,
                fingerprint: row.try_get("", "fingerprint")?,
                summary_dirty: row.try_get::<i64>("", "summary_dirty")? != 0,
            })
        })
        .transpose()
    }

    /// Publishes a checkpoint only after all processing has succeeded.
    pub async fn save_processing_checkpoint(
        &self,
        domain: &str,
        checkpoint: &ProcessingCheckpoint,
    ) -> Result<()> {
        let db = self.db.get(domain).await?;
        db.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO processing_checkpoint (id, commit_id, fingerprint) VALUES (1, ?, ?) ON CONFLICT(id) DO UPDATE SET commit_id = excluded.commit_id, fingerprint = excluded.fingerprint, summary_dirty = 0",
            [checkpoint.commit_id.clone().into(), checkpoint.fingerprint.clone().into()])).await?;
        Ok(())
    }

    /// Invalidates processing before a forced pass so interrupted revalidation is retried.
    pub async fn clear_processing_checkpoint(&self, domain: &str) -> Result<()> {
        self.db
            .get(domain)
            .await?
            .execute_unprepared("DELETE FROM processing_checkpoint")
            .await?;
        Ok(())
    }

    /// Reads one provider's cached summary without scanning other providers.
    pub async fn load_summary(&self, domain: &str) -> Result<Option<ProviderSummary>> {
        results::load_summary(&self.results_dir, domain).await
    }

    /// Explicitly rebuilds a cached summary, preserving its validation timestamp and operator note.
    pub async fn refresh_summary(&self, domain: &str) -> Result<()> {
        let previous = self
            .load_summary(domain)
            .await?
            .with_context(|| format!("No cached summary for {domain}"))?;
        let db = self
            .db
            .get_if_exists(domain)
            .await?
            .with_context(|| format!("No stored validation results for {domain}"))?;
        let mut summary = documents::build_summary_from_db(&db, domain).await?;
        summary.validated_at = previous.validated_at;
        summary.note = previous.note;
        self.save_summary(domain, &summary).await
    }

    /// Removes results for advisory URLs actually deleted from Git.
    pub async fn delete_document_urls(&self, domain: &str, urls: &[String]) -> Result<()> {
        if urls.is_empty() {
            return Ok(());
        }
        let db = self.db.get(domain).await?;
        let txn = db.begin().await?;
        for url in urls {
            let ids: Vec<i64> = document::Entity::find()
                .filter(document::Column::Url.eq(url))
                .select_only()
                .column(document::Column::Id)
                .into_tuple()
                .all(&txn)
                .await?;
            if ids.is_empty() {
                continue;
            }
            check_failure::Entity::delete_many()
                .filter(check_failure::Column::DocumentId.is_in(ids.clone()))
                .exec(&txn)
                .await?;
            revision_history::Entity::delete_many()
                .filter(revision_history::Column::DocumentId.is_in(ids.clone()))
                .exec(&txn)
                .await?;
            document::Entity::delete_many()
                .filter(document::Column::Id.is_in(ids))
                .exec(&txn)
                .await?;
            txn.execute_unprepared(
                "UPDATE processing_checkpoint SET summary_dirty = 1 WHERE id = 1",
            )
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Finds failed downloads that can recover even when downloaded bytes are unchanged.
    pub async fn retrieval_error_urls(&self, domain: &str, after: &str) -> Result<Vec<String>> {
        let db = self.db.get(domain).await?;
        Ok(document::Entity::find()
            .filter(document::Column::RetrievalError.is_not_null())
            .filter(document::Column::Url.gt(after))
            .select_only()
            .column(document::Column::Url)
            .order_by_asc(document::Column::Url)
            .limit(256)
            .into_tuple::<String>()
            .all(&db)
            .await?)
    }
}
