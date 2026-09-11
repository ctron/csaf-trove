pub mod documents;
pub mod git_repo;
pub mod results;
pub mod state;

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::models::{
    metrics::MetricsTimeSeries,
    result::{DocumentValidation, PaginatedDocuments, ProviderSummary},
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
        self.repos_dir.join(format!("{domain}.git"))
    }

    /// Returns `true` if any on-disk data exists for the given provider.
    pub fn has_provider_data(&self, domain: &str) -> bool {
        self.repo_path(domain).exists()
            || self.state_dir.join(domain).exists()
            || self.results_dir.join(domain).exists()
            || self.metrics_dir.join(format!("{domain}.json")).exists()
    }

    /// Deletes all on-disk data for a provider: repo, state, results, and metrics.
    pub async fn delete_provider(&self, domain: &str) -> Result<()> {
        let dirs = [
            self.repo_path(domain),
            self.state_dir.join(domain),
            self.results_dir.join(domain),
        ];

        for path in &dirs {
            if path.exists() {
                tokio::fs::remove_dir_all(path).await?;
            }
        }

        let metrics_path = self.metrics_dir.join(format!("{domain}.json"));
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
        let path = self.metrics_dir.join(format!("{domain}.json"));
        if path.exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            Ok(serde_json::from_str(&data)?)
        } else {
            Ok(MetricsTimeSeries::new(domain.to_string()))
        }
    }

    /// Persists the metrics time series for a provider.
    pub async fn save_metrics(&self, domain: &str, metrics: &MetricsTimeSeries) -> Result<()> {
        let path = self.metrics_dir.join(format!("{domain}.json"));
        let data = serde_json::to_string_pretty(metrics)?;
        tokio::fs::write(&path, data).await?;
        Ok(())
    }

    /// Replaces all per-document validation results for a provider.
    pub fn save_documents(&self, domain: &str, docs: &[DocumentValidation]) -> Result<()> {
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
}
