use super::error::ApiError;
use crate::{
    AppState,
    models::{
        metrics::MetricsEntry,
        result::{ProfileSummary, ProviderSummary},
        state::{JobPhase, JobStatus},
    },
};
use actix_web::{HttpResponse, web};
use csaf_trove_common::overview::{
    Overview, ProviderHealth, SyncActivity, TrendDelta, Trends, ValidationOverview,
};
use std::{collections::HashSet, time::Duration};
use time::OffsetDateTime;

/// Returns aggregate provider counts, health, activity, trends, and persistent storage usage.
///
/// Storage usage and trend baselines come from a cache refreshed after every finished job.
pub async fn get(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    Ok(HttpResponse::Ok().json(build(&state).await?))
}

/// Builds the instance overview shared by the REST API and the MCP endpoint.
pub(crate) async fn build(state: &AppState) -> Result<Overview, ApiError> {
    let summaries = state.storage.list_summaries().await?;
    let awaiting_first_sync = {
        let known: HashSet<&str> = summaries.iter().map(|s| s.provider.as_str()).collect();
        let sources = state.sources.read().await;
        sources
            .iter()
            .filter(|(domain, source)| source.enabled && !known.contains(domain.as_str()))
            .count() as u64
    };

    let snapshot = state.storage.overview_snapshot().await?;

    let sync = {
        let jobs = state.jobs.read().await;
        sync_activity(jobs.values())
    };

    let overview = Overview {
        provider_count: summaries.len() as u64 + awaiting_first_sync,
        document_count: summaries.iter().map(|s| s.document_count).sum(),
        retrieval_errors: summaries.iter().map(|s| s.retrieval_errors).sum(),
        storage: snapshot.storage.clone(),
        validation: aggregate_validation(&summaries),
        providers: provider_health(
            &summaries,
            awaiting_first_sync,
            state.config.scheduler.sync_interval * 2,
            OffsetDateTime::now_utc(),
        ),
        sync,
        trends: Trends {
            week: trend_delta(&summaries, |domain| {
                snapshot.baselines.get(domain)?.week.as_ref()
            }),
            month: trend_delta(&summaries, |domain| {
                snapshot.baselines.get(domain)?.month.as_ref()
            }),
        },
    };
    Ok(overview)
}

/// Accumulates passed and total test counts for a weighted pass rate.
#[derive(Default)]
struct RateAccumulator {
    /// Passed tests.
    valid: u64,
    /// Passed and failed tests.
    total: u64,
}

impl RateAccumulator {
    /// Adds the counts of a profile, if present.
    fn add(&mut self, valid: u64, invalid: u64) {
        self.valid += valid;
        self.total += valid + invalid;
    }

    /// Adds a profile summary, if present.
    fn add_profile(&mut self, profile: Option<&ProfileSummary>) {
        if let Some(profile) = profile {
            self.add(profile.valid, profile.invalid);
        }
    }

    /// Returns the pass rate, or `None` without any tests.
    fn rate(&self) -> Option<f64> {
        (self.total > 0).then(|| self.valid as f64 / self.total as f64)
    }
}

/// Combines profile pass rates (weighted by test count) and essential check outcomes.
fn aggregate_validation(summaries: &[ProviderSummary]) -> ValidationOverview {
    let mut basic = RateAccumulator::default();
    let mut extended = RateAccumulator::default();
    let mut full = RateAccumulator::default();
    let mut validation = ValidationOverview::default();
    for summary in summaries {
        basic.add_profile(summary.profiles.basic.as_ref());
        extended.add_profile(summary.profiles.extended.as_ref());
        full.add_profile(summary.profiles.full.as_ref());
        if let Some(checks) = &summary.checks {
            validation.checks.merge(checks);
        }
    }
    validation.basic_pass_rate = basic.rate();
    validation.extended_pass_rate = extended.rate();
    validation.full_pass_rate = full.rate();
    validation
}

/// Classifies stored summaries by health; stale summaries are never counted as healthy.
fn provider_health(
    summaries: &[ProviderSummary],
    awaiting_first_sync: u64,
    stale_after: Duration,
    now: OffsetDateTime,
) -> ProviderHealth {
    let mut health = ProviderHealth {
        awaiting_first_sync,
        stale_after_seconds: stale_after.as_secs(),
        ..Default::default()
    };
    for summary in summaries {
        let stale = now - summary.validated_at > stale_after;
        let retrieval_errors = summary.retrieval_errors > 0;
        let check_issues = summary
            .checks
            .as_ref()
            .is_some_and(|checks| checks.total_issues() > 0);
        health.stale += u64::from(stale);
        health.with_retrieval_errors += u64::from(retrieval_errors);
        health.with_check_issues += u64::from(check_issues);
        health.healthy += u64::from(!stale && !retrieval_errors && !check_issues);
    }
    health
}

/// Counts running and failed jobs and finds the latest completion.
fn sync_activity<'a>(jobs: impl IntoIterator<Item = &'a JobStatus>) -> SyncActivity {
    let mut activity = SyncActivity::default();
    for job in jobs {
        match job.status {
            JobPhase::Running => activity.running += 1,
            JobPhase::Failed => activity.failed += 1,
            JobPhase::Pending | JobPhase::Completed => {}
        }
        let completed = job.completed_at.or(job.last_completed_at);
        if completed > activity.last_completed_at {
            activity.last_completed_at = completed;
        }
    }
    activity
}

