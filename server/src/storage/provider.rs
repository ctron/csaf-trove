use crate::models::source::sanitize_domain;
use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tokio::fs;

/// Filesystem-safe key of a provider, derived from its domain or metadata URL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProviderKey(String);

impl From<&str> for ProviderKey {
    fn from(domain: &str) -> Self {
        Self(sanitize_domain(domain))
    }
}

impl AsRef<str> for ProviderKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for ProviderKey {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for ProviderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One provider's directory below a data root, e.g. `results/<key>`.
#[derive(Clone, Debug)]
pub struct ProviderDir(PathBuf);

impl ProviderDir {
    /// Accepts a raw domain or an existing [`ProviderKey`], as sanitizing is idempotent.
    pub fn new(root: impl AsRef<Path>, domain: impl AsRef<str>) -> Self {
        Self(root.as_ref().join(ProviderKey::from(domain.as_ref())))
    }

    /// Returns the directory path without touching the filesystem.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Returns the path of a file in this directory without touching the filesystem.
    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Creates the directory if needed and returns its path.
    pub async fn ensure(&self) -> Result<&Path> {
        fs::create_dir_all(&self.0).await?;
        Ok(&self.0)
    }

    /// Reads a JSON file from this directory, returning `None` if it does not exist.
    pub async fn read_json<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        read_json(&self.file(name)).await
    }

    /// Writes a JSON file into this directory, creating the directory if needed.
    pub async fn write_json<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<()> {
        write_json(&self.file(name), value).await
    }
}

/// Reads a JSON file, returning `None` if it does not exist.
pub async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path).await {
        Ok(data) => Ok(Some(serde_json::from_slice(&data)?)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes a value as pretty JSON, creating the parent directory first.
pub async fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys are sanitized from metadata URLs and stable when sanitized again.
    #[test]
    fn key_sanitizes_and_is_idempotent() {
        let key = ProviderKey::from("https://example.com/.well-known/csaf/provider-metadata.json");
        assert_eq!(
            key.as_ref() as &str,
            "example.com-.well-known-csaf-provider-metadata.json"
        );
        assert_eq!(ProviderKey::from(key.as_ref() as &str), key);
    }

    /// Missing files read as `None`, and writing creates the provider directory.
    #[tokio::test]
    async fn json_round_trip_creates_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = ProviderDir::new(root.path(), "https://example.com/");
        assert_eq!(dir.path(), root.path().join("example.com"));
        assert_eq!(dir.read_json::<Vec<u32>>("a.json").await.unwrap(), None);

        dir.write_json("a.json", &[1, 2]).await.unwrap();
        assert_eq!(
            dir.read_json::<Vec<u32>>("a.json").await.unwrap(),
            Some(vec![1, 2])
        );
    }
}
