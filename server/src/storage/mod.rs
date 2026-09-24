pub mod db;
pub mod documents;
pub mod git_repo;
pub mod results;
pub mod scratch;
pub mod state;

use crate::models::{
    metrics::MetricsTimeSeries,
    result::{
        DiffLineInfo, DistributionHealth, DocumentValidation, DocumentVersionInfo,
        HistoricalDocument, ProviderDetail, ProviderSummary, RevisionEntry,
    },
    source::sanitize_domain,
    state::SyncState,
};
use anyhow::Result;
use csaf_trove_common::{Paginated, SyncPoint};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

pub use documents::ProviderInfo;

use db::DbPool;

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
    /// Per-provider SQLite connection pool.
    db: DbPool,
}

impl Storage {
    /// Creates a new storage layer, ensuring all directories exist.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let results_dir = data_dir.join("results");
        let storage = Self {
            repos_dir: data_dir.join("repos"),
            state_dir: data_dir.join("state"),
            results_dir: results_dir.clone(),
            metrics_dir: data_dir.join("metrics"),
            db: DbPool::new(results_dir),
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
    pub async fn provider_detail(
        &self,
        domain: &str,
        skip_directories: &[String],
    ) -> Result<Option<ProviderDetail>> {
        let summary = results::load_summary(&self.results_dir, domain).await?;
        let metrics = self.load_metrics(domain).await.ok();
        let history = self.provider_history(domain).await?.unwrap_or_default();

        let Some(summary) = summary else {
            return Ok(None);
        };

        let distributions = self
            .compute_distribution_health(domain, skip_directories)
            .await
            .unwrap_or_default();

        Ok(Some(ProviderDetail {
            summary,
            metrics,
            history,
            distributions,
            note: None,
        }))
    }

    /// Reads provider-metadata.json from the bare repo and computes per-distribution health.
    async fn compute_distribution_health(
        &self,
        domain: &str,
        skip_directories: &[String],
    ) -> Result<Vec<DistributionHealth>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(vec![]);
        }

        let blob = tokio::task::spawn_blocking(move || {
            git_repo::read_head_blob(&repo_path, "metadata/provider-metadata.json")
        })
        .await??;

        let Some(blob) = blob else {
            return Ok(vec![]);
        };

