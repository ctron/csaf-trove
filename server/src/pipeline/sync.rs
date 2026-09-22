use std::{fmt::Debug, path::Path, sync::Arc, time::SystemTime};

use parking_lot::Mutex;

use anyhow::Result;
use csaf_walker::{
    common::{
        fetcher::Fetcher,
        progress::{Progress, ProgressBar},
        retrieve::RetrievalError,
    },
    discover::{DiscoveredAdvisory, DistributionContext},
    metadata::MetadataRetriever,
    model::metadata::TlpLabel,
    retrieve::{RetrievalContext, RetrievedAdvisory, RetrievedVisitor, RetrievingVisitor},
    source::{HttpOptions, HttpSource, Source},
    walker::Walker,
};
use time::OffsetDateTime;
use walker_common::utils::url::Urlify;

use crate::{AppState, models::source::Source as AppSource};

use super::store::{TroveStoreError, TroveStoreVisitor};

/// A document that could not be retrieved during sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievalFailure {
    /// URL of the document that failed to download.
    pub url: String,
    /// Human-readable error description.
    pub error: String,
}

/// Result of a successful sync run.
pub struct SyncResult {
    /// Documents that could not be retrieved.
    pub retrieval_errors: Vec<RetrievalFailure>,
    /// Distribution feeds that could not be loaded (e.g. 403 on restricted TLP feeds).
    pub distribution_errors: Vec<RetrievalFailure>,
}

/// Wraps [`TroveStoreVisitor`] to increment the synced document count and collect retrieval errors.
struct CountingStoreVisitor {
    /// Inner visitor that performs the actual storage.
    inner: TroveStoreVisitor,
    /// Shared application state for updating job progress.
    state: Arc<AppState>,
    /// Provider domain name.
    domain: String,
    /// Collected retrieval failures (documents that could not be downloaded).
    retrieval_errors: Arc<Mutex<Vec<RetrievalFailure>>>,
}

impl<S: Source + Debug> RetrievedVisitor<S> for CountingStoreVisitor
where
    S::Error: 'static,
{
    type Error = TroveStoreError;
    type Context = ();

    async fn visit_context(
        &self,
        context: &RetrievalContext<'_>,
    ) -> Result<Self::Context, Self::Error> {
        <TroveStoreVisitor as RetrievedVisitor<S>>::visit_context(&self.inner, context).await
    }

    async fn visit_advisory(
        &self,
        context: &Self::Context,
        result: Result<RetrievedAdvisory, RetrievalError<DiscoveredAdvisory, S>>,
    ) -> Result<(), Self::Error> {
        if let Err(err) = &result {
            let url = err.url().to_string();
            let error = format!("{err}");
            tracing::warn!("Failed to retrieve {url}: {error}");
            self.retrieval_errors
                .lock()
                .push(RetrievalFailure { url, error });
            self.state.increment_job_synced(&self.domain).await;
            return Ok(());
        }

        <TroveStoreVisitor as RetrievedVisitor<S>>::visit_advisory(&self.inner, context, result)
            .await?;
        self.state.increment_job_synced(&self.domain).await;
        Ok(())
    }
}

/// Reports the total document count to the job status when the walker starts.
pub(crate) struct JobProgress {
    /// Shared application state for updating job progress.
    pub(crate) state: Arc<AppState>,
    /// Provider domain name.
    pub(crate) domain: String,
}

/// No-op progress bar — individual ticks are handled by the counting visitors.
pub(crate) struct JobProgressBar;

impl ProgressBar for JobProgressBar {
    async fn increment(&mut self, _work: usize) {}
    async fn finish(self) {}
    async fn set_message(&mut self, _msg: String) {}
}

impl Progress for JobProgress {
    type Instance = JobProgressBar;

    fn start(&self, work: usize) -> Self::Instance {
        let state = self.state.clone();
        let domain = self.domain.clone();
        tokio::spawn(async move {
            state.set_job_documents_total(&domain, work).await;
        });
        JobProgressBar
    }
}

