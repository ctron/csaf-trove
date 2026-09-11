pub mod documents;
pub mod git_repo;
pub mod results;
pub mod state;

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::models::{
    metrics::MetricsTimeSeries,
    result::{
        DocumentValidation, DocumentVersionInfo, HistoricalDocument, PaginatedDocuments,
        ProviderSummary,
    },
    source::sanitize_domain,
    state::SyncState,
};

/// Manages on-disk persistence for repos, state, results, and metrics.
pub struct Storage {
    /// Directory containing bare git repos per provider.
    repos_dir: PathBuf,
    /// Directory containing per-provider sync state.
    state_dir: PathBuf,
    /// Directory containing per-provider validation summaries.
    results_dir: PathBuf,
    /// Directory containing per-provider metrics time series.
    metrics_dir: PathBuf,
}

impl Storage {
    /// Creates a new storage layer, ensuring all directories exist.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let storage = Self {
            repos_dir: data_dir.join("repos"),
            state_dir: data_dir.join("state"),
            results_dir: data_dir.join("results"),
            metrics_dir: data_dir.join("metrics"),
        };
        std::fs::create_dir_all(&storage.repos_dir)?;
        std::fs::create_dir_all(&storage.state_dir)?;
        std::fs::create_dir_all(&storage.results_dir)?;
        std::fs::create_dir_all(&storage.metrics_dir)?;
        Ok(storage)
    }

    /// Returns the path to the bare git repo for a provider.
    pub fn repo_path(&self, domain: &str) -> PathBuf {
        let key = sanitize_domain(domain);
        self.repos_dir.join(format!("{key}.git"))
    }

    /// Returns `true` if any on-disk data exists for the given provider.
    pub fn has_provider_data(&self, domain: &str) -> bool {
        let key = sanitize_domain(domain);
        self.repo_path(domain).exists()
            || self.state_dir.join(&key).exists()
            || self.results_dir.join(&key).exists()
            || self.metrics_dir.join(format!("{key}.json")).exists()
    }

    /// Deletes all on-disk data for a provider: repo, state, results, and metrics.
    pub async fn delete_provider(&self, domain: &str) -> Result<()> {
        let key = sanitize_domain(domain);
        let dirs = [
            self.repo_path(domain),
            self.state_dir.join(&key),
            self.results_dir.join(&key),
        ];

        for path in &dirs {
            if path.exists() {
                tokio::fs::remove_dir_all(path).await?;
            }
        }

        let metrics_path = self.metrics_dir.join(format!("{key}.json"));
        if metrics_path.exists() {
            tokio::fs::remove_file(&metrics_path).await?;
        }

        Ok(())
    }

    /// Lists all stored provider summaries, sorted by domain.
    pub async fn list_summaries(&self) -> Result<Vec<ProviderSummary>> {
        results::list_summaries(&self.results_dir).await
    }

    /// Returns the combined summary and metrics for a provider.
    pub async fn provider_detail(&self, domain: &str) -> Result<Option<serde_json::Value>> {
        let summary = results::load_summary(&self.results_dir, domain).await?;
        let metrics = self.load_metrics(domain).await.ok();

        let Some(summary) = summary else {
            return Ok(None);
        };

        Ok(Some(serde_json::json!({
            "summary": summary,
            "metrics": metrics,
        })))
    }

    /// Returns recent git commit history for a provider's document repo.
    pub async fn provider_history(
        &self,
        domain: &str,
    ) -> Result<Option<Vec<git_repo::CommitInfo>>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        let history = git_repo::log(&repo_path, 50)?;
        Ok(Some(history))
    }

    /// Loads the sync state for a provider, creating a default if absent.
    pub async fn load_sync_state(&self, domain: &str) -> Result<SyncState> {
        state::load_sync_state(&self.state_dir, domain).await
    }

    /// Lists all persisted sync states.
    pub async fn list_sync_states(&self) -> Result<Vec<SyncState>> {
        state::list_sync_states(&self.state_dir).await
    }

    /// Persists the sync state for a provider.
    pub async fn save_sync_state(&self, state: &SyncState) -> Result<()> {
        state::save_sync_state(&self.state_dir, state).await
    }

    /// Persists a validation summary for a provider.
    pub async fn save_summary(&self, domain: &str, summary: &ProviderSummary) -> Result<()> {
        results::save_summary(&self.results_dir, domain, summary).await
    }

    /// Loads the metrics time series for a provider.
    pub async fn load_metrics(&self, domain: &str) -> Result<MetricsTimeSeries> {
        let key = sanitize_domain(domain);
        let path = self.metrics_dir.join(format!("{key}.json"));
        if path.exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            Ok(serde_json::from_str(&data)?)
        } else {
            Ok(MetricsTimeSeries::new(domain.to_string()))
        }
    }

    /// Persists the metrics time series for a provider.
    pub async fn save_metrics(&self, domain: &str, metrics: &MetricsTimeSeries) -> Result<()> {
        let key = sanitize_domain(domain);
        let path = self.metrics_dir.join(format!("{key}.json"));
        let data = serde_json::to_string_pretty(metrics)?;
        tokio::fs::write(&path, data).await?;
        Ok(())
    }

    /// Upserts per-document validation results, returning the total document count.
    pub fn save_documents(&self, domain: &str, docs: &[DocumentValidation]) -> Result<u64> {
        documents::save_documents(&self.results_dir, domain, docs)
    }

    /// Loads paginated document validation results for a provider.
    pub fn load_documents_paginated(
        &self,
        domain: &str,
        offset: u64,
        limit: u64,
        status_filter: Option<&str>,
    ) -> Result<Option<PaginatedDocuments>> {
        documents::load_documents_paginated(&self.results_dir, domain, offset, limit, status_filter)
    }

    /// Loads a single document's validation results by tracking ID.
    pub fn load_document(
        &self,
        domain: &str,
        tracking_id: &str,
    ) -> Result<Option<DocumentValidation>> {
        documents::load_document(&self.results_dir, domain, tracking_id)
    }

    /// Returns the version history for a specific document in a provider's repo.
    pub fn document_versions(
        &self,
        domain: &str,
        tracking_id: &str,
    ) -> Result<Option<Vec<DocumentVersionInfo>>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        let versions = git_repo::document_versions(&repo_path, tracking_id, 50)?;
        Ok(versions.map(|vs| {
            vs.into_iter()
                .map(|v| DocumentVersionInfo {
                    commit_id: v.commit_id,
                    timestamp: v.timestamp,
                    message: v.message,
                    is_latest: v.is_latest,
                })
                .collect()
        }))
    }

    /// Reads a historical version of a document from git and extracts its metadata.
    pub fn read_historical_document(
        &self,
        domain: &str,
        tracking_id: &str,
        commit_id: &str,
    ) -> Result<Option<HistoricalDocument>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        let Some((blob, timestamp)) =
            git_repo::read_document_blob(&repo_path, tracking_id, commit_id)?
        else {
            return Ok(None);
        };
        let doc = extract_metadata_from_json(&blob, commit_id, timestamp)?;
        Ok(Some(doc))
    }
}

