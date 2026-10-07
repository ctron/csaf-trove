# csaf-trove [![CI](https://github.com/ctron/csaf-trove/actions/workflows/ci.yml/badge.svg)](https://github.com/ctron/csaf-trove/actions/workflows/ci.yml)

A [CSAF](https://docs.oasis-open.org/csaf/csaf/v2.0/csaf-v2.0.html) security advisory aggregator that syncs,
validates, and tracks advisories from multiple providers.

A public instance is running at [csaf.dentrassi.de](https://csaf.dentrassi.de).

## What it does

csaf-trove periodically fetches CSAF documents from configured providers, validates them against the CSAF profiles
(basic, extended, full), and presents the results through a web dashboard.

- **Multi-provider sync** — incremental and full sync with per-document error tracking
- **Validation** — CSAF profile validation with per-document results and history
- **Version tracking** — tracks document changes over time
- **Web dashboard** — provider summaries, document details, and sync activity sparklines
- **CSAF lister** — generates `aggregator.json` for use as a CSAF lister
- **GitHub config sync** — provider list managed via a GitHub repository with webhook support

Scratch advisories in `work/` use zstd compression (`.json.zst`, level 3), including during full syncs
and revalidation. Signatures, checksums, and provider metadata remain plain files. Validation and Git
insertion decompress individual advisories in memory; Git retains the original JSON bytes and paths,
so document history and diffs remain compatible with existing repositories. No configuration or
repository migration is required.

## Running locally

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
(cd dashboard && trunk build)
cargo run -p csaf-trove-server -- --config local-config.toml
```

The dashboard is then available at <http://127.0.0.1:8080>. For dashboard development, run `trunk serve` in
`dashboard/` instead. It serves on <http://127.0.0.1:9090> and proxies API requests to the server on port 8080.

## Refreshing cached summaries

Provider pages serve cached summaries. Syncs rebuild them when results change or the cache is missing;
unchanged syncs reuse existing summaries, including older summary formats.

To upgrade selected summaries once using stored validation results, stop the service and run:

```sh
csaf-trove-server --config /etc/csaf-trove/config.toml --refresh-summaries suse.com vulnerabilities.ncsc.nl
```

Run this as the service user so file ownership is preserved, then restart the service. The command
processes providers sequentially and exits without starting the HTTP server, scheduler, downloads, or
validation. It preserves the last validation timestamp and operator note. Each provider must already
have a cached summary and validation database. Stop the service first to avoid racing a sync's summary
publication.
