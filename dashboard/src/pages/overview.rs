use crate::components::section_heading::SectionHeading;
use csaf_trove_common::overview::Overview;
use leptos::prelude::*;

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

/// Renders summary cards and storage categories using the dashboard's existing styles.
#[component]
fn OverviewMetrics(metrics: Overview) -> impl IntoView {
    let cards = [
        ("Providers", metrics.provider_count.to_string()),
        ("Documents", metrics.document_count.to_string()),
        ("Retrieval errors", metrics.retrieval_errors.to_string()),
        ("Storage", format_bytes(metrics.storage.total())),
    ];
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
        <dl class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-8">
            {cards.into_iter().map(|(label, value)| view! {
                <div class="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
                    <dt class="text-sm text-gray-500 dark:text-gray-400">{label}</dt>
                    <dd class="mt-2 text-3xl font-semibold text-gray-800 dark:text-white">{value}</dd>
                </div>
            }).collect_view()}
        </dl>
        <div class="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
            <h2 class="text-lg font-semibold text-gray-800 dark:text-white mb-4">"Storage breakdown"</h2>
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
        </div>
    }
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
    use super::format_bytes;

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
}
