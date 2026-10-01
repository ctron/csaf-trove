## Code

* Check with `clippy` for all targets
* Ensure everything is formatted with `cargo fmt`
* Nest imports
* Don't use fully qualified types, functions, ... import them. Unless that creates a clash, then try to limit the prefix (e.g. `my_entity::Entity`)
* Document all functions, fields, variants, ... with a concise rustdoc snippet
* Use `thiserror` for creating custom errors
* Use `anyhow` for startup errors

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
