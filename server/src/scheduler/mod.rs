pub mod runner;

use crate::AppState;
use std::sync::Arc;
use tokio::{sync::Semaphore, task::spawn_blocking, time::sleep};

/// Runs the periodic scheduler loop, syncing all enabled providers on each interval.
pub async fn run(state: Arc<AppState>) {
    let interval = state.config.scheduler.sync_interval;
    let poll_interval = state.config.github.poll_interval;

    let state_poll = state.clone();
    tokio::spawn(async move {
        loop {
            sleep(poll_interval).await;
            state_poll.sync_and_reload_sources().await;
        }
    });

    loop {
        sync_all(&state).await;

        tracing::info!(
            "Scheduled sync complete, sleeping for {}",
            humantime::format_duration(interval)
        );

        tokio::select! {
            () = sleep(interval) => {}
            () = state.sources_changed.notified() => {
                tracing::info!("Sources changed, triggering early sync");
            }
        }
    }
}

/// Syncs all enabled providers, respecting the concurrency limit.
async fn sync_all(state: &Arc<AppState>) {
    tracing::info!("Starting scheduled sync for all providers");
    let sources: Vec<_> = state
        .sources
        .read()
        .await
        .values()
        .filter(|s| s.enabled)
        .cloned()
        .collect();

    let semaphore = Arc::new(Semaphore::new(
        state.config.scheduler.max_concurrent as usize,
    ));

    let mut handles = Vec::new();
    for source in sources {
        let state = state.clone();
        let semaphore = semaphore.clone();
        let handle = tokio::runtime::Handle::current();
        handles.push(spawn_blocking(move || {
            handle.block_on(async move {
                let _permit = semaphore.acquire().await;
                if let Err(e) = runner::run_provider(&state, &source).await {
                    tracing::error!("Sync for {} failed: {e}", source.domain);
                }
            });
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }
}
