use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::{
    AppState,
    models::{
        metrics::{MetricsEntry, MetricsProfileEntry},
        source::Source,
    },
};

/// Appends a metrics entry from the latest summary for a provider.
pub async fn generate_report(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;

    let Some(summary) = state
        .storage
        .list_summaries()
        .await?
        .into_iter()
        .find(|s| s.provider == *domain)
    else {
        tracing::warn!("No summary found for {domain}, skipping report");
        return Ok(());
    };

    let mut metrics = state.storage.load_metrics(domain).await?;

    let entry = MetricsEntry {
        date: Utc::now().format("%Y-%m-%d").to_string(),
        document_count: summary.document_count,
        basic: summary
            .profiles
            .basic
            .as_ref()
            .map(|p| MetricsProfileEntry {
                valid: p.valid,
                invalid: p.invalid,
                pass_rate: p.pass_rate,
            }),
        extended: summary
            .profiles
            .extended
            .as_ref()
            .map(|p| MetricsProfileEntry {
                valid: p.valid,
                invalid: p.invalid,
                pass_rate: p.pass_rate,
            }),
        full: summary.profiles.full.as_ref().map(|p| MetricsProfileEntry {
            valid: p.valid,
            invalid: p.invalid,
            pass_rate: p.pass_rate,
        }),
    };

    metrics.append(entry);
    state.storage.save_metrics(domain, &metrics).await?;

    tracing::info!(
        "Updated metrics for {domain} ({} entries)",
        metrics.entries.len()
    );
    Ok(())
}
