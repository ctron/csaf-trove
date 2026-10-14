pub mod cache;
pub mod content;
pub mod db;
pub mod documents;
mod git_changes;
pub mod git_maintenance;
pub mod git_processing;
pub mod git_repo;
mod membership;
pub mod overview;
mod path_set;
pub mod processing;
pub mod provider;
pub mod results;
pub mod scratch;
pub mod state;
mod usage;
pub mod versions;
use crate::models::{
    metrics::MetricsTimeSeries,
    result::{
        CveSearchResult, DistributionHealth, DocumentValidation, HistoricalDocument,
        ProviderDetail, ProviderSummary, RevisionEntry,
    },
    state::SyncState,
};
use crate::pipeline::sync::RetrievalFailure;
use anyhow::Result;
use cache::{CacheConfig, CveSearchCache, SummaryIndex};
use csaf_trove_common::{Paginated, SyncPoint};
use db::DbPool;
pub use documents::ProviderInfo;
use futures_util::{StreamExt, TryStreamExt, stream};
use provider::{ProviderDir, ProviderKey, read_json, write_json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::{Duration, OffsetDateTime};

/// Manages on-disk persistence for repos, state, results, and metrics.
pub struct Storage {
    /// Directory containing bare git repos per provider.
    repos_root: PathBuf,
    /// Directory containing per-provider sync state.
    state_root: PathBuf,
    /// Directory containing per-provider validation summaries.
    results_root: PathBuf,
    /// Directory containing per-provider metrics time series.
    metrics_root: PathBuf,
    /// Per-provider SQLite connection pool.
    db: DbPool,
    /// Cached storage usage and trend baselines for the overview page.
    overview: overview::OverviewCache,
    /// In-memory index of all provider summaries.
    summaries: SummaryIndex,
    /// Cached cross-provider CVE search results.
    cve_search: CveSearchCache,
}

/// Provider databases queried concurrently by a cross-provider search.
const SEARCH_FANOUT: usize = 8;

impl Storage {
    /// Logs SQLite allocations and existing provider pool occupancy.
    pub async fn log_database_memory(&self) {
        self.db.log_memory().await;
    }

    /// Creates a new storage layer with default cache settings, ensuring all directories exist.
    pub fn new(data_dir: &Path) -> Result<Self> {
        Self::with_cache(data_dir, &CacheConfig::default())
    }

    /// Creates a new storage layer, ensuring all directories exist.
    pub fn with_cache(data_dir: &Path, cache: &CacheConfig) -> Result<Self> {
        let results_root = data_dir.join("results");
        let storage = Self {
            repos_root: data_dir.join("repos"),
            state_root: data_dir.join("state"),
            results_root: results_root.clone(),
            metrics_root: data_dir.join("metrics"),
            db: DbPool::new(results_root),
            overview: Default::default(),
            summaries: Default::default(),
            cve_search: CveSearchCache::new(cache),
        };
        std::fs::create_dir_all(&storage.repos_root)?;
        std::fs::create_dir_all(&storage.state_root)?;
        std::fs::create_dir_all(&storage.results_root)?;
        std::fs::create_dir_all(&storage.metrics_root)?;
        Ok(storage)
    }

    /// Returns a provider's sync state directory.
    fn state_dir(&self, domain: &str) -> ProviderDir {
        ProviderDir::new(&self.state_root, domain)
    }

    /// Returns a provider's results directory, which also holds its database.
    fn results_dir(&self, domain: &str) -> ProviderDir {
        ProviderDir::new(&self.results_root, domain)
    }

    /// Returns the path of a provider's metrics time series file.
    fn metrics_file(&self, domain: &str) -> PathBuf {
        self.metrics_root
            .join(format!("{}.json", ProviderKey::from(domain)))
    }

    /// Returns the path to the bare git repo for a provider.
    pub fn repo_path(&self, domain: &str) -> PathBuf {
        self.repos_root
            .join(format!("{}.git", ProviderKey::from(domain)))
    }

    /// Returns `true` if any on-disk data exists for the given provider.
    pub fn has_provider_data(&self, domain: &str) -> bool {
        self.repo_path(domain).exists()
            || self.state_dir(domain).path().exists()
            || self.results_dir(domain).path().exists()
            || self.metrics_file(domain).exists()
    }

    /// Deletes all on-disk data for a provider: repo, state, results, and metrics.
    pub async fn delete_provider(&self, domain: &str) -> Result<()> {
        let dirs = [
            self.repo_path(domain),
            self.state_dir(domain).path().to_path_buf(),
            self.results_dir(domain).path().to_path_buf(),
        ];

        for path in &dirs {
            if path.exists() {
                tokio::fs::remove_dir_all(path).await?;
            }
        }

        let metrics_path = self.metrics_file(domain);
        if metrics_path.exists() {
            tokio::fs::remove_file(&metrics_path).await?;
        }

        self.summaries.remove(domain).await;
        self.cve_search.invalidate_all();
        Ok(())
    }

    /// Lists cached provider summaries without rebuilding them, sorted by domain.
    pub async fn list_summaries(&self) -> Result<Vec<ProviderSummary>> {
        self.summaries.all(&self.results_root).await
    }

    /// Refreshes cross-provider caches after jobs for these providers finished.
    ///
    /// CVE search results can contain hits from any provider, so they are all dropped.
    pub async fn invalidate_provider_caches(&self, domains: &[String]) -> Result<()> {
        self.cve_search.invalidate_all();
        for domain in domains {
            self.summaries
                .reload(domain, &self.results_dir(domain))
                .await?;
        }
        Ok(())
    }

    /// Finds documents across all providers referencing a CVE, exactly or by prefix.
    ///
    /// Results are cached until a provider job finishes or the configured TTL expires.
    pub async fn find_by_cve(&self, cve: &str, prefix: bool) -> Result<Arc<CveSearchResult>> {
        let cve = content::normalize_cve(cve);
        let mode = if prefix { "prefix" } else { "exact" };
        let key = format!("{mode}:{cve}");
        self.cve_search
            .get_or_compute(key, self.search_cve(cve, prefix))
            .await
    }

    /// Queries every provider database for a normalized CVE, bypassing the cache.
    async fn search_cve(&self, cve: String, prefix: bool) -> Result<CveSearchResult> {
        let max_hits = self.cve_search.max_hits;
        let limit = max_hits as u64 + 1;
        let domains = self.summaries.domains(&self.results_root).await?;
        let cve = &cve;

        let mut hits: Vec<_> = stream::iter(domains)
            .map(|domain| async move {
                let Some(db) = self.db.get_if_exists(&domain).await? else {
                    return Ok(vec![]);
                };
                documents::find_by_cve(&db, &domain, cve, prefix, limit).await
            })
            .buffer_unordered(SEARCH_FANOUT)
            .try_concat()
            .await?;

        hits.sort_by(|a, b| {
            b.current_release_date
                .cmp(&a.current_release_date)
                .then_with(|| a.domain.cmp(&b.domain))
                .then_with(|| a.tracking_id.cmp(&b.tracking_id))
                .then_with(|| a.cve.cmp(&b.cve))
        });
        let truncated = hits.len() > max_hits;
        hits.truncate(max_hits);
        Ok(CveSearchResult { hits, truncated })
    }

    /// Returns the combined summary, metrics, and history for a provider.
    pub async fn provider_detail(
        &self,
        domain: &str,
        skip_directories: &[String],
    ) -> Result<Option<ProviderDetail>> {
        let summary = self.load_summary(domain).await?;
        let metrics = self.load_metrics(domain).await.ok();
        let history = self.provider_history(domain).await?.unwrap_or_default();

        let Some(summary) = summary else {
            return Ok(None);
        };

        let distributions = self
            .distribution_health(domain, skip_directories)
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

    /// Returns cached per-distribution health, computing and caching it on first access.
    ///
    /// Aggregating health reads every member document, which is far too slow for large
    /// providers to do per request; it only changes when a sync or revalidation runs.
    async fn distribution_health(
        &self,
        domain: &str,
        skip_directories: &[String],
    ) -> Result<Vec<DistributionHealth>> {
        let mut distributions = match self
            .results_dir(domain)
            .read_json("distributions.json")
            .await?
        {
            Some(distributions) => distributions,
            None => self.refresh_distribution_health(domain).await?,
        };
        for distribution in &mut distributions {
            distribution.skipped =
                distribution.kind == "directory" && skip_directories.contains(&distribution.url);
        }
        Ok(distributions)
    }

    /// Recomputes per-distribution health and caches it next to the provider summary.
    pub async fn refresh_distribution_health(
        &self,
        domain: &str,
    ) -> Result<Vec<DistributionHealth>> {
        let distributions = self.compute_distribution_health(domain, &[]).await?;
        self.results_dir(domain)
            .write_json("distributions.json", &distributions)
            .await?;
        Ok(distributions)
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

        let metadata = tokio::task::spawn_blocking(move || {
            git_repo::with_head_blob(&repo_path, "metadata/provider-metadata.json", |blob| {
                serde_json::from_slice::<serde_json::Value>(blob)
            })
        })
        .await??;

        let Some(metadata) = metadata.transpose()? else {
            return Ok(vec![]);
        };
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
            let (document_count, check_issues, basic, extended, full) =
                documents::distribution_health(&db, &entry.url).await?;

            let distribution_error = dist_errors
                .iter()
                .find(|e| e.url == entry.url)
                .map(|e| e.error.clone());

            let skipped = entry.kind == "directory" && skip_directories.contains(&entry.url);

            results.push(DistributionHealth {
                label: entry.label.clone(),
                kind: entry.kind.clone(),
                url: entry.url.clone(),
                tlp_labels: entry.tlp_labels.clone(),
                document_count,
                check_issues,
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

    /// Records a completed sync run with the number of documents changed and its duration.
    pub async fn save_sync_run(
        &self,
        domain: &str,
        timestamp: OffsetDateTime,
        documents_changed: u64,
        duration: Duration,
    ) -> Result<()> {
        let db = self.db.get(domain).await?;
        documents::save_sync_run(&db, timestamp, documents_changed, duration).await
    }

    /// Returns the duration of the most recent sync run of a provider, if recorded.
    pub async fn last_sync_duration(&self, domain: &str) -> Result<Option<Duration>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        documents::load_last_sync_duration(&db).await
    }

    /// Loads the sync state for a provider, creating a default if absent.
    pub async fn load_sync_state(&self, domain: &str) -> Result<SyncState> {
        Ok(self
            .state_dir(domain)
            .read_json("sync.json")
            .await?
            .unwrap_or_else(|| SyncState::new(domain.to_string())))
    }

    /// Lists all persisted sync states.
    pub async fn list_sync_states(&self) -> Result<Vec<SyncState>> {
        state::list_sync_states(&self.state_root).await
    }

    /// Persists the sync state for a provider.
    pub async fn save_sync_state(&self, state: &SyncState) -> Result<()> {
        self.state_dir(&state.domain)
            .write_json("sync.json", state)
            .await
    }

    /// Persists distribution-level errors for a provider.
    pub async fn save_distribution_errors(
        &self,
        domain: &str,
        errors: &[RetrievalFailure],
    ) -> Result<()> {
        self.state_dir(domain)
            .write_json("distribution-errors.json", errors)
            .await
    }

    /// Loads distribution-level errors for a provider.
    pub async fn load_distribution_errors(&self, domain: &str) -> Result<Vec<RetrievalFailure>> {
        Ok(self
            .state_dir(domain)
            .read_json("distribution-errors.json")
            .await?
            .unwrap_or_default())
    }

    /// Persists a validation summary for a provider.
    pub async fn save_summary(&self, domain: &str, summary: &ProviderSummary) -> Result<()> {
        self.results_dir(domain)
            .write_json("summary.json", summary)
            .await?;
        self.summaries.update(domain, summary).await;
        Ok(())
    }

    /// Loads the metrics time series for a provider.
    pub async fn load_metrics(&self, domain: &str) -> Result<MetricsTimeSeries> {
        Ok(read_json(&self.metrics_file(domain))
            .await?
            .unwrap_or_else(|| MetricsTimeSeries::new(domain.to_string())))
    }

    /// Persists the metrics time series for a provider.
    pub async fn save_metrics(&self, domain: &str, metrics: &MetricsTimeSeries) -> Result<()> {
        write_json(&self.metrics_file(domain), metrics).await
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
    ) -> Result<bool> {
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
        let mut entries = tokio::fs::read_dir(&self.results_root).await?;
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

/// One independently reported directory or ROLIE feed from provider metadata.
struct DistributionEntry {
    /// Display name derived from the distribution URL path.
    label: String,
    /// Distribution protocol used for display and skip handling.
    kind: String,
    /// Exact discovery URL used to look up persisted membership and errors.
    url: String,
    /// Traffic light labels advertised by this feed.
    tlp_labels: Vec<String>,
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
            let label = url::Url::parse(dir)
                .ok()
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| dir.clone());
            entries.push(DistributionEntry {
                label,
                kind: "directory".to_string(),
                url: dir.clone(),
                tlp_labels: vec![],
            });
        }

        for (feed_url, tlp) in &rolie_feeds {
            let label = url::Url::parse(feed_url)
                .ok()
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| feed_url.to_string());
            let tlp_labels = tlp.iter().map(|t| t.to_uppercase()).collect();
            entries.push(DistributionEntry {
                label,
                kind: "rolie".to_string(),
                url: feed_url.to_string(),
                tlp_labels,
            });
        }
    }

    entries
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
