//! Internal hooks that react to finished provider jobs.
//!
//! The runner publishes a [`JobFinished`] event after every sync or revalidation. Each hook runs in
//! its own task and handles events one batch at a time, so a slow hook only delays itself. Events
//! that queue up while a hook is busy are delivered together as one batch.

use crate::{AppState, scheduler::runner::JobKind};
use anyhow::Result;
use std::{future::Future, sync::Arc};
use tokio::sync::broadcast::{self, Receiver, Sender, error::RecvError};

/// Events buffered per hook before it starts missing them.
const CAPACITY: usize = 256;

/// A provider job that finished, successfully or not.
#[derive(Debug, Clone)]
pub struct JobFinished {
    /// Domain of the provider.
    pub domain: String,
    /// Kind of job that ran.
    pub kind: JobKind,
    /// Whether the job completed without error.
    pub success: bool,
}

/// Publishes job events to registered hooks.
#[derive(Debug)]
pub struct Hooks {
    /// Broadcasts events to every subscribed hook.
    sender: Sender<JobFinished>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            sender: broadcast::channel(CAPACITY).0,
        }
    }
}

impl Hooks {
    /// Notifies all hooks that a job finished; does nothing if no hook is registered.
    pub fn job_finished(&self, event: JobFinished) {
        self.sender.send(event).ok();
    }

    /// Subscribes to future job events.
    pub fn subscribe(&self) -> Receiver<JobFinished> {
        self.sender.subscribe()
    }
}

/// Starts the built-in hooks.
pub fn spawn(state: &Arc<AppState>) {
    spawn_hook(state, "overview_cache", |state, _events| async move {
        state.storage.refresh_overview().await.map(|_| ())
    });
    spawn_hook(state, "provider_caches", |state, events| async move {
        let mut domains: Vec<String> = events.into_iter().map(|e| e.domain).collect();
        domains.sort();
        domains.dedup();
        state.storage.invalidate_provider_caches(&domains).await
    });
}

/// Runs `handler` for every batch of events until the channel closes.
fn spawn_hook<F, Fut>(state: &Arc<AppState>, name: &'static str, handler: F)
where
    F: Fn(Arc<AppState>, Vec<JobFinished>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut receiver = state.hooks.subscribe();
    let state = state.clone();
    tokio::spawn(async move {
        while let Some(events) = next_batch(&mut receiver, name).await {
            if let Err(e) = handler(state.clone(), events).await {
                tracing::warn!(hook = name, "Hook failed: {e:#}");
            }
        }
    });
}

/// Waits for an event and collects any others already queued; `None` once the channel closes.
async fn next_batch(receiver: &mut Receiver<JobFinished>, name: &str) -> Option<Vec<JobFinished>> {
    let mut events = Vec::new();
    loop {
        match receiver.recv().await {
            Ok(event) => {
                events.push(event);
                break;
            }
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(hook = name, missed, "Hook missed job events");
            }
            Err(RecvError::Closed) => return None,
        }
    }
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a successful sync event.
    fn event(domain: &str) -> JobFinished {
        JobFinished {
            domain: domain.into(),
            kind: JobKind::Sync,
            success: true,
        }
    }

    /// Delivers queued events as one batch and stops once the channel closes.
    #[tokio::test]
    async fn batches_queued_events() {
        let hooks = Hooks::default();
        let mut receiver = hooks.subscribe();
        hooks.job_finished(event("a"));
        hooks.job_finished(event("b"));

        let batch = next_batch(&mut receiver, "test").await.unwrap();
        let domains: Vec<_> = batch.iter().map(|e| e.domain.as_str()).collect();
        assert_eq!(domains, ["a", "b"]);

        drop(hooks);
        assert!(next_batch(&mut receiver, "test").await.is_none());
    }

    /// Every finished job, of any kind and outcome, refreshes the provider caches.
    #[tokio::test]
    async fn finished_jobs_refresh_provider_caches() {
        use crate::models::result::{ProfileResults, ProviderSummary};
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let state = AppState::for_test(dir.path());
        let summary = ProviderSummary {
            checks: None,
            provider: "a.example".into(),
            publisher_name: None,
            validated_at: time::OffsetDateTime::now_utc(),
            document_count: 0,
            profiles: ProfileResults {
                basic: None,
                extended: None,
                full: None,
            },
            signatures: None,
            top_failing_tests: vec![],
            retrieval_errors: 0,
            note: None,
        };
        state
            .storage
            .save_summary("a.example", &summary)
            .await
            .unwrap();
        spawn(&state);

        let path = dir.path().join("results/a.example/summary.json");
        let jobs = [
            (JobKind::Sync, true),
            (JobKind::Sync, false),
            (JobKind::Revalidate, true),
            (JobKind::Revalidate, false),
        ];
        for (count, (kind, success)) in (1..).zip(jobs) {
            // Change the summary behind the index's back; only the hook can pick it up.
            let mut json: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            json["document_count"] = count.into();
            std::fs::write(&path, json.to_string()).unwrap();

            state.hooks.job_finished(JobFinished {
                domain: "a.example".into(),
                kind,
                success,
            });

            let refreshed = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let summaries = state.storage.list_summaries().await.unwrap();
                    if summaries[0].document_count == count {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(refreshed.is_ok(), "{kind:?} (success: {success}) refreshed");
        }
    }
}
