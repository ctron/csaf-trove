use crate::models::result::ProviderSummary;
use anyhow::Result;
use std::path::Path;
use tokio::fs;

/// Reads all provider summaries from the results directory, sorted by domain.
pub async fn list_summaries(results_dir: &Path) -> Result<Vec<ProviderSummary>> {
    let mut summaries = Vec::new();

    let mut entries = fs::read_dir(results_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            let summary_path = entry.path().join("summary.json");
            if summary_path.exists() {
                let data = fs::read_to_string(&summary_path).await?;
                match serde_json::from_str::<ProviderSummary>(&data) {
                    Ok(summary) => summaries.push(summary),
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse summary at {}: {e}",
                            summary_path.display()
                        );
                    }
                }
            }
        }
    }

    summaries.sort_by(|a, b| a.provider.cmp(&b.provider));
    Ok(summaries)
}