        let metadata: serde_json::Value = serde_json::from_slice(&blob)?;
        let dist_array = match metadata.get("distributions").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return Ok(vec![]),
        };

        let entries = extract_distribution_entries(dist_array);
        if entries.is_empty() {
            return Ok(vec![]);
        }

        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(vec![]);
        };

        let dist_errors = self
            .load_distribution_errors(domain)
            .await
            .unwrap_or_default();

        let mut results = Vec::new();
        for entry in &entries {
            let (document_count, retrieval_errors, basic, extended, full) =
                documents::distribution_health(&db, &entry.prefix).await?;

            let distribution_error = dist_errors
                .iter()
                .find(|e| entry.feed_urls.iter().any(|u| u == &e.url) || e.url == entry.url)
                .map(|e| e.error.clone());

            let skipped = entry.kind == "directory" && skip_directories.contains(&entry.url);

            results.push(DistributionHealth {
                label: entry.label.clone(),
                kind: entry.kind.clone(),
                url: entry.url.clone(),
                tlp_labels: entry.tlp_labels.clone(),
                document_count,
                retrieval_errors,
                skipped,
                distribution_error,
                basic_pass_rate: basic,
                extended_pass_rate: extended,
                full_pass_rate: full,
            });
        }

        Ok(results)
    }

    /// Returns recent sync run history for a provider from the database.
    pub async fn provider_history(
        &self,
        domain: &str,
    ) -> Result<Option<Vec<csaf_trove_common::CommitInfo>>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        let runs = documents::load_sync_runs(&db, 50).await?;
        if runs.is_empty() {
            return Ok(None);
        }
        Ok(Some(runs))
    }

    /// Returns the last N sync points for a provider in chronological order.
    pub async fn recent_sync_points(
        &self,
        domain: &str,
        limit: u64,
    ) -> Result<Option<Vec<SyncPoint>>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        let points = documents::load_recent_sync_points(&db, limit).await?;
        if points.is_empty() {
            return Ok(None);
        }
        Ok(Some(points))
    }

    /// Records a completed sync run with the number of documents changed.
    pub async fn save_sync_run(
        &self,
        domain: &str,
        timestamp: OffsetDateTime,
        documents_changed: u64,
    ) -> Result<()> {
        let db = self.db.get(domain).await?;
        documents::save_sync_run(&db, timestamp, documents_changed).await
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

    /// Persists distribution-level errors for a provider.
    pub async fn save_distribution_errors(
        &self,
        domain: &str,
        errors: &[crate::pipeline::sync::RetrievalFailure],
    ) -> Result<()> {
        state::save_distribution_errors(&self.state_dir, domain, errors).await
    }

    /// Loads distribution-level errors for a provider.
    pub async fn load_distribution_errors(
        &self,
        domain: &str,
    ) -> Result<Vec<crate::pipeline::sync::RetrievalFailure>> {
        state::load_distribution_errors(&self.state_dir, domain).await
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

    /// Deletes all document rows for a provider (used before full re-validation).
    pub async fn delete_all_documents(&self, domain: &str) -> Result<()> {
        let db = self.db.get(domain).await?;
        documents::delete_all_documents(&db).await
    }

    /// Returns the number of documents stored for a provider.
    pub async fn document_count(&self, domain: &str) -> Result<u64> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(0);
        };
        documents::document_count(&db).await
    }

    /// Upserts per-document validation results, returning the total document count.
    pub async fn save_documents(&self, domain: &str, docs: &[DocumentValidation]) -> Result<u64> {
        let db = self.db.get(domain).await?;
        documents::save_documents(&db, docs).await
    }

    /// Computes version counts from git history and updates all documents.
    pub async fn update_version_counts(&self, domain: &str) -> Result<()> {
        let db = self.db.get(domain).await?;
        let repo_path = self.repo_path(domain);
        documents::update_version_counts(&db, &repo_path).await
    }

    /// Builds a provider summary from all documents in the database.
    pub async fn build_summary_from_db(&self, domain: &str) -> Result<ProviderSummary> {
        let db = self.db.get(domain).await?;
        documents::build_summary_from_db(&db, domain).await
    }

    /// Returns paginated sync run history for a provider from the database.
    pub async fn provider_history_paginated(
        &self,
        domain: &str,
        offset: u64,
        limit: u64,
    ) -> Result<Option<Paginated<csaf_trove_common::CommitInfo>>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(Some(Paginated {
                items: vec![],
                total: 0,
                offset,
                limit,
            }));
        };
        let page = documents::load_sync_runs_paginated(&db, offset, limit).await?;
        Ok(Some(page))
    }

    /// Loads paginated document validation results for a provider.
    pub async fn load_documents_paginated(
        &self,
        domain: &str,
        offset: u64,
        limit: u64,
        status_filter: Option<&str>,
    ) -> Result<Option<Paginated<DocumentValidation>>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(Some(Paginated {
                items: vec![],
                total: 0,
                offset,
                limit,
            }));
        };

        let page = documents::load_documents_paginated(&db, offset, limit, status_filter).await?;
        Ok(Some(page))
    }

    /// Persists retrieval errors as document entries with error information.
    pub async fn save_retrieval_errors(
        &self,
        domain: &str,
        errors: &[(String, String)],
    ) -> Result<()> {
        let db = self.db.get(domain).await?;
        documents::save_retrieval_errors(&db, errors).await
    }

    /// Loads a single document's validation results by tracking ID.
    pub async fn load_document(
        &self,
        domain: &str,
        tracking_id: &str,
    ) -> Result<Option<DocumentValidation>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        documents::load_document(&db, tracking_id).await
    }

    /// Returns the version history for a specific document in a provider's repo.
    pub async fn document_versions(
        &self,
        domain: &str,
        tracking_id: &str,
    ) -> Result<Option<Vec<DocumentVersionInfo>>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        let Some(url) = documents::document_url(&db, tracking_id).await? else {
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
    pub async fn diff_document_versions(
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
        let Some(db) = self.db.get_if_exists(domain).await? else {
            tracing::debug!("diff: no db for {domain}");
            return Ok(None);
        };
        let Some(url) = documents::document_url(&db, tracking_id).await? else {
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
    pub async fn read_historical_document(
        &self,
        domain: &str,
        tracking_id: &str,
        commit_id: &str,
    ) -> Result<Option<HistoricalDocument>> {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        let Some(url) = documents::document_url(&db, tracking_id).await? else {
            return Ok(None);
        };
        let Some((blob, timestamp)) = git_repo::read_document_blob(&repo_path, &url, commit_id)?
        else {
            return Ok(None);
        };
        let doc = extract_metadata_from_json(&blob, commit_id, timestamp)?;
        Ok(Some(doc))
    }

    /// Saves provider metadata info for aggregator generation.
    pub async fn save_provider_info(&self, domain: &str, info: &ProviderInfo) -> Result<()> {
        let db = self.db.get(domain).await?;
        documents::save_provider_info(&db, info).await
    }

    /// Loads provider metadata info for a single provider.
    pub async fn load_provider_info(&self, domain: &str) -> Result<Option<ProviderInfo>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        documents::load_provider_info(&db).await
    }

    /// Loads provider metadata info for all providers that have results directories.
    pub async fn load_all_provider_info(&self) -> Result<Vec<(String, ProviderInfo)>> {
        let mut result = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.results_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.path().is_dir() {
                continue;
            }
            let key = entry.file_name().to_string_lossy().to_string();
            if let Ok(Some(db)) = self.db.get_if_exists(&key).await
                && let Ok(Some(info)) = documents::load_provider_info(&db).await
            {
                result.push((key, info));
            }
        }
        Ok(result)
    }
}

