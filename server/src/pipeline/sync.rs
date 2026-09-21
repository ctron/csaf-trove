use std::{fmt::Debug, path::Path, sync::Arc, time::SystemTime};

use anyhow::Result;
use chrono::Utc;
use csaf_walker::{
    common::{
        fetcher::{Fetcher, FetcherOptions},
        progress::{Progress, ProgressBar},
        retrieve::RetrievalError,
    },
    discover::DiscoveredAdvisory,
    metadata::MetadataRetriever,
    retrieve::{RetrievalContext, RetrievedAdvisory, RetrievedVisitor, RetrievingVisitor},
    source::{HttpOptions, HttpSource, Source},
    walker::Walker,
};

use crate::{AppState, models::source::Source as AppSource};

use super::store::{TroveStoreError, TroveStoreVisitor};

/// Wraps [`TroveStoreVisitor`] to increment the synced document count after each advisory.
struct CountingStoreVisitor {
    /// Inner visitor that performs the actual storage.
    inner: TroveStoreVisitor,
    /// Shared application state for updating job progress.
    state: Arc<AppState>,
    /// Provider domain name.
    domain: String,
}

impl<S: Source + Debug> RetrievedVisitor<S> for CountingStoreVisitor
where
    S::Error: 'static,
{
    type Error = TroveStoreError<S>;
    type Context = ();

    async fn visit_context(
        &self,
        context: &RetrievalContext<'_>,
    ) -> Result<Self::Context, Self::Error> {
        self.inner.visit_context(context).await
    }

    async fn visit_advisory(
        &self,
        context: &Self::Context,
        result: Result<RetrievedAdvisory, RetrievalError<DiscoveredAdvisory, S>>,
    ) -> Result<(), Self::Error> {
        self.inner.visit_advisory(context, result).await?;
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
pub async fn sync_provider(
    state: &Arc<AppState>,
    source: &AppSource,
    worktree_dir: &Path,
) -> Result<()> {
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

    let mut fetcher_options = FetcherOptions::default();
    if let Some(retries) = source.retries {
        fetcher_options = fetcher_options.retries(retries);
    }
    let fetcher = Fetcher::new(fetcher_options).await?;
    let metadata = MetadataRetriever::new(domain);

    let mut http_options = HttpOptions::default();
    if let Some(since) = sync_state.since_token {
        http_options = http_options.since(SystemTime::from(since));
    }

    let http_source = HttpSource::new(metadata, fetcher, http_options);
    let store = TroveStoreVisitor::new(worktree_dir);
    let counting_store = CountingStoreVisitor {
        inner: store,
        state: state.clone(),
        domain: domain.to_string(),
    };
    let retriever = RetrievingVisitor::new(http_source.clone(), counting_store);
    let progress = JobProgress {
        state: state.clone(),
        domain: domain.to_string(),
    };

    Walker::new(http_source)
        .with_progress(progress)
        .walk(retriever)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    sync_state.last_sync = Some(Utc::now());
    sync_state.since_token = Some(Utc::now());
    state.storage.save_sync_state(&sync_state).await?;

    tracing::info!("Sync complete for {domain}");
    Ok(())
}
