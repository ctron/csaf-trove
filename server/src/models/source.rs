use serde::{Deserialize, Serialize};

/// A CSAF provider source loaded from a TOML file in `sources/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Domain name or metadata URL of the CSAF provider (e.g. `redhat.com`).
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

/// Converts a domain string into a filesystem-safe key.
///
/// Strips URL scheme prefixes, replaces `:` and `/` with `-`,
/// collapses consecutive dashes, and trims trailing dashes.
/// Plain domain names pass through unchanged.
pub fn sanitize_domain(domain: &str) -> String {
    let s = domain
        .strip_prefix("https://")
        .or_else(|| domain.strip_prefix("http://"))
        .unwrap_or(domain);
    let mut result = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '/' || c == ':' {
            if !prev_dash && !result.is_empty() {
                result.push('-');
                prev_dash = true;
            }
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_domain_unchanged() {
        assert_eq!(sanitize_domain("redhat.com"), "redhat.com");
    }

    #[test]
    fn subdomain_unchanged() {
        assert_eq!(
            sanitize_domain("cert-portal.siemens.com"),
            "cert-portal.siemens.com"
        );
    }

    #[test]
    fn https_url_sanitized() {
        assert_eq!(
            sanitize_domain(
                "https://cert-portal.siemens.com/productcert/csaf/provider-metadata.json"
            ),
            "cert-portal.siemens.com-productcert-csaf-provider-metadata.json"
        );
    }

    #[test]
    fn http_url_sanitized() {
        assert_eq!(
            sanitize_domain("http://example.com/path/to/resource"),
            "example.com-path-to-resource"
        );
    }

    #[test]
    fn url_with_trailing_slash() {
        assert_eq!(sanitize_domain("https://example.com/"), "example.com");
    }

    #[test]
    fn url_with_port() {
        assert_eq!(
            sanitize_domain("https://example.com:8443/path"),
            "example.com-8443-path"
        );
    }
}
