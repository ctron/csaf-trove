use crate::components::section_heading::{SectionHeading, SubHeading};
use leptos::prelude::*;

/// Path of the MCP endpoint.
const MCP_PATH: &str = "/mcp";
/// Path of the CSAF aggregator (lister) document.
const AGGREGATOR_PATH: &str = "/.well-known/csaf-aggregator/aggregator.json";

/// Read-only MCP tools with a short description each.
const MCP_TOOLS: &[(&str, &str)] = &[
    (
        "find_advisories_by_cve",
        "Find advisories across all providers that reference a CVE.",
    ),
    (
        "get_overview",
        "Provider and document counts, validation pass rates, health, activity, trends and storage.",
    ),
    (
        "list_providers",
        "All providers with document counts and validation results.",
    ),
    (
        "get_provider",
        "Validation summary, metrics history, sync runs and distribution health of a provider.",
    ),
    (
        "list_documents",
        "A provider's documents with validation results, optionally filtered by status.",
    ),
    (
        "get_document",
        "Metadata, validation results and revision history of a document.",
    ),
    (
        "get_document_content",
        "Notes, references and vulnerabilities (CVEs, CWEs, CVSS) of the current version.",
    ),
    (
        "list_document_versions",
        "Recorded versions of a document, newest first.",
    ),
    (
        "get_document_version",
        "Metadata of a historical document version.",
    ),
    (
        "diff_document_version",
        "Changes between a document version and the next newer one.",
    ),
    ("get_sync_history", "Past sync runs of a provider."),
    ("get_sync_status", "Current sync status of every provider."),
];

/// Returns the origin of the current page, e.g. `https://csaf.example.com`.
fn origin() -> String {
    web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default()
}

/// Describes the MCP and CSAF aggregator endpoints offered by this instance.
#[component]
pub fn EndpointsPage() -> impl IntoView {
    let origin = origin();
    let mcp_url = format!("{origin}{MCP_PATH}");
    let aggregator_url = format!("{origin}{AGGREGATOR_PATH}");
    let aggregator_href = aggregator_url.clone();
    let claude_command = format!("claude mcp add --transport http csaf-trove {mcp_url}");
    let client_config = format!(
        "{{\n  \"mcpServers\": {{\n    \"csaf-trove\": {{\n      \"type\": \"http\",\n      \"url\": \"{mcp_url}\"\n    }}\n  }}\n}}"
    );

    view! {
        <div>
            <SectionHeading>"Endpoints"</SectionHeading>
            <p class="text-sm text-gray-500 dark:text-gray-400 mb-6">
                "Besides this dashboard, csaf-trove offers machine-readable access to its data. Both endpoints are read-only, may be disabled by the operator, and are rate limited per client."
            </p>
            <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
                <Panel title="MCP endpoint">
                    <p class="text-gray-600 dark:text-gray-300 mb-4">
                        "A "
                        <a href="https://modelcontextprotocol.io" target="_blank" rel="noopener noreferrer">"Model Context Protocol"</a>
                        " server that lets LLM clients query providers, advisories, validation results, document history and sync status."
                    </p>
                    <Field label="URL"><Code>{mcp_url}</Code></Field>
                    <Field label="Transport">
                        <span class="text-gray-800 dark:text-white">"Streamable HTTP (stateless, JSON responses)"</span>
                    </Field>
                    <SubHeading>"Connecting a client"</SubHeading>
                    <p class="text-sm text-gray-600 dark:text-gray-300 mb-2">"Claude Code:"</p>
                    <Code block=true>{claude_command}</Code>
                    <p class="text-sm text-gray-600 dark:text-gray-300 mt-4 mb-2">"Clients using a JSON configuration:"</p>
                    <Code block=true>{client_config}</Code>
                    <SubHeading>"Tools"</SubHeading>
                    <dl class="divide-y divide-gray-200 dark:divide-gray-700">
                        {MCP_TOOLS.iter().map(|(name, description)| view! {
                            <div class="py-2">
                                <dt class="font-mono text-sm text-gray-800 dark:text-white">{*name}</dt>
                                <dd class="text-sm text-gray-500 dark:text-gray-400">{*description}</dd>
                            </div>
                        }).collect_view()}
                    </dl>
                    <SubHeading>"Resources"</SubHeading>
                    <p class="text-sm text-gray-600 dark:text-gray-300 mb-2">
                        "The raw JSON of an advisory, with both identifiers percent-encoded. Append "
                        <Code>"@{commit_id}"</Code>
                        " for a historical version."
                    </p>
                    <Code block=true>"csaf://{domain}/{tracking_id}"</Code>
                </Panel>
                <Panel title="Aggregator endpoint">
                    <p class="text-gray-600 dark:text-gray-300 mb-4">
                        "An "
                        <Code>"aggregator.json"</Code>
                        " document following the "
                        <a href="https://docs.oasis-open.org/csaf/csaf/v2.0/csaf-v2.0.html" target="_blank" rel="noopener noreferrer">"CSAF 2.0"</a>
                        " specification. It lists the eligible providers with a reference to their "
                        <Code>"provider-metadata.json"</Code>
                        ", so CSAF tooling can discover them through this instance."
                    </p>
                    <Field label="URL">
                        <a href=aggregator_href target="_blank" rel="noopener noreferrer" class="font-mono text-sm break-all">{aggregator_url}</a>
                    </Field>
                    <SubHeading>"Provider selection"</SubHeading>
                    <p class="text-sm text-gray-600 dark:text-gray-300">
                        "Providers are listed when their metadata allows listing on aggregators ("
                        <Code>"list_on_CSAF_aggregators"</Code>
                        "), unless the operator overrides this. The document is regenerated after providers sync."
                    </p>
                </Panel>
            </div>
        </div>
    }
}

/// Renders a titled card.
#[component]
fn Panel(title: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
            <h2 class="text-lg font-semibold text-gray-800 dark:text-white mb-4">{title}</h2>
            {children()}
        </div>
    }
}

/// Renders a labeled value on its own line.
#[component]
fn Field(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="flex flex-col sm:flex-row sm:items-baseline gap-x-4 gap-y-1 mb-2">
            <span class="text-sm text-gray-500 dark:text-gray-400 sm:w-24 shrink-0">{label}</span>
            <span class="min-w-0 break-all">{children()}</span>
        </div>
    }
}

/// Renders inline code, or a scrollable code block when `block` is set.
#[component]
fn Code(#[prop(optional)] block: bool, children: Children) -> impl IntoView {
    if block {
        view! {
            <pre class="bg-gray-100 dark:bg-gray-900 text-gray-800 dark:text-gray-200 rounded p-3 text-sm overflow-x-auto"><code>{children()}</code></pre>
        }
        .into_any()
    } else {
        view! {
            <code class="font-mono text-sm bg-gray-100 dark:bg-gray-900 text-gray-800 dark:text-gray-200 rounded px-1">{children()}</code>
        }
        .into_any()
    }
}
