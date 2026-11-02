# csaf-trove [![CI](https://github.com/ctron/csaf-trove/actions/workflows/ci.yml/badge.svg)](https://github.com/ctron/csaf-trove/actions/workflows/ci.yml)

A [CSAF](https://docs.oasis-open.org/csaf/csaf/v2.0/csaf-v2.0.html) security advisory aggregator that syncs,
validates, and tracks advisories from multiple providers.

A public instance is running at [csaf.dentrassi.de](https://csaf.dentrassi.de).

## What it does

csaf-trove periodically fetches CSAF documents from configured providers, validates them against the CSAF profiles
(basic, extended, full), and presents the results through a web dashboard.

- **Multi-provider sync** — incremental and full sync with per-document error tracking
- **Validation** — CSAF profile validation with per-document results and history
- **Version tracking** — document history and diffs between versions
- **CVE search** — finds matching advisories across providers
- **REST API and MCP server** — read access for integrations and AI clients; MCP is opt-in
- **Web dashboard** — provider summaries, document details, and sync activity sparklines
- **CSAF lister** — generates `aggregator.json` for use as a CSAF lister
- **GitHub config sync** — provider list managed via a GitHub repository with webhook support

## Running locally

Requires a current stable Rust toolchain, the `wasm32-unknown-unknown` target,
[Trunk](https://trunkrs.dev/), and OpenSSL development libraries with `pkg-config`
(on Debian/Ubuntu: `libssl-dev pkg-config`).

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

Create a `local-config.toml` in the repository root (it is ignored by Git). This example keeps all data in
`./data`, skips the GitHub config sync, and defines a provider inline:

```toml
[server]
listen = "127.0.0.1:8080"

[data]
dir = "data"

[scheduler]
sync_interval = "1d"
max_concurrent = 2

[[source]]
domain = "intevation.de"
```

Further `[[source]]` entries accept the same fields as the files in `sources/`. Instead of inline sources,
you can also copy files from `sources/` into `data/sources/`.

Build the dashboard first, since the server embeds `dashboard/dist`, then start the server:

```sh
trunk -C dashboard/Trunk.toml build
cargo run -p csaf-trove-server -- --config local-config.toml
```

The dashboard is then available at <http://127.0.0.1:8080>. For dashboard development, run `trunk -C dashboard/Trunk.toml serve`
from the repository root instead. It serves on <http://127.0.0.1:9090> and proxies API requests to the server on port 8080.

## MCP server

Enable the read-only MCP server in your configuration:

```toml
[mcp]
enabled = true
```

Connect an MCP client using Streamable HTTP at `http://127.0.0.1:8080/mcp`.
It provides CVE search, provider and document queries, validation results, version history and diffs.
See [MCP setup and capabilities](docs/mcp.md) for remote access and available tools.

## Documentation

- [Deployment](deploy/DEPLOY.md) — Ansible, TLS, API authentication, webhooks and rate limiting
- [Operations](docs/operations.md) — scratch storage and refreshing cached summaries
- [REST API routes](server/src/api/mod.rs) — providers, documents, CVE search and sync control under `/api`

CSAF lister output is optional: configure `[aggregator]` to publish
`/.well-known/csaf-aggregator/aggregator.json`. Full mirror hosting remains a
[design proposal](docs/design-aggregator-mirror.md).
