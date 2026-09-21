use crate::{
    AggregatorCategory, AppState,
    models::{
        aggregator::{
            AggregatorDocument, AggregatorInfo, CsafProviderEntry, ProviderMetadataRef,
            PublisherRef,
        },
        source::Source,
    },
    storage::ProviderInfo,
};
use anyhow::Result;
use chrono::Utc;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Generates the `aggregator.json` file from all synced providers.
pub async fn generate_aggregator(state: &Arc<AppState>) -> Result<()> {
    let config = match &state.config.aggregator {
        Some(c) => c,
        None => return Ok(()),
    };

    let output_dir = state.aggregator_dir();
    tokio::fs::create_dir_all(&output_dir).await?;

    let sources = state.sources.read().await;
    let all_info = state.storage.load_all_provider_info()?;

    let category_str = match config.category {
        AggregatorCategory::Lister => "lister",
        AggregatorCategory::Aggregator => "aggregator",
    };

    let mut entries = Vec::new();
    let mut namespaces = HashSet::new();

    for (domain, info) in &all_info {
        if !should_include(&sources, domain, info, &config.category) {
            tracing::debug!("Aggregator: excluding {domain}");
            continue;
        }

        namespaces.insert(info.publisher_namespace.clone());

        entries.push(CsafProviderEntry {
            metadata: ProviderMetadataRef {
                last_updated: chrono::DateTime::parse_from_rfc3339(&info.last_updated)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                publisher: PublisherRef {
                    category: info.publisher_category.clone(),
                    name: info.publisher_name.clone(),
                    namespace: info.publisher_namespace.clone(),
                },
                url: info.canonical_url.clone(),
                role: info.role.clone(),
            },
            mirrors: None,
        });
    }

    if namespaces.len() < 2 {
        tracing::warn!(
            "Aggregator: only {} disjoint issuing party namespace(s) found \
             (spec requirement 22 requires at least 2)",
            namespaces.len()
        );
    }

    if entries.is_empty() {
        tracing::warn!("Aggregator: no providers eligible for listing, skipping generation");
        return Ok(());
    }

    let doc = AggregatorDocument {
        aggregator: AggregatorInfo {
            category: category_str.to_string(),
            name: config.name.clone(),
            namespace: config.namespace.clone(),
            contact_details: config.contact_details.clone(),
            issuing_authority: config.issuing_authority.clone(),
        },
        aggregator_version: "2.0".to_string(),
        canonical_url: config.canonical_url.clone(),
        csaf_providers: entries,
        last_updated: Utc::now(),
    };

    let path = output_dir.join("aggregator.json");
    let data = serde_json::to_string_pretty(&doc)?;
    tokio::fs::write(&path, data).await?;

    tracing::info!(
        "Aggregator: wrote {} with {} provider(s)",
        path.display(),
        doc.csaf_providers.len()
    );

    Ok(())
}

/// Determines whether a provider should be included in the aggregator output.
fn should_include(
    sources: &HashMap<String, Source>,
    domain: &str,
    info: &ProviderInfo,
    category: &AggregatorCategory,
) -> bool {
    if let Some(source) = sources.get(domain)
        && let Some(include) = source.aggregator_include
    {
        return include;
    }

    match category {
        AggregatorCategory::Lister => info.list_on_aggregators,
        AggregatorCategory::Aggregator => info.mirror_on_aggregators,
    }
}
