pub mod runner;

use crate::{AppState, memory, models::state::JobPhase, pipeline::aggregator::generate_aggregator};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Semaphore, task::spawn_blocking, time::sleep};

/// Interval between memory samples while provider jobs are running.
const MEMORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

/// Runs the periodic scheduler loop, syncing all enabled providers on each interval.
pub async fn run(state: Arc<AppState>) {
    let interval = state.config.scheduler.sync_interval;

    if let Some(github) = &state.config.github {
        let poll_interval = github.poll_interval;
        let state_poll = state.clone();
        tokio::spawn(async move {
            loop {
                sleep(poll_interval).await;
                state_poll.sync_and_reload_sources().await;
            }
        });
    }

    tokio::spawn(sample_memory(state.clone()));

    loop {
        sync_all(&state).await;

        if state.config.aggregator.is_some()
            && let Err(e) = generate_aggregator(&state).await
        {
            tracing::error!("Aggregator generation failed: {e:#}");
        }

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

/// Periodically logs process memory with the phase of every running job.
///
/// Concurrent jobs share one process, so this attributes memory peaks to active phases.
async fn sample_memory(state: Arc<AppState>) {
    loop {
        sleep(MEMORY_SAMPLE_INTERVAL).await;
        let mut jobs: Vec<_> = state
            .jobs
            .read()
            .await
            .iter()
            .filter(|(_, job)| job.status == JobPhase::Running)
            .map(|(domain, job)| match job.phase {
                Some(phase) => format!("{domain}:{phase}"),
                None => domain.clone(),
            })
            .collect();
        if jobs.is_empty() {
            continue;
        }
        jobs.sort_unstable();
        if let Some(memory) = memory::usage() {
            tracing::info!(
                rss_mib = memory.rss_mib,
                peak_mib = memory.peak_mib,
                jobs = jobs.join(","),
                "Memory sample"
            );
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
