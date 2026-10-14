//! In-memory cache for overview data that only changes when a provider job finishes.

use super::Storage;
use crate::models::metrics::{MetricsEntry, MetricsTimeSeries};
use anyhow::Result;
use chrono::{Days, NaiveDate, Utc};
use csaf_trove_common::overview::StorageUsage;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Mutex;

/// Expensive overview inputs: a storage walk and every provider's metrics history.
#[derive(Debug)]
pub struct OverviewSnapshot {
    /// Cache generation this snapshot was computed for.
    generation: u64,
    /// Day the trend baselines were selected for.
    date: NaiveDate,
    /// Measured persistent storage.
    pub storage: StorageUsage,
    /// Trend baselines keyed by provider domain.
    pub baselines: HashMap<String, TrendBaselines>,
}

/// A provider's latest daily snapshots old enough for each trend window.
#[derive(Debug, Default)]
pub struct TrendBaselines {
    /// Latest snapshot at least 7 days old.
    pub week: Option<MetricsEntry>,
    /// Latest snapshot at least 30 days old.
    pub month: Option<MetricsEntry>,
}

/// Caches the overview snapshot until invalidated or the day changes.
#[derive(Debug, Default)]
pub struct OverviewCache {
    /// Incremented on every invalidation.
    generation: AtomicU64,
    /// Latest snapshot; the lock also ensures only one computation runs at a time.
    snapshot: Mutex<Option<Arc<OverviewSnapshot>>>,
}

impl Storage {
    /// Returns the cached overview snapshot, recomputing it when stale.
    pub async fn overview_snapshot(&self) -> Result<Arc<OverviewSnapshot>> {
        let mut cached = self.overview.snapshot.lock().await;
        let generation = self.overview.generation.load(Ordering::Acquire);
        let today = Utc::now().date_naive();
        if let Some(snapshot) = cached
            .as_ref()
            .filter(|s| s.generation == generation && s.date == today)
        {
            return Ok(snapshot.clone());
        }

        let storage = self.storage_usage().await?;
        let metrics_root = self.metrics_root.clone();
        let baselines =
            tokio::task::spawn_blocking(move || load_baselines(&metrics_root, today)).await??;
        let snapshot = Arc::new(OverviewSnapshot {
            generation,
            date: today,
            storage,
            baselines,
        });
        *cached = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// Marks the overview snapshot stale; the next request recomputes it.
    pub fn invalidate_overview(&self) {
        self.overview.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Discards the cached snapshot and computes a fresh one.
    pub async fn refresh_overview(&self) -> Result<Arc<OverviewSnapshot>> {
        self.invalidate_overview();
        self.overview_snapshot().await
    }
}

/// Reads all metrics files and selects each provider's trend baselines.
fn load_baselines(
    metrics_root: &Path,
    today: NaiveDate,
) -> Result<HashMap<String, TrendBaselines>> {
    let mut baselines = HashMap::new();
    for path in metrics_files(metrics_root)? {
        let series = match std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|data| Ok(serde_json::from_slice::<MetricsTimeSeries>(&data)?))
        {
            Ok(series) => series,
            Err(e) => {
                tracing::warn!("Failed to read metrics at {}: {e}", path.display());
                continue;
            }
        };
        let entry = TrendBaselines {
            week: baseline(&series, today, 7).cloned(),
            month: baseline(&series, today, 30).cloned(),
        };
        baselines.insert(series.provider, entry);
    }
    Ok(baselines)
}

/// Lists the JSON files in the metrics directory.
fn metrics_files(metrics_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(metrics_root)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            files.push(path);
        }
    }
    Ok(files)
}

/// Returns the latest snapshot dated at least `days` before `today`.
fn baseline(series: &MetricsTimeSeries, today: NaiveDate, days: u64) -> Option<&MetricsEntry> {
    let cutoff = today
        .checked_sub_days(Days::new(days))?
        .format("%Y-%m-%d")
        .to_string();
    // Dates are zero-padded `YYYY-MM-DD`, so string order matches date order.
    series
        .entries
        .iter()
        .rev()
        .find(|entry| entry.date <= cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a metrics snapshot for a date.
    fn entry(date: &str, document_count: u64) -> MetricsEntry {
        MetricsEntry {
            checks: None,
            date: date.into(),
            document_count,
            basic: None,
            extended: None,
            full: None,
            signatures: None,
            retrieval_errors: 0,
        }
    }

    /// Picks the latest snapshot on or before the cutoff.
    #[test]
    fn selects_baseline() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        let series = MetricsTimeSeries {
            provider: "example.com".into(),
            entries: vec![
                entry("2026-10-01", 1),
                entry("2026-10-07", 2),
                entry("2026-10-10", 3),
            ],
        };
        assert_eq!(
            baseline(&series, today, 7).map(|e| e.document_count),
            Some(2)
        );
        assert_eq!(
            baseline(&series, today, 13).map(|e| e.document_count),
            Some(1)
        );
        assert!(baseline(&series, today, 30).is_none());
    }

    /// Serves the cached snapshot until invalidated.
    #[tokio::test]
    async fn caches_until_invalidated() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path()).unwrap();
        let path = storage.metrics_root.join("example.com.json");
        let today = Utc::now().date_naive();
        let old = (today - Days::new(10)).format("%Y-%m-%d").to_string();
        let write = |count| {
            let series = MetricsTimeSeries {
                provider: "example.com".into(),
                entries: vec![entry(&old, count)],
            };
            std::fs::write(&path, serde_json::to_vec(&series).unwrap()).unwrap();
        };
        let week = |snapshot: &OverviewSnapshot| {
            snapshot.baselines["example.com"]
                .week
                .as_ref()
                .map(|e| e.document_count)
        };

        write(1);
        assert_eq!(week(&storage.overview_snapshot().await.unwrap()), Some(1));
        write(2);
        assert_eq!(week(&storage.overview_snapshot().await.unwrap()), Some(1));
        storage.invalidate_overview();
        assert_eq!(week(&storage.overview_snapshot().await.unwrap()), Some(2));
    }
}