/// Extracts document metadata from raw CSAF JSON via `serde_json::Value`.
fn extract_metadata_from_json(
    json: &[u8],
    commit_id: &str,
    timestamp: i64,
) -> Result<HistoricalDocument> {
    let val: serde_json::Value = serde_json::from_slice(json)?;
    let doc = &val["document"];
    let tracking = &doc["tracking"];

    let tracking_id = tracking["id"].as_str().unwrap_or("").to_string();
    let title = doc["title"].as_str().unwrap_or("").to_string();

    let csaf_version = if val.get("$schema").is_some() || doc.get("csaf_version").is_some() {
        doc.get("csaf_version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(Some("2.1".to_string()))
    } else {
        Some("2.0".to_string())
    };

    Ok(HistoricalDocument {
        tracking_id,
        title,
        category: doc
            .get("category")
            .and_then(|v| v.as_str())
            .map(String::from),
        publisher_name: doc
            .get("publisher")
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .map(String::from),
        initial_release_date: tracking
            .get("initial_release_date")
            .and_then(|v| v.as_str())
            .map(String::from),
        current_release_date: tracking
            .get("current_release_date")
            .and_then(|v| v.as_str())
            .map(String::from),
        status: tracking
            .get("status")
            .and_then(|v| v.as_str())
            .map(String::from),
        revision: tracking
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from),
        aggregate_severity: doc
            .get("aggregate_severity")
            .and_then(|s| s.get("text"))
            .and_then(|v| v.as_str())
            .map(String::from),
        csaf_version,
        commit_id: commit_id.to_string(),
        timestamp,
    })
}
