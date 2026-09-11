use std::path::Path;

use anyhow::Result;

use crate::models::{result::ProviderSummary, source::sanitize_domain};

/// Reads all provider summaries from the results directory, sorted by domain.
pub async fn list_summaries(results_dir: &Path) -> Result<Vec<ProviderSummary>> {
    let mut summaries = Vec::new();

    let mut entries = tokio::fs::read_dir(results_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            let summary_path = entry.path().join("summary.json");
            if summary_path.exists() {
                let data = tokio::fs::read_to_string(&summary_path).await?;
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

/// Loads a single provider's summary from disk, if it exists.
pub async fn load_summary(results_dir: &Path, domain: &str) -> Result<Option<ProviderSummary>> {
    let path = results_dir
        .join(sanitize_domain(domain))
        .join("summary.json");
    if !path.exists() {
        return Ok(None);
    }
    let data = tokio::fs::read_to_string(&path).await?;
    Ok(Some(serde_json::from_str(&data)?))
}

/// Persists a provider's validation summary to disk as JSON.
pub async fn save_summary(
    results_dir: &Path,
    domain: &str,
    summary: &ProviderSummary,
) -> Result<()> {
    let dir = results_dir.join(sanitize_domain(domain));
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("summary.json");
    let data = serde_json::to_string_pretty(summary)?;
    tokio::fs::write(&path, data).await?;
    Ok(())
}
