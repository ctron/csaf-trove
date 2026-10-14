//! MCP tools; each one is a read-only view over the same storage used by the REST API.

use super::{TroveMcp, internal, json_result, tool_error};
use crate::{
    api::{overview, providers, sync},
    models::result::{DiffLineInfo, DiffTag},
};
use rmcp::{
    ErrorData,
    handler::server::wrapper::Parameters,
    model::CallToolResult,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};

/// Default page size for paginated tools.
const DEFAULT_LIMIT: u64 = 50;
/// Maximum page size for paginated tools, matching the REST API.
const MAX_LIMIT: u64 = 200;
/// Maximum number of rendered lines returned by the diff tool.
const MAX_DIFF_LINES: usize = 1000;

/// Parameters for the CVE search.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CveParams {
    /// CVE identifier, e.g. `CVE-2024-3094`; case-insensitive.
    pub cve: String,
    /// Match all CVE IDs starting with `cve` (e.g. `CVE-2024-30`) instead of exactly.
    #[serde(default)]
    pub prefix: bool,
}

/// Parameters identifying a provider.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProviderParams {
    /// Provider domain, as returned by `list_providers`.
    pub domain: String,
}

/// Parameters for a paginated per-provider listing.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PageParams {
    /// Provider domain.
    pub domain: String,
    /// Zero-based offset of the first item.
    #[serde(default)]
    pub offset: Option<u64>,
    /// Maximum number of items (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Parameters for listing a provider's documents.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocumentsParams {
    /// Provider domain.
    pub domain: String,
    /// Filter: `failing` (errors), `warnings` (warnings or errors), `passing`, `errors`
    /// (retrieval errors), `signature-errors`, `parse-errors`, `digest-errors`,
    /// `digest-warnings`, `missing-signatures`, `missing-digests` or `not-evaluated`.
    #[serde(default)]
    pub status: Option<String>,
    /// Zero-based offset of the first document.
    #[serde(default)]
    pub offset: Option<u64>,
    /// Maximum number of documents (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Parameters identifying a document.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocumentParams {
    /// Provider domain.
    pub domain: String,
    /// CSAF tracking ID of the document.
    pub tracking_id: String,
}

/// Parameters for a paginated listing of a document's versions.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VersionsParams {
    /// Provider domain.
    pub domain: String,
    /// CSAF tracking ID of the document.
    pub tracking_id: String,
    /// Zero-based offset of the first version.
    #[serde(default)]
    pub offset: Option<u64>,
    /// Maximum number of versions (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Parameters identifying a version of a document.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VersionParams {
    /// Provider domain.
    pub domain: String,
    /// CSAF tracking ID of the document.
    pub tracking_id: String,
    /// Commit ID of the version, as returned by `list_document_versions`.
    pub commit_id: String,
}

/// Rendered diff between two document versions.
#[derive(Debug, Serialize)]
struct DiffOutput {
    /// Number of inserted lines.
    inserted: usize,
    /// Number of deleted lines.
    deleted: usize,
    /// Changed lines prefixed with `+` or `-`; unchanged runs are collapsed.
    diff: String,
    /// Whether the rendered diff was cut off.
    truncated: bool,
}

/// Returns the effective offset and limit of a page request.
fn page(offset: Option<u64>, limit: Option<u64>) -> (u64, u64) {
    (
        offset.unwrap_or(0),
        limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
    )
}

/// Renders diff lines compactly, collapsing unchanged runs and capping the output.
fn render_diff(lines: &[DiffLineInfo]) -> DiffOutput {
    let inserted = lines
        .iter()
        .filter(|l| matches!(l.tag, DiffTag::Insert))
        .count();
    let deleted = lines
        .iter()
        .filter(|l| matches!(l.tag, DiffTag::Delete))
        .count();

    let mut rendered = Vec::new();
    let mut unchanged = 0;
    for line in lines {
        let prefix = match line.tag {
            DiffTag::Equal => {
                unchanged += 1;
                continue;
            }
            DiffTag::Insert => '+',
            DiffTag::Delete => '-',
        };
        if unchanged > 0 {
            rendered.push(format!("  ({unchanged} unchanged lines)"));
            unchanged = 0;
        }
        rendered.push(format!("{prefix} {}", line.content));
    }

    let truncated = rendered.len() > MAX_DIFF_LINES;
    rendered.truncate(MAX_DIFF_LINES);
    DiffOutput {
        inserted,
        deleted,
        diff: rendered.join("\n"),
        truncated,
    }
}