struct DistributionEntry {
    label: String,
    kind: String,
    url: String,
    prefix: String,
    tlp_labels: Vec<String>,
    feed_urls: Vec<String>,
}

/// Extracts distribution entries from the `distributions` array in provider-metadata.json.
///
/// Each ROLIE feed and each directory URL becomes its own entry so that
/// per-feed errors and TLP labels are shown individually in the dashboard.
fn extract_distribution_entries(distributions: &[serde_json::Value]) -> Vec<DistributionEntry> {
    let mut entries = Vec::new();

    for dist in distributions {
        let dir_url = dist
            .get("directory_url")
            .and_then(|v| v.as_str())
            .map(String::from);

        let rolie_feeds: Vec<(&str, Option<&str>)> = dist
            .get("rolie")
            .and_then(|r| r.get("feeds"))
            .and_then(|f| f.as_array())
            .map(|feeds| {
                feeds
                    .iter()
                    .filter_map(|f| {
                        let url = f.get("url").and_then(|u| u.as_str())?;
                        let tlp = f.get("tlp_label").and_then(|t| t.as_str());
                        Some((url, tlp))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if let Some(ref dir) = dir_url {
            let prefix = normalize_url_prefix(dir);
            let label = url::Url::parse(dir)
                .ok()
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| dir.clone());
            entries.push(DistributionEntry {
                label,
                kind: "directory".to_string(),
                url: dir.clone(),
                prefix,
                tlp_labels: vec![],
                feed_urls: vec![],
            });
        }

        for (feed_url, tlp) in &rolie_feeds {
            let prefix = rolie_feed_to_prefix(feed_url);
            let label = url::Url::parse(feed_url)
                .ok()
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| feed_url.to_string());
            let tlp_labels = tlp.iter().map(|t| t.to_uppercase()).collect();
            entries.push(DistributionEntry {
                label,
                kind: "rolie".to_string(),
                url: feed_url.to_string(),
                prefix,
                tlp_labels,
                feed_urls: vec![feed_url.to_string()],
            });
        }
    }

    entries
}

/// Normalizes a directory URL to use as a LIKE prefix, ensuring trailing slash.
fn normalize_url_prefix(url: &str) -> String {
    if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{url}/")
    }
}

/// Derives a URL prefix from a ROLIE feed URL by taking the parent directory.
fn rolie_feed_to_prefix(feed_url: &str) -> String {
    if let Some(pos) = feed_url.rfind('/') {
        let prefix = &feed_url[..=pos];
        prefix.to_string()
    } else {
        feed_url.to_string()
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
