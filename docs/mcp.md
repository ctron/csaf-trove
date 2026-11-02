# MCP server

csaf-trove includes an optional, read-only Model Context Protocol server. It shares the
HTTP listener and stored data with the dashboard and REST API; no separate process is needed.

## Configuration

Add to the server TOML configuration and restart:

```toml
[mcp]
enabled = true
```

The endpoint is disabled by default. Use **Streamable HTTP** with the URL
`http://127.0.0.1:8080/mcp` in your MCP client. The service is stateless and returns JSON responses.

By default, only loopback hosts are accepted in the HTTP `Host` header. For a remote deployment,
set the hostnames (or `host:port` values) clients will use:

```toml
[mcp]
enabled = true
allowed_hosts = ["csaf.example.com"]
```

Connect to `https://csaf.example.com/mcp` through your TLS reverse proxy. An empty
`allowed_hosts` list is rejected at startup. Host filtering is DNS-rebinding protection,
not client authentication: the endpoint exposes read access without a bearer token.
There are no tools for triggering syncs, changing configuration or deleting providers.

See the [deployment guide](../deploy/DEPLOY.md#mcp-endpoint) for Ansible settings.
The dashboard's Trunk development proxy only forwards `/api`; connect MCP clients directly
to the server on port 8080 during development.

## Tools

| Tool | Purpose |
|---|---|
| `find_advisories_by_cve` | Find advisories across providers by CVE, with optional prefix matching |
| `get_overview` | Instance-wide statistics |
| `list_providers`, `get_provider` | Provider summaries and details |
| `list_documents`, `get_document` | Document listings and validation metadata |
| `get_document_content` | Current advisory content |
| `list_document_versions`, `get_document_version` | Version history and historical metadata |
| `diff_document_version` | Changed lines between a version and the next newer version |
| `get_sync_history`, `get_sync_status` | Past sync runs and current progress |

Paginated tools accept `offset` and `limit`; limits are capped at 200.
Diffs are truncated to 1,000 changed lines.

## Advisory resources

Clients can read raw advisory JSON using these resource templates:

- `csaf://{domain}/{tracking_id}` — current version
- `csaf://{domain}/{tracking_id}@{commit_id}` — historical version

Percent-encode the provider identifier and tracking ID. Use commit IDs returned by
`list_document_versions`. Resources are limited to 4 MiB; larger documents must be accessed
through tools.

MCP requests share the server's per-client rate limiting with the REST API and aggregator
routes: by default, 60 requests per minute, a burst of 60, and up to 10 seconds of waiting
before `429 Too Many Requests`. See the [deployment guide](../deploy/DEPLOY.md#rate-limiting)
for tuning.
