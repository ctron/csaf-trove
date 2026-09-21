use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Top-level `aggregator.json` document per the CSAF 2.0 specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorDocument {
    /// Information about the aggregating entity.
    pub aggregator: AggregatorInfo,
    /// Version of the aggregator metadata format (always `"2.0"`).
    pub aggregator_version: String,
    /// Authoritative URL of this `aggregator.json` (must end in `/aggregator.json`).
    pub canonical_url: String,
    /// List of tracked CSAF providers.
    pub csaf_providers: Vec<CsafProviderEntry>,
    /// When this document was last updated.
    pub last_updated: DateTime<Utc>,
}

/// Describes the aggregating entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorInfo {
    /// Aggregator category: `"lister"` or `"aggregator"`.
    pub category: String,
    /// Human-readable name.
    pub name: String,
    /// URI namespace identifying the aggregator.
    pub namespace: String,
    /// Contact information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_details: Option<String>,
    /// Description of the aggregator's authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuing_authority: Option<String>,
}

/// An entry in the `csaf_providers` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsafProviderEntry {
    /// Metadata about the provider.
    pub metadata: ProviderMetadataRef,
    /// Mirror URLs (only present in aggregator/mirror mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirrors: Option<Vec<String>>,
}

/// Reference to a provider's metadata within `aggregator.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetadataRef {
    /// When the provider's metadata was last updated.
    pub last_updated: DateTime<Utc>,
    /// Publisher information from the provider's metadata.
    pub publisher: PublisherRef,
    /// URL of the provider's `provider-metadata.json`.
    pub url: String,
    /// Role of the issuing party.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Publisher information embedded in an aggregator entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherRef {
    /// Publisher category (e.g. `"vendor"`, `"coordinator"`).
    pub category: String,
    /// Publisher name.
    pub name: String,
    /// Publisher namespace URI.
    pub namespace: String,
}
