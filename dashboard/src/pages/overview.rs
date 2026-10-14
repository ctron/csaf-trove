use crate::{
    components::section_heading::SectionHeading, pages::sync_status::format_relative_time,
};
use csaf_trove_common::overview::{Overview, SyncActivity, Trends, ValidationOverview};
use leptos::prelude::*;
use std::time::Duration;
use time::format_description::well_known::Rfc3339;

/// Loads aggregate metrics, reporting HTTP errors before decoding the response.
async fn fetch_overview() -> Result<Overview, String> {
    let response = gloo_net::http::Request::get("/api/overview")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(format!(
            "Unable to load overview (HTTP {}).",
            response.status()
        ));
    }
    response.json().await.map_err(|error| error.to_string())
}

/// Displays provider totals and a breakdown of persistent storage.
#[component]
pub fn OverviewPage() -> impl IntoView {
    let overview = LocalResource::new(fetch_overview);
    view! {
        <div>
            <SectionHeading>"Overview"</SectionHeading>
            <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading overview..."</p> }>
                {move || overview.get().map(|result| match result {
                    Ok(metrics) => view! { <OverviewMetrics metrics /> }.into_any(),
                    Err(error) => view! { <p role="alert" class="text-red-500 dark:text-red-400 text-center py-12">{error}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

/// Renders summary cards, health, validation, activity, trends, and storage categories.
#[component]
fn OverviewMetrics(metrics: Overview) -> impl IntoView {
    let cards = [
        ("Providers", metrics.provider_count.to_string()),
        ("Documents", metrics.document_count.to_string()),
        ("Retrieval errors", metrics.retrieval_errors.to_string()),
        ("Storage", format_bytes(metrics.storage.total())),
    ];
    let health = &metrics.providers;
    let health_cards = [
        ("Healthy", health.healthy.to_string()),
        (
            "With retrieval errors",
            health.with_retrieval_errors.to_string(),
        ),
        ("With check issues", health.with_check_issues.to_string()),
        (
            "Awaiting first sync",
            health.awaiting_first_sync.to_string(),
        ),
        ("Stale", health.stale.to_string()),
    ];
    let stale_after = csaf_trove_common::format_duration_hms(
        Duration::from_secs(health.stale_after_seconds),
        true,
    );
    let storage = [
        (
            "Repositories and document history",
            metrics.storage.repositories,
        ),
        ("Validation results and databases", metrics.storage.results),
        ("Sync state", metrics.storage.state),
        ("Historical metrics", metrics.storage.metrics),
    ];
    view! {
        <p class="text-sm text-gray-500 dark:text-gray-400 mb-6">
            "Document and retrieval-error counts reflect the latest completed provider summaries. Providers awaiting their first sync are included."
        </p>
        <StatCards cards />
        <h2 class="text-lg font-semibold text-gray-800 dark:text-white mb-4">"Provider health"</h2>
        <StatCards cards=health_cards />
        <p class="text-xs text-gray-500 dark:text-gray-400 -mt-4 mb-8">
            {format!("A provider is stale when its latest summary is older than {stale_after}. Healthy providers are current and have no retrieval errors or essential check issues.")}
        </p>
        <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 mb-8">
            <ValidationPanel validation=metrics.validation />
            <div class="flex flex-col gap-4">
                <SyncPanel sync=metrics.sync />
                <TrendsPanel trends=metrics.trends />
            </div>
        </div>
        <Panel title="Storage breakdown">
            <dl class="divide-y divide-gray-200 dark:divide-gray-700">
                {storage.into_iter().map(|(label, bytes)| view! {
                    <div class="flex justify-between gap-4 py-3">
                        <dt class="text-gray-600 dark:text-gray-300">{label}</dt>
                        <dd class="font-medium text-gray-800 dark:text-white whitespace-nowrap" title=format!("{bytes} bytes")>{format_bytes(bytes)}</dd>
                    </div>
                }).collect_view()}
            </dl>
            <p class="text-xs text-gray-500 dark:text-gray-400 mt-4">
                "Storage is the sum of file sizes, including Git history. Temporary worktrees and aggregator output are excluded. Sizes may change during sync."
            </p>
        </Panel>
    }
}

/// Renders a responsive grid of labeled summary values.
#[component]
fn StatCards<const N: usize>(cards: [(&'static str, String); N]) -> impl IntoView {
    view! {
        <dl class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-5 gap-4 mb-8">
            {cards.into_iter().map(|(label, value)| view! {
                <div class="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
                    <dt class="text-sm text-gray-500 dark:text-gray-400">{label}</dt>
                    <dd class="mt-2 text-3xl font-semibold text-gray-800 dark:text-white">{value}</dd>
                </div>
            }).collect_view()}
        </dl>
    }
}

/// Renders a titled card.
#[component]
fn Panel(title: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
            <h2 class="text-lg font-semibold text-gray-800 dark:text-white mb-4">{title}</h2>
            {children()}
        </div>
    }
}

/// Renders a label/value row inside a definition list.
#[component]
fn Row(label: &'static str, value: String, #[prop(optional)] class: &'static str) -> impl IntoView {
    view! {
        <div class="flex justify-between gap-4 py-3">
            <dt class="text-gray-600 dark:text-gray-300">{label}</dt>
            <dd class=format!("font-medium whitespace-nowrap {}", if class.is_empty() { "text-gray-800 dark:text-white" } else { class })>{value}</dd>
        </div>
    }
}

/// Shows weighted profile pass rates and essential check outcomes across providers.
#[component]
fn ValidationPanel(validation: ValidationOverview) -> impl IntoView {
    let rates = [
        ("Basic profile pass rate", validation.basic_pass_rate),
        ("Extended profile pass rate", validation.extended_pass_rate),
        ("Full profile pass rate", validation.full_pass_rate),
    ];
    let checks = validation.checks;
    let stages = [
        ("Retrieval", checks.retrieval),
        ("Parsing", checks.parsing),
        ("OpenPGP signature", checks.signature),
        ("Digest", checks.digest),
    ];
    view! {
        <Panel title="Validation">
            <dl class="divide-y divide-gray-200 dark:divide-gray-700 mb-4">
                {rates.into_iter().map(|(label, rate)| view! {
                    <Row label value=rate.map(format_percent).unwrap_or_else(|| "\u{2014}".into()) />
                }).collect_view()}
            </dl>
            <table class="w-full text-sm">
                <thead>
                    <tr class="text-left text-gray-500 dark:text-gray-400">
                        <th class="py-2 font-normal">"Check"</th>
                        <th class="py-2 font-normal text-right">"Passed"</th>
                        <th class="py-2 font-normal text-right">"Failed"</th>
                        <th class="py-2 font-normal text-right">"Warning"</th>
                        <th class="py-2 font-normal text-right">"Missing"</th>
                    </tr>
                </thead>
                <tbody class="divide-y divide-gray-200 dark:divide-gray-700 text-gray-800 dark:text-white">
                    {stages.into_iter().map(|(label, counts)| view! {
                        <tr>
                            <td class="py-2 text-gray-600 dark:text-gray-300">{label}</td>
                            <td class="py-2 text-right">{counts.passed}</td>
                            <td class=format!("py-2 text-right {}", highlight(counts.failed, "text-red-600 dark:text-red-400"))>{counts.failed}</td>
                            <td class=format!("py-2 text-right {}", highlight(counts.warning, "text-yellow-600 dark:text-yellow-400"))>{counts.warning}</td>
                            <td class="py-2 text-right">{counts.missing}</td>
                        </tr>
                    }).collect_view()}
                </tbody>
            </table>
            <p class="text-xs text-gray-500 dark:text-gray-400 mt-4">
                "Pass rates are weighted by the number of tests across all providers. Check counts are documents per outcome."
            </p>
        </Panel>
    }
}

/// Shows scheduler job counts and the latest completion.
#[component]
fn SyncPanel(sync: SyncActivity) -> impl IntoView {
    let last_completed = sync
        .last_completed_at
        .and_then(|at| at.format(&Rfc3339).ok())
        .map(|at| format_relative_time(&at, js_sys::Date::now()))
        .unwrap_or_else(|| "\u{2014}".into());
    let failed_class = highlight(sync.failed, "text-red-600 dark:text-red-400");
    view! {
        <Panel title="Sync activity">
            <dl class="divide-y divide-gray-200 dark:divide-gray-700">
                <Row label="Running" value=sync.running.to_string() />
                <Row label="Failed" value=sync.failed.to_string() class=failed_class />
                <Row label="Last completed" value=last_completed />
            </dl>
        </Panel>
    }
}

/// Shows changes over the last 7 and 30 days.
#[component]
fn TrendsPanel(trends: Trends) -> impl IntoView {
    let windows = [("7 days", trends.week), ("30 days", trends.month)];
    view! {
        <Panel title="Trends">
            <table class="w-full text-sm">
                <thead>
                    <tr class="text-left text-gray-500 dark:text-gray-400">
                        <th class="py-2 font-normal">"Period"</th>
                        <th class="py-2 font-normal text-right">"Documents"</th>
                        <th class="py-2 font-normal text-right">"Retrieval errors"</th>
                        <th class="py-2 font-normal text-right">"Basic pass rate"</th>
                    </tr>
                </thead>
                <tbody class="divide-y divide-gray-200 dark:divide-gray-700 text-gray-800 dark:text-white">
                    {windows.into_iter().map(|(label, delta)| match delta {
                        Some(delta) => view! {
                            <tr title=format!("Compared across {} providers with history", delta.providers)>
                                <td class="py-2 text-gray-600 dark:text-gray-300">{label}</td>
                                <td class="py-2 text-right">{format_signed(delta.documents)}</td>
                                <td class=format!("py-2 text-right {}", trend_class(-delta.retrieval_errors as f64))>{format_signed(delta.retrieval_errors)}</td>
                                <td class=format!("py-2 text-right {}", trend_class(delta.basic_pass_rate.unwrap_or_default()))>
                                    {delta.basic_pass_rate.map(format_points).unwrap_or_else(|| "\u{2014}".into())}
                                </td>
                            </tr>
                        }.into_any(),
                        None => view! {
                            <tr>
                                <td class="py-2 text-gray-600 dark:text-gray-300">{label}</td>
                                <td class="py-2 text-right text-gray-500 dark:text-gray-400" colspan="3">"Not enough history"</td>
                            </tr>
                        }.into_any(),
                    }).collect_view()}
                </tbody>
            </table>
            <p class="text-xs text-gray-500 dark:text-gray-400 mt-4">
                "Compares current summaries with daily snapshots of the same providers; newly added providers are excluded."
            </p>
        </Panel>
    }
}

/// Returns the highlight classes for non-zero counts.
fn highlight(count: u64, classes: &'static str) -> &'static str {
    if count > 0 { classes } else { "" }
}

/// Colors improvements green and regressions red.
fn trend_class(improvement: f64) -> &'static str {
    if improvement > 0.0 {
        "text-green-600 dark:text-green-400"
    } else if improvement < 0.0 {
        "text-red-600 dark:text-red-400"
    } else {
        ""
    }
}

/// Formats a fraction as a percentage.
fn format_percent(rate: f64) -> String {
    format!("{:.1} %", rate * 100.0)
}

/// Formats a fraction difference as signed percentage points.
fn format_points(delta: f64) -> String {
    format!("{:+.1} pp", delta * 100.0)
}

/// Formats a count difference with an explicit sign.
fn format_signed(delta: i64) -> String {
    format!("{delta:+}")
}

/// Formats byte sizes using binary units.
fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, format_points, format_signed};

    /// Covers empty storage, the binary-unit boundary, and large sizes.
    #[test]
    fn formats_storage_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(format_bytes(u64::MAX), "16.0 EiB");
    }

    /// Shows explicit signs for trend deltas.
    #[test]
    fn formats_trend_deltas() {
        assert_eq!(format_signed(12), "+12");
        assert_eq!(format_signed(-3), "-3");
        assert_eq!(format_signed(0), "+0");
        assert_eq!(format_points(0.004), "+0.4 pp");
        assert_eq!(format_points(-0.1), "-10.0 pp");
    }
}