/// Compares current summaries with each provider's trend baseline.
///
/// Only providers with a baseline are included on both sides, so newly added providers
/// don't show up as growth. Returns `None` if no provider has a baseline.
fn trend_delta<'a>(
    summaries: &[ProviderSummary],
    baseline: impl Fn(&str) -> Option<&'a MetricsEntry>,
) -> Option<TrendDelta> {
    let mut delta = TrendDelta::default();
    let mut current_rate = RateAccumulator::default();
    let mut past_rate = RateAccumulator::default();
    for summary in summaries {
        let Some(past) = baseline(&summary.provider) else {
            continue;
        };
        delta.providers += 1;
        delta.documents += summary.document_count as i64 - past.document_count as i64;
        delta.retrieval_errors += summary.retrieval_errors as i64 - past.retrieval_errors as i64;
        current_rate.add_profile(summary.profiles.basic.as_ref());
        if let Some(basic) = &past.basic {
            past_rate.add(basic.valid, basic.invalid);
        }
    }

    (delta.providers > 0).then(|| TrendDelta {
        basic_pass_rate: current_rate
            .rate()
            .zip(past_rate.rate())
            .map(|(current, past)| current - past),
        ..delta
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        metrics::{MetricsEntry, MetricsProfileEntry},
        result::ProfileResults,
    };
    use csaf_trove_common::document_checks::{CheckCounts, DocumentCheckSummary};

    /// Builds a summary with a basic profile and the given check issues.
    fn summary(
        provider: &str,
        documents: u64,
        basic: (u64, u64),
        retrieval_errors: u64,
        failed_parsing: u64,
        validated_at: OffsetDateTime,
    ) -> ProviderSummary {
        ProviderSummary {
            checks: Some(DocumentCheckSummary {
                parsing: CheckCounts {
                    passed: documents - failed_parsing,
                    failed: failed_parsing,
                    ..Default::default()
                },
                ..Default::default()
            }),
            provider: provider.into(),
            publisher_name: None,
            validated_at,
            document_count: documents,
            profiles: ProfileResults {
                basic: Some(ProfileSummary {
                    valid: basic.0,
                    invalid: basic.1,
                    pass_rate: 0.0,
                }),
                extended: None,
                full: None,
            },
            signatures: None,
            top_failing_tests: vec![],
            retrieval_errors,
            note: None,
        }
    }

    /// Builds a metrics snapshot for a date.
    fn entry(date: &str, documents: u64, basic: (u64, u64)) -> MetricsEntry {
        MetricsEntry {
            checks: None,
            date: date.into(),
            document_count: documents,
            basic: Some(MetricsProfileEntry {
                valid: basic.0,
                invalid: basic.1,
                pass_rate: 0.0,
            }),
            extended: None,
            full: None,
            signatures: None,
            retrieval_errors: 0,
        }
    }

    /// Weights pass rates by test count and sums check outcomes.
    #[test]
    fn aggregates_validation() {
        let now = OffsetDateTime::now_utc();
        let summaries = [
            summary("a", 10, (90, 10), 0, 1, now),
            summary("b", 5, (10, 0), 0, 2, now),
        ];
        let validation = aggregate_validation(&summaries);
        assert_eq!(validation.basic_pass_rate, Some(100.0 / 110.0));
        assert_eq!(validation.extended_pass_rate, None);
        assert_eq!(validation.checks.parsing.failed, 3);
        assert_eq!(validation.checks.parsing.passed, 12);
    }

    /// Classifies healthy, stale, and problematic providers.
    #[test]
    fn classifies_provider_health() {
        let now = OffsetDateTime::now_utc();
        let old = now - time::Duration::days(3);
        let summaries = [
            summary("healthy", 1, (1, 0), 0, 0, now),
            summary("errors", 1, (1, 0), 1, 0, now),
            summary("issues", 1, (1, 0), 0, 1, now),
            summary("stale", 1, (1, 0), 0, 0, old),
        ];
        let health = provider_health(&summaries, 2, Duration::from_secs(2 * 86400), now);
        assert_eq!(health.healthy, 1);
        assert_eq!(health.with_retrieval_errors, 1);
        assert_eq!(health.with_check_issues, 1);
        assert_eq!(health.stale, 1);
        assert_eq!(health.awaiting_first_sync, 2);
    }

    /// Compares only providers with a baseline.
    #[test]
    fn computes_trends() {
        let now = OffsetDateTime::now_utc();
        let summaries = [
            summary("a", 20, (90, 10), 2, 0, now),
            summary("new", 100, (1, 0), 0, 0, now),
        ];
        let past = entry("2026-10-07", 10, (80, 20));
        let baseline = |domain: &str| (domain == "a").then_some(&past);

        assert_eq!(
            trend_delta(&summaries, baseline),
            Some(TrendDelta {
                providers: 1,
                documents: 10,
                retrieval_errors: 2,
                basic_pass_rate: Some(0.9 - 0.8),
            })
        );
        assert_eq!(trend_delta(&summaries, |_| None), None);
    }
}
