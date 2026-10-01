use std::{
    fs::{canonicalize, read},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use anyhow::{Context, Error, Result, anyhow, ensure};
use bytes::Bytes;
use csaf_walker::{
    discover::{DiscoveredAdvisory, DiscoveredContext, DiscoveredVisitor, DistributionContext},
    model::metadata::{self, Distribution, ProviderMetadata},
    retrieve::RetrievedAdvisory,
    source::Source,
};
use time::OffsetDateTime;
use tokio::{
    fs,
    sync::mpsc::{Receiver, channel},
    task::spawn_blocking,
};
use url::Url;
use walkdir::{DirEntry, Error as WalkError, WalkDir};
use walker_common::{
    retrieve::RetrievalMetadata,
    source::file::read_sig_and_digests,
    utils::openpgp::PublicKey,
    validate::source::{Key, KeySource, KeySourceError},
};

use super::store::DIR_METADATA;
use crate::storage::scratch;

/// Reads CSAF documents from a `<domain>/<url_path>` layout on disk.
#[derive(Clone, Debug)]
pub struct TroveFileSource {
    /// Absolute path to the worktree root.
    base: PathBuf,
}

impl TroveFileSource {
    /// Creates a new source rooted at `base`.
    pub fn new(base: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            base: canonicalize(base)?,
        })
    }

    /// Visits the prepared directory through a bounded queue instead of building a full index.
    pub async fn walk_prepared(&self, visitor: impl DiscoveredVisitor) -> Result<()> {
        let metadata = self.load_metadata().await?;
        let context = visitor
            .visit_context(&DiscoveredContext {
                metadata: &metadata,
            })
            .await
            .map_err(|error| anyhow!("Validation context failed: {error}"))?;
        let distribution = Arc::new(DistributionContext::Directory(
            Url::from_directory_path(&self.base)
                .map_err(|()| anyhow!("Invalid validation root"))?,
        ));
        let mut entries = self.walk_distribution(distribution.clone())?;
        while let Some(entry) = entries.recv().await {
            let entry = entry?;
            if !entry.file_type().is_file() || !scratch::is_advisory(entry.path()) {
                continue;
            }
            let path = scratch::logical_path(entry.path());
            if path != entry.path() {
                ensure!(
                    !path.exists(),
                    "Both plain and compressed scratch files exist: {}",
                    path.display()
                );
            }
            let advisory = DiscoveredAdvisory {
                url: Url::from_file_path(&path)
                    .map_err(|()| anyhow!("Invalid advisory path: {}", path.display()))?,
                modified: SystemTime::UNIX_EPOCH,
                digest: None,
                signature: None,
                context: distribution.clone(),
            };
            visitor
                .visit_advisory(&context, advisory)
                .await
                .map_err(|error| anyhow!("Validation visitor failed: {error}"))?;
        }
        Ok(())
    }

    /// Scans `metadata/keys/` and returns key entries.
    async fn scan_keys(&self) -> Result<Vec<metadata::Key>> {
        let dir = self.base.join(DIR_METADATA).join("keys");
        let mut result = Vec::new();

        let mut entries = match fs::read_dir(&dir).await {
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(result),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("Failed scanning for keys: {}", dir.display()));
            }
            Ok(entries) => entries,
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                result.push(metadata::Key {
                    fingerprint: None,
                    url: Url::from_file_path(&path)
                        .map_err(|()| anyhow!("Failed to build file URL: {}", path.display()))?,
                });
            }
        }

        Ok(result)
    }

    /// Walks a distribution directory for plain or zstd-compressed advisories.
    fn walk_distribution(
        &self,
        context: Arc<DistributionContext>,
    ) -> Result<Receiver<Result<DirEntry, WalkError>>> {
        let (tx, rx) = channel(8);

        let path = context
            .url()
            .clone()
            .to_file_path()
            .map_err(|()| anyhow!("Failed to convert to path: {:?}", context.url()))?;

        if !path.exists() {
            tracing::debug!(
                "Distribution directory does not exist, skipping: {}",
                path.display()
            );
            return Ok(rx);
        }

        let metadata_dir = self.base.join(DIR_METADATA);
        spawn_blocking(move || {
            for entry in WalkDir::new(path).into_iter().filter_entry(|entry| {
                entry.path() != metadata_dir
                    && (!entry.file_type().is_file() || scratch::is_advisory(entry.path()))
            }) {
                if tx.blocking_send(entry).is_err() {
                    return;
                }
            }
        });

        Ok(rx)
    }
}

