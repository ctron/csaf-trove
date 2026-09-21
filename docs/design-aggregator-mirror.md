# CSAF Aggregator Mirror Mode Design

This document describes how to extend csaf-trove's aggregator support from lister mode (Phase 1, already implemented) to full mirror mode. It covers the CSAF 2.0 spec requirements, the directory layout, and the implementation approach.

## Spec Requirements for Mirror Mode

A CSAF aggregator with `category: "aggregator"` must satisfy requirements 1-6 and 21-23 from the CSAF 2.0 specification (section 7.2.5):

| # | Requirement | Summary |
|---|-------------|---------|
| 1 | Valid CSAF document | All mirrored documents must be valid CSAF |
| 2 | Filename | Filenames must follow the CSAF naming convention |
| 3 | TLS | All content served over HTTPS |
| 4 | TLP:WHITE | Freely accessible without authentication |
| 5 | TLP:AMBER/RED | Access-controlled (not publicly accessible) |
| 6 | No redirects | Documents served directly, no HTTP 3xx |
| 18 | Integrity hashes | SHA-256/SHA-512 hash file per document |
| 19 | Signatures | OpenPGP detached signature per document |
| 20 | Public key | Signing key must be publicly available |
| 21 | aggregator.json | Must publish an aggregator.json listing providers |
| 22 | Two disjoint parties | At least 2 providers with different publisher namespaces |
| 23 | Mirror | Must host mirrored copies under its own domain |

Additionally:
- If the upstream provider has hash/signature files, they SHOULD be copied.
- If the upstream does NOT have them, the aggregator SHALL create them.
- A signature by the aggregator does NOT imply liability for the content; it confirms the document has not been modified after download.
- The aggregator MAY add additional signatures and hashes.

## Mirror Directory Layout

Per the spec, the mirror structure is organized by provider, with documents in year-based subdirectories:

```
<aggregator_dir>/
  aggregator.json
  <Provider_Name>/
    provider-metadata.json          # generated, local URLs
    feed-tlp-white.json             # ROLIE feed linking to local copies
    index.txt                       # list of all document paths
    changes.csv                     # path,timestamp pairs
    <YYYY>/
      <document>.json
      <document>.json.sha256
      <document>.json.sha512
      <document>.json.asc
  <Another_Provider>/
    ...
```

The `<Provider_Name>` directory name is derived from the publisher name, sanitized for filesystem and URL safety. Directories must be adjacent to `aggregator.json`.

## Provider Consent

An aggregator must check `mirror_on_CSAF_aggregators` in each provider's `provider-metadata.json` before mirroring. The lister-mode implementation already persists this flag in the `provider_info` SQLite table. The `aggregator_include` field on `Source` provides an operator override.

Special case: if `list_on_CSAF_aggregators: false` but `mirror_on_CSAF_aggregators: true`, the provider may only be listed if also mirrored.

## Implementation Approach

### Document Population

csaf-trove already stores all synced documents with their hash/signature sidecar files in bare git repos per provider. The mirror population step reads from these repos rather than re-downloading from upstream.

**Strategy: copy from git repo at generation time.** After all providers sync, the aggregator generator walks each eligible provider's bare git repo and copies documents into the mirror layout. This keeps the sync pipeline unchanged and the mirror output is a derived artifact that can be regenerated without re-syncing.

To save disk space, use hard links when the aggregator output directory is on the same filesystem as the git repos. Fall back to copies otherwise.

**Incremental updates:** Track a `mirror_commit` per provider in the `provider_info` table. On each generation cycle, compare the current HEAD commit with the last mirrored commit. If unchanged, skip the provider. If changed, diff the two commits and only update files that were added, modified, or removed.

### Per-Provider `provider-metadata.json`

For each mirrored provider, generate a new `provider-metadata.json` with:
- `canonical_url` pointing to `<mirror_base_url>/<Provider_Name>/provider-metadata.json`
- `distributions` with a `directory_url` pointing to the local mirror, and a ROLIE feed URL pointing to the local `feed-tlp-white.json`
- `list_on_CSAF_aggregators: false` and `mirror_on_CSAF_aggregators: false` (the mirror should not advertise itself as aggregatable)
- `publisher` information copied from the original
- `role` copied from the original
- `public_openpgp_keys` with a URL pointing to the aggregator's public key (if the aggregator signs documents) plus the original provider's keys

The existing `csaf_walker::model::metadata::ProviderMetadata` struct can be used directly for serialization.

### ROLIE Feed Generation

Each mirrored provider directory needs a ROLIE feed (`feed-tlp-white.json`) per TLP label. Most providers only have TLP:WHITE/UNLABELED content, so a single feed suffices in most cases.