/// Downloads new and changed CSAF documents from a provider into the worktree.
///
/// Returns a list of documents that could not be retrieved (e.g. due to HTTP errors).
/// These are collected instead of aborting the sync so that remaining documents can
/// still be processed.
pub async fn sync_provider(
    state: &Arc<AppState>,
    source: &AppSource,
    worktree_dir: &Path,
) -> Result<SyncResult> {
    let domain = &source.domain;

    let mut sync_state = state.storage.load_sync_state(domain).await?;

    if sync_state.since_token.is_some() {
        let db_count = state.storage.document_count(domain).await?;
        if db_count == 0 {
            tracing::warn!(
                "{domain}: document database is empty but since_token is set; \
                 clearing token to force full sync"
            );
            sync_state.since_token = None;
        }
    }

    if let Some(ref since) = sync_state.since_token {
        tracing::info!("{domain}: starting incremental sync (since {since})");
    } else {
        tracing::info!("{domain}: starting full sync (no since_token)");
    }

    // TODO(#6): remove custom client once upstream csaf-walker sets a User-Agent in Fetcher::new
    let client = reqwest::ClientBuilder::new()
        .user_agent(concat!("csaf-trove/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let fetcher = Fetcher::from(client);
    let metadata = MetadataRetriever::new(domain);

    let mut http_options = HttpOptions::default();
    if let Some(since) = sync_state.since_token {
        http_options = http_options.since(SystemTime::from(since));
    }

    let retrieval_errors = Arc::new(Mutex::new(Vec::new()));
    let distribution_errors = Arc::new(Mutex::new(Vec::new()));

    let http_source = HttpSource::new(metadata, fetcher, http_options);
    let store = TroveStoreVisitor::new(worktree_dir);
    let counting_store = CountingStoreVisitor {
        inner: store,
        state: state.clone(),
        domain: domain.to_string(),
        retrieval_errors: retrieval_errors.clone(),
    };
    let retriever = RetrievingVisitor::new(http_source.clone(), counting_store);
    let progress = JobProgress {
        state: state.clone(),
        domain: domain.to_string(),
    };

    let de = distribution_errors.clone();
    let skip_dirs = source.skip_directories.clone();
    let mut walker = Walker::new(http_source);
    if !skip_dirs.is_empty() {
        walker = walker.with_distribution_filter(move |ctx: &DistributionContext| {
            if let DistributionContext::Directory(url) = ctx {
                let dominated = skip_dirs.iter().any(|s| s == url.as_str());
                if dominated {
                    tracing::info!("Skipping configured directory distribution {url}");
                }
                !dominated
            } else {
                true
            }
        });
    }
    walker
        .with_distribution_error_handler(move |ctx: &DistributionContext, error| {
            match ctx {
                DistributionContext::Feed {
                    tlp_label: Some(label),
                    ..
                } if *label == TlpLabel::Clear => Err(error),
                _ => {
                    let label = ctx
                        .tlp_label()
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    tracing::warn!("Skipping {label} distribution {}: {error}", ctx.url());
                    de.lock().push(RetrievalFailure {
                        url: ctx.url().to_string(),
                        error: format!("{error}"),
                    });
                    Ok(())
                }
            }
        })
        .with_progress(progress)
        .walk(retriever)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    sync_state.last_sync = Some(OffsetDateTime::now_utc());
    sync_state.since_token = Some(OffsetDateTime::now_utc());
    state.storage.save_sync_state(&sync_state).await?;

    let retrieval_errors = match Arc::try_unwrap(retrieval_errors) {
        Ok(mutex) => mutex.into_inner(),
        Err(arc) => std::mem::take(&mut *arc.lock()),
    };
    let distribution_errors = match Arc::try_unwrap(distribution_errors) {
        Ok(mutex) => mutex.into_inner(),
        Err(arc) => std::mem::take(&mut *arc.lock()),
    };

    let total_errors = retrieval_errors.len() + distribution_errors.len();
    if total_errors == 0 {
        tracing::info!("Sync complete for {domain}");
    } else {
        tracing::warn!(
            "Sync complete for {domain} with {total_errors} error(s) \
             ({} retrieval, {} distribution)",
            retrieval_errors.len(),
            distribution_errors.len(),
        );
    }

    Ok(SyncResult {
        retrieval_errors,
        distribution_errors,
    })
}
