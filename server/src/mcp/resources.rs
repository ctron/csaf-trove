//! MCP resources exposing the raw JSON of stored advisories.

use super::internal;
use crate::{AppState, storage::versions::RawDocument};
use bytesize::ByteSize;
use percent_encoding::percent_decode_str;
use rmcp::{
    ErrorData,
    model::{ReadResourceResult, ResourceContents, ResourceTemplate},
};

/// URI scheme of advisory resources.
const SCHEME: &str = "csaf://";

/// Largest advisory served as a resource; bigger ones are only available through the tools.
const MAX_RAW_BYTES: usize = 4 * 1024 * 1024;

/// Advertised resource templates.
pub fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("csaf://{domain}/{tracking_id}", "advisory")
            .with_title("CSAF advisory")
            .with_description(
                "Raw JSON of the current version of an advisory. The tracking ID is percent-encoded.",
            )
            .with_mime_type("application/json"),
        ResourceTemplate::new("csaf://{domain}/{tracking_id}@{commit_id}", "advisory-version")
            .with_title("CSAF advisory version")
            .with_description(
                "Raw JSON of a historical version of an advisory, by commit ID from list_document_versions.",
            )
            .with_mime_type("application/json"),
    ]
}

/// A parsed advisory resource URI.
#[derive(Debug, PartialEq, Eq)]
struct AdvisoryRef {
    /// Provider domain.
    domain: String,
    /// Decoded CSAF tracking ID.
    tracking_id: String,
    /// Commit ID of a historical version, or `None` for the current version.
    commit_id: Option<String>,
}

/// Parses `csaf://{domain}/{tracking_id}[@{commit_id}]`.
fn parse(uri: &str) -> Option<AdvisoryRef> {
    let (domain, rest) = uri.strip_prefix(SCHEME)?.split_once('/')?;
    let (tracking_id, commit_id) = match rest.rsplit_once('@') {
        Some((tracking_id, commit_id)) => (tracking_id, Some(commit_id.to_string())),
        None => (rest, None),
    };
    let tracking_id = percent_decode_str(tracking_id).decode_utf8().ok()?;
    (!domain.is_empty() && !tracking_id.is_empty()).then(|| AdvisoryRef {
        domain: domain.to_string(),
        tracking_id: tracking_id.into_owned(),
        commit_id: commit_id.filter(|c| !c.is_empty()),
    })
}

/// Reads an advisory resource.
pub async fn read(state: &AppState, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let Some(advisory) = parse(uri) else {
        return Err(ErrorData::invalid_params(
            format!("Expected csaf://{{domain}}/{{tracking_id}}[@{{commit_id}}], got {uri}"),
            None,
        ));
    };
    let raw = state
        .storage
        .read_document_raw(
            &advisory.domain,
            &advisory.tracking_id,
            advisory.commit_id.as_deref(),
            MAX_RAW_BYTES,
        )
        .await
        .map_err(internal)?;
    let text = match raw {
        Some(RawDocument::Json(text)) => text,
        Some(RawDocument::TooLarge { size }) => {
            return Err(ErrorData::invalid_request(
                format!(
                    "Advisory is {} (limit {}); use get_document_content instead",
                    ByteSize(size as u64),
                    ByteSize(MAX_RAW_BYTES as u64)
                ),
                None,
            ));
        }
        None => {
            return Err(ErrorData::resource_not_found(
                format!("No advisory at {uri}"),
                None,
            ));
        }
    };
    Ok(ReadResourceResult::new(vec![
        ResourceContents::text(text, uri).with_mime_type("application/json"),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

    /// Characters a client must escape in a tracking ID to keep it a single URI segment.
    const SEGMENT: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'/')
        .add(b'?')
        .add(b'@');

    /// Builds the resource URI of an advisory, optionally at a specific version.
    fn uri(domain: &str, tracking_id: &str, commit_id: Option<&str>) -> String {
        let tracking_id = utf8_percent_encode(tracking_id, SEGMENT);
        match commit_id {
            Some(commit_id) => format!("{SCHEME}{domain}/{tracking_id}@{commit_id}"),
            None => format!("{SCHEME}{domain}/{tracking_id}"),
        }
    }

    #[test]
    fn round_trips_tracking_ids() {
        for tracking_id in ["RHSA-2024:1234", "a/b@c d%", "cisco-sa-x"] {
            for commit_id in [None, Some("abc123")] {
                let uri = uri("example.com", tracking_id, commit_id);
                assert_eq!(
                    parse(&uri),
                    Some(AdvisoryRef {
                        domain: "example.com".into(),
                        tracking_id: tracking_id.into(),
                        commit_id: commit_id.map(Into::into),
                    }),
                    "{uri}"
                );
            }
        }
    }

    #[test]
    fn rejects_malformed_uris() {
        for uri in [
            "https://example.com/x",
            "csaf://example.com",
            "csaf:///x",
            "csaf://example.com/",
        ] {
            assert_eq!(parse(uri), None, "{uri}");
        }
    }
}
