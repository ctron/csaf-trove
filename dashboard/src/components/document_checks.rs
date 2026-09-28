//! Shared presentation of essential document outcomes and provider counts.

use crate::components::badge::{Badge, BadgeVariant};
use csaf_trove_common::document_checks::{CheckCounts, CheckOutcome, CheckStatus};
use leptos::prelude::*;

/// Returns a human-readable label and color for an outcome.
fn presentation(status: CheckStatus) -> (&'static str, BadgeVariant) {
    match status {
        CheckStatus::Passed => ("Passed", BadgeVariant::Success),
        CheckStatus::Failed => ("Failed", BadgeVariant::Danger),
        CheckStatus::Warning => ("Warning", BadgeVariant::Warning),
        CheckStatus::Missing => ("Missing", BadgeVariant::Warning),
        CheckStatus::NotEvaluated => ("Not evaluated", BadgeVariant::Neutral),
    }
}

/// Displays one outcome with its diagnostic available on hover.
#[component]
pub fn CheckBadge(
    /// Outcome to display.
    outcome: CheckOutcome,
) -> impl IntoView {
    let (label, variant) = presentation(outcome.status);
    view! {
        <span title=outcome.message><Badge variant=variant>{label}</Badge></span>
    }
}

/// Displays outcome counts linked to the corresponding document filter.
#[component]
pub fn CheckSummaryView(
    /// Counts, absent when an old summary has no independent results.
    counts: Option<CheckCounts>,
    /// Provider page URL.
    provider_url: String,
    /// Essential check name used by the API filter.
    stage: &'static str,
) -> impl IntoView {
    let Some(counts) = counts else {
        return view! { <span class="text-gray-500">"Not evaluated"</span> }.into_any();
    };
    if counts.passed + counts.failed + counts.warning + counts.missing + counts.not_evaluated == 0 {
        return view! { <span class="text-gray-500">"—"</span> }.into_any();
    }
    let outcomes = [
        (CheckStatus::Passed, "passed", counts.passed),
        (CheckStatus::Failed, "failed", counts.failed),
        (CheckStatus::Warning, "warning", counts.warning),
        (CheckStatus::Missing, "missing", counts.missing),
        (
            CheckStatus::NotEvaluated,
            "not_evaluated",
            counts.not_evaluated,
        ),
    ];
    view! {
        <div class="flex flex-wrap gap-1">
            {outcomes.into_iter().filter(|(_, _, count)| *count > 0).map(|(status, key, count)| {
                let (label, variant) = presentation(status);
                let href = format!("{provider_url}?status={stage}-{key}");
                view! { <a href=href aria-label=format!("{stage}: {count} {}", label.to_lowercase())><Badge variant=variant>{format!("{count} {}", label.to_lowercase())}</Badge></a> }
            }).collect::<Vec<_>>()}
        </div>
    }.into_any()
}
