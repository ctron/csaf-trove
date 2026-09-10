use std::{path::Path, sync::Arc, time::SystemTime};

use anyhow::Result;
use chrono::Utc;
use csaf_walker::{
    common::fetcher::{Fetcher, FetcherOptions},
    metadata::MetadataRetriever,
    retrieve::RetrievingVisitor,
    source::{HttpOptions, HttpSource},
    visitors::store::StoreVisitor,
    walker::Walker,
};

use crate::{AppState, models::source::Source};

/// Downloads new and changed CSAF documents from a provider into the worktree.
pub async fn sync_provider(
    state: &Arc<AppState>,
    source: &Source,
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
    let retriever = RetrievingVisitor::new(http_source.clone(), store);

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
