//! In-memory caches for data spanning all providers.
#[cfg(test)]
pub(crate) mod test;

use super::{provider::ProviderDir, results};
use crate::models::result::{CveSearchResult, ProviderSummary};
use anyhow::{Result, anyhow};
use moka::future::Cache;
use serde::Deserialize;
use std::{collections::BTreeMap, future::Future, path::Path, sync::Arc, time::Duration};
use tokio::sync::RwLock;

/// Cache settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Upper bound of the CVE search cache, weighed as one unit per entry plus one per hit.
    pub cve_search_capacity: u64,
    /// Time after which a cached CVE search result expires, even without a sync.
    #[serde(with = "humantime_serde")]
    pub cve_search_ttl: Duration,
    /// Maximum number of hits returned (and cached) for a single CVE search.
    pub cve_search_max_hits: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            cve_search_capacity: 50_000,
            cve_search_ttl: Duration::from_secs(15 * 60),
            cve_search_max_hits: 500,
        }
    }
}

/// Bounded, expiring cache of cross-provider CVE search results.
pub struct CveSearchCache {
    /// Results keyed by match mode and normalized query.
    cache: Cache<String, Arc<CveSearchResult>>,
    /// Maximum number of hits per result.
    pub max_hits: usize,
}

impl CveSearchCache {
    /// Creates a cache from its settings.
    pub fn new(config: &CacheConfig) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(config.cve_search_capacity)
                .weigher(|_key, value: &Arc<CveSearchResult>| {
                    u32::try_from(value.hits.len().saturating_add(1)).unwrap_or(u32::MAX)
                })
                .time_to_live(config.cve_search_ttl)
                .build(),
            max_hits: config.cve_search_max_hits,
        }
    }

    /// Returns the cached result for the key, or computes and caches it; errors are not cached.
    ///
    /// Concurrent requests for the same key share a single computation.
    pub async fn get_or_compute(
        &self,
        key: String,
        compute: impl Future<Output = Result<CveSearchResult>>,
    ) -> Result<Arc<CveSearchResult>> {
        self.cache
            .try_get_with(key, async { compute.await.map(Arc::new) })
            .await
            .map_err(|e| anyhow!("{e:#}"))
    }

    /// Drops all cached results.
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }

    /// Applies pending evictions and returns the number of cached results.
    #[cfg(test)]
    pub async fn entry_count(&self) -> u64 {
        self.cache.run_pending_tasks().await;
        self.cache.entry_count()
    }
}

/// In-memory index of every provider's `summary.json`, loaded on first use.
#[derive(Default)]
pub struct SummaryIndex {
    /// Summaries keyed by provider domain; `None` until first loaded from disk.
    entries: RwLock<Option<BTreeMap<String, Arc<ProviderSummary>>>>,
}

impl SummaryIndex {
    /// Returns all summaries sorted by domain, loading them from disk on first use.
    pub async fn all(&self, results_root: &Path) -> Result<Vec<ProviderSummary>> {
        if let Some(entries) = self.entries.read().await.as_ref() {
            return Ok(entries.values().map(|s| (**s).clone()).collect());
        }

        let mut entries = self.entries.write().await;
        let loaded = match entries.as_ref() {
            Some(loaded) => loaded,
            None => entries.insert(Self::load(results_root).await?),
        };
        Ok(loaded.values().map(|s| (**s).clone()).collect())
    }

    /// Returns the domains of all providers with a summary.
    pub async fn domains(&self, results_root: &Path) -> Result<Vec<String>> {
        Ok(self
            .all(results_root)
            .await?
            .into_iter()
            .map(|s| s.provider)
            .collect())
    }

    /// Replaces a provider's summary if the index is loaded.
    pub async fn update(&self, domain: &str, summary: &ProviderSummary) {
        if let Some(entries) = self.entries.write().await.as_mut() {
            entries.insert(domain.to_string(), Arc::new(summary.clone()));
        }
    }

    /// Removes a provider from the index if it is loaded.
    pub async fn remove(&self, domain: &str) {
        if let Some(entries) = self.entries.write().await.as_mut() {
            entries.remove(domain);
        }
    }

    /// Re-reads a provider's summary from disk if the index is loaded.
    pub async fn reload(&self, domain: &str, dir: &ProviderDir) -> Result<()> {
        let mut entries = self.entries.write().await;
        let Some(entries) = entries.as_mut() else {
            return Ok(());
        };
        match dir.read_json::<ProviderSummary>("summary.json").await? {
            Some(summary) => entries.insert(domain.to_string(), Arc::new(summary)),
            None => entries.remove(domain),
        };
        Ok(())
    }

    /// Reads all summaries from the results directory.
    async fn load(results_root: &Path) -> Result<BTreeMap<String, Arc<ProviderSummary>>> {
        Ok(results::list_summaries(results_root)
            .await?
            .into_iter()
            .map(|s| (s.provider.clone(), Arc::new(s)))
            .collect())
    }
}
