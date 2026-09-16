use std::{fmt::Debug, io::ErrorKind, path::PathBuf};

use anyhow::Context;
use csaf_walker::{
    common::retrieve::RetrievalError,
    discover::DiscoveredAdvisory,
    model::metadata::ProviderMetadata,
    retrieve::{RetrievalContext, RetrievedAdvisory, RetrievedVisitor},
    source::Source,
};
use tokio::fs;
use walker_common::{
    store::{Document, StoreError, store_document},
    utils::{openpgp::PublicKey, url::Urlify},
};

/// Stores CSAF documents under a clean `<domain>/<url_path>` layout.
pub struct TroveStoreVisitor {
    /// Output base directory (the worktree root).
    base: PathBuf,
}

/// Errors that can occur during storage.
#[derive(Debug, thiserror::Error)]
pub enum TroveStoreError<S: Source> {
    /// Failed to store a document or metadata.
    #[error("{0:#}")]
    Store(#[from] StoreError),
    /// Failed to retrieve an advisory from the source.
    #[error("retrieval of {} failed: {inner}", inner.url())]
    Retrieval {
        /// The underlying retrieval error.
        inner: Box<RetrievalError<DiscoveredAdvisory, S>>,
    },
    /// General I/O or processing error.
    #[error("{0:#}")]
    Io(anyhow::Error),
}

pub const DIR_METADATA: &str = "metadata";

impl TroveStoreVisitor {
    /// Creates a new visitor that stores files under `base`.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// Converts an advisory URL to a local filesystem path: `<base>/<domain>/<url_path>`.
    fn advisory_path(&self, url: &url::Url) -> Option<PathBuf> {
        let domain = url.host_str()?;
        let path = url.path().trim_start_matches('/');
        if path.is_empty() {
            return None;
        }
        Some(self.base.join(domain).join(path))
    }

    /// Writes the provider metadata JSON to `metadata/provider-metadata.json`.
    async fn store_provider_metadata(&self, metadata: &ProviderMetadata) -> anyhow::Result<()> {
        let metadir = self.base.join(DIR_METADATA);

        fs::create_dir(&metadir)
            .await
            .or_else(|err| match err.kind() {
                ErrorKind::AlreadyExists => Ok(()),
                _ => Err(err),
            })
            .with_context(|| {
                format!("Failed to create metadata directory: {}", metadir.display())
            })?;

        let file = metadir.join("provider-metadata.json");
        let data =
            serde_json::to_vec_pretty(metadata).context("Failed to serialize provider metadata")?;
        fs::write(&file, data)
            .await
            .with_context(|| format!("Failed to write provider metadata: {}", file.display()))?;

        Ok(())
    }

    /// Writes PGP public keys to `metadata/keys/`.
    async fn store_keys(&self, keys: &[PublicKey]) -> anyhow::Result<()> {
        let dir = self.base.join(DIR_METADATA).join("keys");
        fs::create_dir(&dir)
            .await
            .or_else(|err| match err.kind() {
                ErrorKind::AlreadyExists => Ok(()),
                _ => Err(err),
            })
            .with_context(|| format!("Failed to create keys directory: {}", dir.display()))?;

        for (i, key) in keys.iter().enumerate() {
            let name = dir.join(format!("{i}.key"));
            fs::write(&name, &key.raw)
                .await
                .with_context(|| format!("Failed to write key: {}", name.display()))?;
        }

        Ok(())
    }
}

impl<S: Source + Debug> RetrievedVisitor<S> for TroveStoreVisitor
where
    S::Error: 'static,
{
    type Error = TroveStoreError<S>;
    type Context = ();

    async fn visit_context(
        &self,
        context: &RetrievalContext<'_>,
    ) -> Result<Self::Context, Self::Error> {
        self.store_provider_metadata(context.metadata)
            .await
            .map_err(TroveStoreError::Io)?;
        self.store_keys(context.keys)
            .await
            .map_err(TroveStoreError::Io)?;
        Ok(())
    }

    async fn visit_advisory(
        &self,
        _context: &Self::Context,
        result: Result<RetrievedAdvisory, RetrievalError<DiscoveredAdvisory, S>>,
    ) -> Result<(), Self::Error> {
        let advisory = match result {
            Ok(adv) => adv,
            Err(err) => {
                tracing::warn!("Failed to retrieve advisory {}: {err}", err.url());
                return Err(TroveStoreError::Retrieval {
                    inner: Box::new(err),
                });
            }
        };

        let file = self.advisory_path(&advisory.url).ok_or_else(|| {
            TroveStoreError::Io(anyhow::anyhow!(
                "Cannot derive file path from URL: {}",
                advisory.url
            ))
        })?;

        tracing::debug!("Storing: {} → {}", advisory.url, file.display());

        store_document(
            &file,
            Document {
                data: &advisory.data,
                changed: advisory.modified,
                metadata: &advisory.metadata,
                sha256: &advisory.sha256,
                sha512: &advisory.sha512,
                signature: &advisory.signature,
                no_timestamps: false,
                no_xattrs: false,
            },
        )
        .await?;

        Ok(())
    }
}
