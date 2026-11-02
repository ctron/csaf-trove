# Operations

## Scratch storage

Scratch advisories in `work/` use zstd compression (`.json.zst`, level 3), including during full syncs
and revalidation. Signatures, checksums, and provider metadata remain plain files. Validation and Git
insertion decompress individual advisories in memory; Git retains the original JSON bytes and paths,
so document history and diffs remain compatible with existing repositories. No configuration or
repository migration is required.

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
