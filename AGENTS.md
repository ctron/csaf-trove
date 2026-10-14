## Code

* Check with `clippy` for all targets
* Ensure everything is formatted with `cargo fmt`
* Nest imports
* Don't use fully qualified types, functions, ... import them. Unless that creates a clash, then try to limit the prefix (e.g. `my_entity::Entity`)
* Document all functions, fields, variants, ... with a concise rustdoc snippet
* Use `thiserror` for creating custom errors
* Use `anyhow` for startup errors
* Bump `VALIDATION_REVISION` in `server/src/scheduler/runner.rs` when validation behavior or the stored result format changes, so existing results are revalidated

## Tools

* Try to avoid `gh api`, use the other `gh` commands
* Try to avoid -C for git
* Use conventional commits
* Use gitflow branches

## Release

* Increment the version in `Cargo.toml` and `deploy/roles/csaf-trove/defaults/main.yml`
* Run `cargo generate-lockfile`
* Commit and create a tag with a `v` prefix
* Push that tag

@RTK.md

<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->
