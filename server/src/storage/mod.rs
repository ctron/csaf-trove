pub mod documents;
pub mod git_repo;
pub mod results;
pub mod state;

use std::path::{Path, PathBuf};

use crate::models::{
    metrics::MetricsTimeSeries,
    result::{
        DiffLineInfo, DocumentValidation, DocumentVersionInfo, HistoricalDocument,
        PaginatedDocuments, ProviderDetail, ProviderSummary, RevisionEntry,
    },
    source::sanitize_domain,
    state::SyncState,
};
use anyhow::Result;

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

    /// Returns the combined summary, metrics, and history for a provider.
    pub async fn provider_detail(&self, domain: &str) -> Result<Option<ProviderDetail>> {
        let summary = results::load_summary(&self.results_dir, domain).await?;
        let metrics = self.load_metrics(domain).await.ok();
        let history = self.provider_history(domain)?.unwrap_or_default();

        let Some(summary) = summary else {
            return Ok(None);
        };

        Ok(Some(ProviderDetail {
            summary,
            metrics,
            history,
        }))
    }

    /// Returns recent sync run history for a provider from the database.
    pub fn provider_history(
        &self,
        domain: &str,
    ) -> Result<Option<Vec<csaf_trove_common::CommitInfo>>> {
        let runs = documents::load_sync_runs(&self.results_dir, domain, 50)?;
        if runs.is_empty() {
            return Ok(None);
        }
        Ok(Some(runs))
    }

    /// Records a completed sync run with the number of documents changed.
    pub fn save_sync_run(
        &self,
        domain: &str,
        timestamp: &chrono::DateTime<chrono::Utc>,
        documents_changed: u64,
    ) -> Result<()> {
        documents::save_sync_run(&self.results_dir, domain, timestamp, documents_changed)
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

    /// Returns the number of documents stored for a provider.
    pub fn document_count(&self, domain: &str) -> Result<u64> {
        documents::document_count(&self.results_dir, domain)
    }

    /// Upserts per-document validation results, returning the total document count.
    pub fn save_documents(&self, domain: &str, docs: &[DocumentValidation]) -> Result<u64> {
        documents::save_documents(&self.results_dir, domain, docs)
    }

    /// Builds a provider summary from all documents in the database.
    pub fn build_summary_from_db(&self, domain: &str) -> Result<ProviderSummary> {
        documents::build_summary_from_db(&self.results_dir, domain)
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
        let Some(url) = documents::document_url(&self.results_dir, domain, tracking_id)? else {
            return Ok(None);
        };
        let versions = git_repo::document_versions(&repo_path, &url, 50)?;
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

    /// Computes a structured diff between a document version and its next newer version.
    pub fn diff_document_versions(
        &self,
        domain: &str,
        tracking_id: &str,
        commit_id: &str,
    ) -> Result<Option<Vec<DiffLineInfo>>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            tracing::debug!("diff: repo not found for {domain}");
            return Ok(None);
        }
        let Some(url) = documents::document_url(&self.results_dir, domain, tracking_id)? else {
            tracing::debug!("diff: no URL for {domain}/{tracking_id}");
            return Ok(None);
        };
        let Some(versions) = git_repo::document_versions(&repo_path, &url, 50)? else {
            tracing::debug!("diff: no versions for {domain}/{tracking_id}");
            return Ok(None);
        };
        let Some(pos) = versions.iter().position(|v| v.commit_id == commit_id) else {
            tracing::debug!(
                "diff: commit {commit_id} not in {} versions for {domain}/{tracking_id}",
                versions.len()
            );
            return Ok(None);
        };
        if pos == 0 {
            tracing::debug!("diff: {commit_id} is the latest version, no newer version to diff");
            return Ok(None);
        }
        let new_commit_id = &versions[pos - 1].commit_id;
        git_repo::diff_document_versions(&repo_path, &url, commit_id, new_commit_id)
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
        let Some(url) = documents::document_url(&self.results_dir, domain, tracking_id)? else {
            return Ok(None);
        };
        let Some((blob, timestamp)) = git_repo::read_document_blob(&repo_path, &url, commit_id)?
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

    let revision_history = tracking
        .get("revision_history")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    let number = entry.get("number").and_then(|v| {
                        v.as_str()
                            .map(String::from)
                            .or_else(|| v.as_i64().map(|n| n.to_string()))
                    })?;
                    let date = entry.get("date")?.as_str()?.to_string();
                    let summary = entry.get("summary")?.as_str()?.to_string();
                    Some(RevisionEntry {
                        number,
                        date,
                        summary,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

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
        revision_history,
        commit_id: commit_id.to_string(),
        timestamp,
    })
}
