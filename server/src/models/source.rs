use serde::{Deserialize, Serialize};

/// A CSAF provider source loaded from a TOML file in `sources/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Domain name of the CSAF provider (e.g. `redhat.com`).
    pub domain: String,
    /// Whether this source is active for scheduled syncs.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional override for the provider metadata URL.
    pub metadata_url: Option<String>,
    /// Whether to accept OpenPGP v3 signatures.
    #[serde(default)]
    pub accept_v3_signatures: bool,
}

fn default_true() -> bool {
    true
}
