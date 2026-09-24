//! Compression of disposable advisory files; Git and sidecars retain their original bytes.

use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};
use zstd::stream::{Encoder, decode_all, encode_all};

/// Compression level used for downloaded and checked-out advisories.
const COMPRESSION_LEVEL: i32 = 3;

/// Returns the physical filename for a compressed advisory.
pub fn compressed_path(path: &Path) -> PathBuf {
    path.with_added_extension("zst")
}

/// Maps compressed advisory filenames to the original URL and Git path.
pub fn logical_path(path: &Path) -> PathBuf {
    if path.to_string_lossy().ends_with(".json.zst") {
        path.with_extension("")
    } else {
        path.to_path_buf()
    }
}

/// Recognizes both compressed advisories and legacy uncompressed advisories.
pub fn is_advisory(path: &Path) -> bool {
    logical_path(path)
        .extension()
        .is_some_and(|ext| ext == "json")
}

/// Compresses the exact downloaded bytes without normalizing JSON.
pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    encode_all(data, COMPRESSION_LEVEL).context("Failed to compress scratch advisory")
}

/// Reads an actual scratch file, decoding compressed advisories only.
pub fn read(path: &Path) -> Result<Vec<u8>> {
    if logical_path(path) != path {
        decode_all(File::open(path)?)
            .with_context(|| format!("Failed to decompress scratch advisory: {}", path.display()))
    } else {
        fs::read(path).with_context(|| format!("Failed to read scratch file: {}", path.display()))
    }
}

/// Writes a Git blob directly to scratch, compressing advisories outside metadata.
pub fn write(root: &Path, relative: &Path, data: &[u8]) -> Result<()> {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().context("Scratch file has no parent")?)?;
    if relative.extension().is_some_and(|ext| ext == "json") && !relative.starts_with("metadata") {
        let path = compressed_path(&path);
        let mut encoder = Encoder::new(File::create(&path)?, COMPRESSION_LEVEL)?;
        encoder.write_all(data)?;
        encoder.finish()?;
    } else {
        fs::write(path, data)?;
    }
    Ok(())
}