Each feed entry needs:
- `id`: the document's tracking ID
- `title`: the document title
- `published` / `updated`: from the document's `current_release_date`
- `content.src`: local URL to the `.json` file
- `content.type`: `"application/json"`
- `link` entries with `rel: "hash"` (pointing to `.sha256`/`.sha512`) and `rel: "signature"` (pointing to `.asc`)
- `format.schema`: the CSAF JSON schema URL
- `format.version`: the CSAF version

csaf-walker's `csaf_walker::rolie` module defines `RolieFeed`, `Feed`, `Entry`, `Link`, `Content`, and `Format` structs. Evaluate whether these can be used for serialization or if custom types are needed.

### `index.txt` and `changes.csv`

- `index.txt`: one relative path per document, one per line, sorted alphabetically. Generated by walking the mirror directory.
- `changes.csv`: two columns `path,timestamp` (no header row). The timestamp is the document's `current_release_date` or the file's modification time. Sorted by timestamp descending.

These files go at the root of each provider's mirror directory (not per-year).

### PGP Signing

If a provider's documents lack `.asc` signature files, the aggregator must create them. This requires:

1. **Key management:** A PGP private key configured in `AggregatorConfig.signing_key_file`. The corresponding public key must be served at a stable URL.
2. **Signing:** For each document without an `.asc` file, generate a detached OpenPGP signature using the configured key.
3. **Dependency:** Add `sequoia-openpgp` to `server/Cargo.toml`. The `walker-common` crate already uses `sequoia-openpgp` for signature validation, so it's in the dependency tree.

```rust
/// Signs data and returns an ASCII-armored detached signature.
fn sign_detached(signing_key: &sequoia_openpgp::Cert, data: &[u8]) -> Result<Vec<u8>>
```

If no signing key is configured, skip providers whose documents lack signatures and log a warning.

### `aggregator.json` Updates

In mirror mode, each `CsafProviderEntry` in `csaf_providers` includes a `mirrors` array:

```json
{
  "metadata": {
    "url": "https://original-provider.com/.well-known/csaf/provider-metadata.json",
    "publisher": { ... },
    "last_updated": "..."
  },
  "mirrors": [
    "https://csaf.example.com/.well-known/csaf-aggregator/Original_Provider/provider-metadata.json"
  ]
}
```

For providers with `role: "csaf_publisher"` (not provider/trusted_provider), entries go in the `csaf_publishers` array instead. `csaf_publishers` entries additionally require:
- `mirrors`: (array, required)
- `update_interval`: (string, required) — use the scheduler's `sync_interval` formatted as humantime (e.g. `"daily"`, `"12h"`)

### HTTP Serving

Serve the entire aggregator output directory as static files using `actix-files::Files`:

```rust
.service(
    actix_files::Files::new(
        "/.well-known/csaf-aggregator",
        &aggregator_output_dir,
    )
    .prefer_utf8(true)
)
```

This replaces the single-route `aggregator.json` handler from lister mode. The `actix-files` crate needs to be added as a dependency.

The route must be registered before the SPA catch-all `ResourceFiles::new("/", ...)` in `main.rs` to take priority.

## Files to Create/Modify

| File | Change |
|------|--------|
| `server/Cargo.toml` | Add `actix-files`, `sequoia-openpgp` |
| `server/src/pipeline/aggregator.rs` | Add `populate_mirror()`, `build_mirror_metadata()`, per-provider generation |
| `server/src/pipeline/rolie.rs` (new) | ROLIE feed generation |
| `server/src/storage/documents.rs` | Add `mirror_commit` column to `provider_info` |
| `server/src/main.rs` | Replace single-route scope with `actix_files::Files` serving |
| `server/src/models/aggregator.rs` | May need `CsafPublisherEntry` type |

## Open Questions

1. **Provider name sanitization**: Use the publisher name or domain as the directory name? Publisher name is more human-readable but less stable. Domain is stable but less informative. Could use publisher name with a fallback to domain.

2. **TLP filtering**: Should the mirror only mirror TLP:WHITE documents? The spec says aggregators SHOULD only mirror TLP:WHITE unless they have explicit permission for higher levels. For the initial implementation, restrict to TLP:WHITE/UNLABELED.

3. **Document validation before mirroring**: Should the aggregator only mirror documents that pass basic validation? The spec doesn't require this, but mirroring known-invalid documents undermines trust. Consider making this configurable.

4. **Disk space management**: Mirror mode roughly doubles storage per provider. Options:
   - Hard links (same filesystem only)
   - Configurable retention (e.g. only mirror last N years)
   - Configurable per-provider mirror opt-in at the source level

5. **Concurrency with sync**: The aggregator generator reads from git repos that may be written to by concurrent sync jobs. Git atomic operations make this safe in practice, but the generated output may be momentarily inconsistent during a long sync cycle. Consider acquiring a read lock or using git snapshots.

6. **Dashboard integration**: Add an aggregator status page showing: number of providers listed/mirrored, which providers were included/excluded and why, last generation timestamp, mirror disk usage.
