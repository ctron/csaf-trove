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
