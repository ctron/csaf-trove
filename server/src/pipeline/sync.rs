use std::{fmt::Debug, path::Path, sync::Arc, time::SystemTime};

use anyhow::Result;
use chrono::Utc;
use csaf_walker::{
    common::{
        fetcher::{Fetcher, FetcherOptions},
        retrieve::RetrievalError,
    },
    discover::DiscoveredAdvisory,
    metadata::MetadataRetriever,
    retrieve::{RetrievalContext, RetrievedAdvisory, RetrievedVisitor, RetrievingVisitor},
    source::{HttpOptions, HttpSource, Source},
    visitors::store::{StoreRetrievedError, StoreVisitor},
    walker::Walker,
};

use crate::{AppState, models::source::Source as AppSource};

/// Wraps [`StoreVisitor`] to increment the synced document count after each advisory.
struct CountingStoreVisitor {
    /// Inner visitor that performs the actual storage.
    inner: StoreVisitor,
    /// Shared application state for updating job progress.
    state: Arc<AppState>,
    /// Provider domain name.
    domain: String,
}

impl<S: Source + Debug> RetrievedVisitor<S> for CountingStoreVisitor
where
    S::Error: 'static,
{
    type Error = StoreRetrievedError<S>;
    type Context = <StoreVisitor as RetrievedVisitor<S>>::Context;

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

/// Downloads new and changed CSAF documents from a provider into the worktree.
pub async fn sync_provider(
    state: &Arc<AppState>,
    source: &AppSource,
    worktree_dir: &Path,
) -> Result<()> {
    let domain = &source.domain;
    tracing::info!("Syncing documents for {domain}");

    let mut sync_state = state.storage.load_sync_state(domain).await?;

    let fetcher = Fetcher::new(FetcherOptions::default()).await?;
    let metadata = MetadataRetriever::new(domain);

    let mut http_options = HttpOptions::default();
    if let Some(since) = sync_state.since_token {
        http_options = http_options.since(SystemTime::from(since));
    }

    let http_source = HttpSource::new(metadata, fetcher, http_options);
    let store = StoreVisitor::new(worktree_dir);
    let counting_store = CountingStoreVisitor {
        inner: store,
        state: state.clone(),
        domain: domain.to_string(),
    };
    let retriever = RetrievingVisitor::new(http_source.clone(), counting_store);

    Walker::new(http_source)
        .walk(retriever)
        .await
        .map_err(|e| anyhow::anyhow!("Walker failed for {domain}: {e}"))?;

    sync_state.last_sync = Some(Utc::now());
    sync_state.since_token = Some(Utc::now());
    state.storage.save_sync_state(&sync_state).await?;

    tracing::info!("Sync complete for {domain}");
    Ok(())
}
