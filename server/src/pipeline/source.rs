use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, anyhow};
use bytes::Bytes;
use csaf_walker::{
    discover::{DiscoveredAdvisory, DistributionContext},
    model::metadata::{self, ProviderMetadata},
    retrieve::RetrievedAdvisory,
    source::Source,
};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use url::Url;
use walkdir::WalkDir;
use walker_common::{
    retrieve::RetrievalMetadata,
    source::file::read_sig_and_digests,
    utils::openpgp::PublicKey,
    validate::source::{Key, KeySource, KeySourceError},
};

use super::store::DIR_METADATA;

/// Reads CSAF documents from a `<domain>/<url_path>` layout on disk.
#[derive(Clone, Debug)]
pub struct TroveFileSource {
    /// Absolute path to the worktree root.
    base: PathBuf,
}

impl TroveFileSource {
    /// Creates a new source rooted at `base`.
    pub fn new(base: impl AsRef<Path>) -> anyhow::Result<Self> {
        Ok(Self {
            base: std::fs::canonicalize(base)?,
        })
    }

    /// Maps an HTTP(S) distribution URL to the corresponding local directory.
    fn url_to_local_dir(&self, url_str: &str) -> anyhow::Result<PathBuf> {
        let parsed =
            Url::parse(url_str).with_context(|| format!("Invalid distribution URL: {url_str}"))?;
        let domain = parsed
            .host_str()
            .ok_or_else(|| anyhow!("Distribution URL has no host: {url_str}"))?;
        let path = parsed.path().trim_start_matches('/');
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            Ok(self.base.join(domain))
        } else {
            Ok(self.base.join(domain).join(trimmed))
        }
    }

    /// Scans `metadata/keys/` and returns key entries.
    async fn scan_keys(&self) -> anyhow::Result<Vec<metadata::Key>> {
        let dir = self.base.join(DIR_METADATA).join("keys");
        let mut result = Vec::new();

        let mut entries = match tokio::fs::read_dir(&dir).await {
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

    /// Walks a distribution directory for `.json` files.
    fn walk_distribution(
        &self,
        context: Arc<DistributionContext>,
    ) -> anyhow::Result<mpsc::Receiver<walkdir::Result<walkdir::DirEntry>>> {
        let (tx, rx) = mpsc::channel(8);

        let path = context
            .url()
            .clone()
            .to_file_path()
            .map_err(|()| anyhow!("Failed to convert to path: {:?}", context.url()))?;

        tokio::task::spawn_blocking(move || {
            for entry in WalkDir::new(path).into_iter().filter_entry(|entry| {
                !entry.file_type().is_file()
                    || entry.file_name().to_string_lossy().ends_with(".json")
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
    type Error = anyhow::Error;
    type Retrieved = RetrievedAdvisory;
}

impl Source for TroveFileSource {
    async fn load_metadata(&self) -> Result<ProviderMetadata, Self::Error> {
        let metadata_file = self.base.join(DIR_METADATA).join("provider-metadata.json");
        let data = std::fs::read(&metadata_file)
            .with_context(|| format!("Failed to read metadata: {}", metadata_file.display()))?;

        let mut metadata: ProviderMetadata =
            serde_json::from_slice(&data).context("Failed to parse provider metadata")?;

        metadata.public_openpgp_keys = self.scan_keys().await?;

        for dist in &mut metadata.distributions {
            if let Some(ref directory_url) = dist.directory_url {
                let local_dir = self.url_to_local_dir(directory_url.as_str())?;
                dist.directory_url = Some(Url::from_directory_path(&local_dir).map_err(|()| {
                    anyhow!(
                        "Failed to convert directory to URL: {}",
                        local_dir.display()
                    )
                })?);
            }

            if let Some(ref mut rolie) = dist.rolie {
                for feed in &mut rolie.feeds {
                    let feed_str = feed.url.as_str();
                    let parsed = Url::parse(feed_str)?;
                    let domain = parsed
                        .host_str()
                        .ok_or_else(|| anyhow!("Feed URL has no host: {feed_str}"))?;
                    let path = parsed.path().trim_start_matches('/');
                    let parent = Path::new(path)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_str()
                        .unwrap_or("");
                    let local_dir = if parent.is_empty() {
                        self.base.join(domain)
                    } else {
                        self.base.join(domain).join(parent)
                    };
                    feed.url = Url::from_directory_path(&local_dir).map_err(|()| {
                        anyhow!(
                            "Failed to convert directory to URL: {}",
                            local_dir.display()
                        )
                    })?;
                }
            }
        }

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
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };
            if !name.ends_with(".json") {
                continue;
            }

            let url = Url::from_file_path(path)
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

        let data = Bytes::from(tokio::fs::read(&path).await?);
        let (signature, sha256, sha512) = read_sig_and_digests(&path, &data).await?;

        let last_modification = path
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
    type Error = anyhow::Error;

    async fn load_public_key(
        &self,
        key: Key<'_>,
    ) -> Result<PublicKey, KeySourceError<Self::Error>> {
        let path = key
            .url
            .to_file_path()
            .map_err(|()| anyhow!("Failed to convert key URL to path: {}", key.url))
            .map_err(KeySourceError::Source)?;

        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|err| KeySourceError::Source(err.into()))?;

        walker_common::utils::openpgp::validate_keys(bytes.into(), key.fingerprint)
            .map_err(KeySourceError::OpenPgp)
    }
}