#[tool_router(vis = "pub(super)")]
impl TroveMcp {
    /// Finds advisories referencing a CVE across all providers.
    #[tool(
        description = "Find CSAF advisories across all providers that reference a CVE. Returns the provider domain, tracking ID, title, release date and CVSS score/severity of each match, newest first. `truncated` is set when the result was cut off.",
        annotations(read_only_hint = true)
    )]
    async fn find_advisories_by_cve(
        &self,
        Parameters(params): Parameters<CveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if params.cve.trim().is_empty() {
            return tool_error("`cve` must not be empty");
        }
        let result = self
            .state
            .storage
            .find_by_cve(&params.cve, params.prefix)
            .await
            .map_err(internal)?;
        json_result(&*result)
    }

    /// Returns instance-wide statistics.
    #[tool(
        description = "Get an overview of the whole aggregator: provider and document counts, validation pass rates, provider health, sync activity, weekly/monthly trends and storage usage.",
        annotations(read_only_hint = true)
    )]
    async fn get_overview(&self) -> Result<CallToolResult, ErrorData> {
        let overview = overview::build(&self.state).await.map_err(internal)?;
        json_result(&overview)
    }

    /// Lists all providers with their validation summaries.
    #[tool(
        description = "List all CSAF providers (by domain) with document counts, per-profile validation results, signature status, retrieval errors and most frequent failing tests.",
        annotations(read_only_hint = true)
    )]
    async fn list_providers(&self) -> Result<CallToolResult, ErrorData> {
        let providers = providers::list_summaries(&self.state)
            .await
            .map_err(internal)?;
        json_result(&providers)
    }

    /// Returns a provider's detail view.
    #[tool(
        description = "Get details for one provider: validation summary, metrics history, recent sync runs and per-distribution health.",
        annotations(read_only_hint = true)
    )]
    async fn get_provider(
        &self,
        Parameters(params): Parameters<ProviderParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match providers::load_detail(&self.state, params.domain.clone())
            .await
            .map_err(internal)?
        {
            Some(detail) => json_result(&detail),
            None => tool_error(format!("Unknown provider: {}", params.domain)),
        }
    }

    /// Lists a provider's documents with validation results.
    #[tool(
        description = "List a provider's documents with their per-profile validation results, optionally filtered by status (e.g. `failing`, `signature-errors`). Paginated.",
        annotations(read_only_hint = true)
    )]
    async fn list_documents(
        &self,
        Parameters(params): Parameters<DocumentsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (offset, limit) = page(params.offset, params.limit);
        match self
            .state
            .storage
            .load_documents_paginated(&params.domain, offset, limit, params.status.as_deref())
            .await
            .map_err(internal)?
        {
            Some(page) => json_result(&page),
            None => tool_error(format!("Unknown provider: {}", params.domain)),
        }
    }

    /// Returns a document's validation results.
    #[tool(
        description = "Get a document's metadata and validation results, including failing tests per profile, signature and digest outcomes, and revision history.",
        annotations(read_only_hint = true)
    )]
    async fn get_document(
        &self,
        Parameters(params): Parameters<DocumentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .state
            .storage
            .load_document(&params.domain, &params.tracking_id)
            .await
            .map_err(internal)?
        {
            Some(doc) => json_result(&doc),
            None => tool_error(format!(
                "Unknown document {} at {}",
                params.tracking_id, params.domain
            )),
        }
    }

    /// Returns a document's displayable content.
    #[tool(
        description = "Get the content of the current version of an advisory: TLP, publisher, notes, references and vulnerabilities with CVEs, CWEs, CVSS scores and product status counts.",
        annotations(read_only_hint = true)
    )]
    async fn get_document_content(
        &self,
        Parameters(params): Parameters<DocumentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .state
            .storage
            .read_document_content(&params.domain, &params.tracking_id)
            .await
            .map_err(internal)?
        {
            Some(content) => json_result(&content),
            None => tool_error(format!(
                "Unknown document {} at {}",
                params.tracking_id, params.domain
            )),
        }
    }

    /// Lists a document's versions.
    #[tool(
        description = "List the recorded versions of a document, newest first, with commit IDs, timestamps and tracking versions. Paginated.",
        annotations(read_only_hint = true)
    )]
    async fn list_document_versions(
        &self,
        Parameters(params): Parameters<VersionsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (offset, limit) = page(params.offset, params.limit);
        match self
            .state
            .storage
            .document_versions(&params.domain, &params.tracking_id, offset, limit)
            .await
            .map_err(internal)?
        {
            Some(versions) => json_result(&versions),
            None => tool_error(format!(
                "Unknown document {} at {}",
                params.tracking_id, params.domain
            )),
        }
    }

    /// Returns metadata of a historical document version.
    #[tool(
        description = "Get the metadata of a historical version of a document, identified by its commit ID.",
        annotations(read_only_hint = true)
    )]
    async fn get_document_version(
        &self,
        Parameters(params): Parameters<VersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .state
            .storage
            .read_historical_document(&params.domain, &params.tracking_id, &params.commit_id)
            .await
            .map_err(internal)?
        {
            Some(doc) => json_result(&doc),
            None => tool_error(format!(
                "Unknown version {} of {} at {}",
                params.commit_id, params.tracking_id, params.domain
            )),
        }
    }

    /// Diffs a document version against the next newer one.
    #[tool(
        description = "Show what changed between a document version (by commit ID) and the next newer version. Returns only changed lines of the pretty-printed JSON, prefixed with + or -; long diffs are truncated.",
        annotations(read_only_hint = true)
    )]
    async fn diff_document_version(
        &self,
        Parameters(params): Parameters<VersionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .state
            .storage
            .diff_document_versions(&params.domain, &params.tracking_id, &params.commit_id)
            .await
            .map_err(internal)?
        {
            Some(lines) => json_result(&render_diff(&lines)),
            None => tool_error(format!(
                "No newer version to compare {} of {} at {} with",
                params.commit_id, params.tracking_id, params.domain
            )),
        }
    }

    /// Returns a provider's sync history.
    #[tool(
        description = "List a provider's past sync runs, newest first, with timestamps, number of changed files and durations. Paginated.",
        annotations(read_only_hint = true)
    )]
    async fn get_sync_history(
        &self,
        Parameters(params): Parameters<PageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (offset, limit) = page(params.offset, params.limit);
        match self
            .state
            .storage
            .provider_history_paginated(&params.domain, offset, limit)
            .await
            .map_err(internal)?
        {
            Some(history) => json_result(&history),
            None => tool_error(format!("Unknown provider: {}", params.domain)),
        }
    }

    /// Returns the current sync status of all providers.
    #[tool(
        description = "Get the current sync status of every provider: running phase and progress, ETA, last completed run, errors and recent sync activity.",
        annotations(read_only_hint = true)
    )]
    async fn get_sync_status(&self) -> Result<CallToolResult, ErrorData> {
        json_result(&sync::build_status_entries(&self.state).await)
    }
}