impl walker_common::source::Source for TroveFileSource {
    type Error = Error;
    type Retrieved = RetrievedAdvisory;
}

impl Source for TroveFileSource {
    async fn load_metadata(&self) -> Result<ProviderMetadata, Self::Error> {
        let metadata_file = self.base.join(DIR_METADATA).join("provider-metadata.json");
        let data = read(&metadata_file)
            .with_context(|| format!("Failed to read metadata: {}", metadata_file.display()))?;

        let mut metadata: ProviderMetadata =
            serde_json::from_slice(&data).context("Failed to parse provider metadata")?;

        metadata.public_openpgp_keys = self.scan_keys().await?;

        metadata.distributions = vec![Distribution {
            directory_url: Some(
                Url::from_directory_path(&self.base)
                    .map_err(|()| anyhow!("Invalid validation root"))?,
            ),
            rolie: None,
        }];

        Ok(metadata)
    }

    async fn load_index(
        &self,
        context: DistributionContext,
    ) -> Result<Vec<DiscoveredAdvisory>, Self::Error> {
        let context = Arc::new(context);
        let mut entries = self.walk_distribution(context.clone())?;
        let mut result = vec![];

        while let Some(entry) = entries.recv().await {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if !scratch::is_advisory(path) {
                continue;
            }

            let logical_path = scratch::logical_path(path);
            if logical_path != path {
                ensure!(
                    !logical_path.exists(),
                    "Both plain and compressed scratch files exist: {}",
                    logical_path.display()
                );
            }
            let url = Url::from_file_path(&logical_path)
                .map_err(|()| anyhow!("Failed to convert to URL: {}", path.display()))?;
            let modified = path.metadata()?.modified()?;

            result.push(DiscoveredAdvisory {
                url,
                modified,
                digest: None,
                signature: None,
                context: context.clone(),
            });
        }

        Ok(result)
    }

    async fn load_advisory(
        &self,
        discovered: DiscoveredAdvisory,
    ) -> Result<RetrievedAdvisory, Self::Error> {
        let path = discovered
            .url
            .to_file_path()
            .map_err(|()| anyhow!("Unable to convert URL to path: {}", discovered.url))?;

        let compressed = scratch::compressed_path(&path);
        let physical = if fs::try_exists(&compressed).await? {
            compressed
        } else {
            path.clone()
        };
        let read_path = physical.clone();
        let data = Bytes::from(spawn_blocking(move || scratch::read(&read_path)).await??);
        let (signature, sha256, sha512) = read_sig_and_digests(&path, &data).await?;

        let last_modification = physical
            .metadata()
            .ok()
            .and_then(|md| md.modified().ok())
            .map(OffsetDateTime::from);

        Ok(RetrievedAdvisory {
            discovered,
            data,
            signature,
            sha256,
            sha512,
            metadata: RetrievalMetadata {
                last_modification,
                etag: None,
            },
        })
    }
}

impl KeySource for TroveFileSource {
    type Error = Error;

    async fn load_public_key(
        &self,
        key: Key<'_>,
    ) -> Result<PublicKey, KeySourceError<Self::Error>> {
        let path = key
            .url
            .to_file_path()
            .map_err(|()| anyhow!("Failed to convert key URL to path: {}", key.url))
            .map_err(KeySourceError::Source)?;

        let bytes = fs::read(&path)
            .await
            .map_err(|err| KeySourceError::Source(err.into()))?;

        walker_common::utils::openpgp::validate_keys(bytes.into(), key.fingerprint)
            .map_err(KeySourceError::OpenPgp)
    }
}

#[cfg(test)]
mod tests;
