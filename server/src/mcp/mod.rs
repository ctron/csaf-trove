//! Read-only MCP (Model Context Protocol) endpoint exposing aggregator data to LLM clients.
mod resources;
#[cfg(test)]
mod test;
mod tools;

use crate::AppState;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{
        CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult,
        PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
        ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    tool_handler,
};
use rmcp_actix_web::transport::{LocalSessionManager, StreamableHttpService};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Instructions sent to clients during initialization.
const INSTRUCTIONS: &str = "\
csaf-trove aggregates CSAF security advisories from many providers (identified by domain), \
validates them against the CSAF profiles (basic, extended, full) and tracks their versions. \
Use find_advisories_by_cve to find advisories for a CVE across all providers. \
Use list_providers and get_provider for provider health, list_documents and get_document for \
per-document validation results, and get_document_content for an advisory's notes, \
vulnerabilities and scores. Document versions can be listed and diffed. \
Raw advisory JSON is available as the resource csaf://{domain}/{tracking_id}, with both \
identifiers percent-encoded (domain may be a full metadata URL). \
All data is read-only.";

/// MCP endpoint configuration.
#[derive(Debug, Default, Deserialize)]
pub struct McpConfig {
    /// Serve the read-only MCP endpoint at `/mcp`.
    #[serde(default)]
    pub enabled: bool,
    /// Hostnames (or `host:port`) accepted in the `Host` header; loopback only when omitted.
    #[serde(default)]
    pub allowed_hosts: Option<Vec<String>>,
}

/// MCP server handler backed by the shared application state.
#[derive(Clone)]
pub struct TroveMcp {
    /// Shared application state.
    state: Arc<AppState>,
    /// Routes tool calls to their handlers.
    tool_router: ToolRouter<Self>,
}

impl TroveMcp {
    /// Creates a handler for the given state.
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TroveMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("csaf-trove", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(
            resources::templates(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        resources::read(&self.state, &request.uri)
            .await
            .map(Into::into)
    }
}

/// Builds the stateless Streamable HTTP service, shared across all actix workers.
pub fn service(
    state: Arc<AppState>,
    config: &McpConfig,
) -> anyhow::Result<StreamableHttpService<TroveMcp, LocalSessionManager>> {
    if config.allowed_hosts.as_ref().is_some_and(Vec::is_empty) {
        // rmcp treats an empty list as "accept any host", silently disabling DNS-rebinding protection.
        anyhow::bail!("mcp.allowed_hosts must not be empty; omit it to allow loopback only");
    }
    Ok(StreamableHttpService::builder()
        .service_factory(Arc::new(move || Ok(TroveMcp::new(state.clone()))))
        .session_manager(Arc::new(LocalSessionManager::default()))
        .stateful_mode(false)
        .json_response(true)
        .maybe_allowed_hosts(config.allowed_hosts.clone())
        .build())
}

/// Serializes a value as the JSON text content of a successful tool result.
fn json_result(value: &impl Serialize) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::json(value)?]))
}

/// Returns a tool error result the model can read, e.g. for unknown providers or documents.
fn tool_error(message: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(message)]))
}

/// Maps an internal error to an MCP internal error.
fn internal(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}
