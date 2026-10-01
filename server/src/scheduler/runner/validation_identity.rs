//! Fingerprints the locked dependencies that implement document validation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Error as JsonError, to_vec};
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml::{de::Error as TomlError, from_str};

/// Invalid embedded lockfile or dependency reference.
#[derive(Debug, Error)]
pub(super) enum FingerprintError {
    /// The lockfile could not be decoded.
    #[error("Invalid validation lockfile: {0}")]
    Lockfile(#[from] TomlError),
    /// A reference must resolve to exactly one package.
    #[error("Validation dependency {0:?} is missing or ambiguous")]
    Dependency(String),
    /// The canonical dependency records could not be encoded.
    #[error("Cannot encode validation dependencies: {0}")]
    Encoding(#[from] JsonError),
}

/// Package records from Cargo's lockfile.
#[derive(Deserialize)]
struct Lockfile {
    /// All locked packages, including unrelated workspace dependencies.
    package: Vec<Package>,
}

/// Locked identity and outgoing dependencies of one package.
#[derive(Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct Package {
    /// Cargo package name.
    name: String,
    /// Exact locked version.
    version: String,
    /// Registry or Git source; absent for workspace packages.
    source: Option<String>,
    /// Registry content checksum, when present.
    checksum: Option<String>,
    /// Cargo dependency references, sorted before hashing.
    #[serde(default)]
    dependencies: Vec<String>,
}

/// Resolves Cargo's `name [version [(source)]]` dependency references uniquely.
fn resolve(packages: &[Package], reference: &str) -> Result<usize, FingerprintError> {
    let mut parts = reference.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let version = parts.next();
    let source = parts.next().map(|value| value.trim_matches(['(', ')']));
    let mut matches = packages.iter().enumerate().filter(|(_, package)| {
        package.name == name
            && version.is_none_or(|value| package.version == value)
            && source.is_none_or(|value| package.source.as_deref() == Some(value))
    });
    match (matches.next(), matches.next(), parts.next()) {
        (Some((index, _)), None, None) => Ok(index),
        _ => Err(FingerprintError::Dependency(reference.to_owned())),
    }
}

/// Hashes validation packages and their transitive closure independently of lockfile order.
pub(super) fn dependency_fingerprint(lockfile: &str) -> Result<String, FingerprintError> {
    let mut lockfile: Lockfile = from_str(lockfile)?;
    for package in &mut lockfile.package {
        package.dependencies.sort();
    }
    let packages = &lockfile.package;
    let mut pending = ["csaf-rs", "csaf-walker", "walker-common"]
        .into_iter()
        .map(|name| resolve(packages, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut selected = BTreeSet::new();
    while let Some(index) = pending.pop() {
        let package = &packages[index];
        if !selected.insert(package) {
            continue;
        }
        for dependency in &package.dependencies {
            pending.push(resolve(packages, dependency)?);
        }
    }
    Ok(hex::encode(Sha256::digest(to_vec(&selected)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small lockfile with an unrelated workspace package and two versions of a dependency.
    const LOCKFILE: &str = r#"
version = 4
[[package]]
name = "csaf-rs"
version = "0.5.0"
dependencies = ["parser 1.0.0 (registry+https://example.com)", "walker-common"]
[[package]]
name = "csaf-walker"
version = "0.19.0"
dependencies = ["csaf-rs", "walker-common"]
[[package]]
name = "walker-common"
version = "0.19.0"
[[package]]
name = "parser"
version = "1.0.0"
source = "registry+https://example.com"
checksum = "original"
[[package]]
name = "parser"
version = "2.0.0"
[[package]]
name = "csaf-trove-server"
version = "0.1.0"
dependencies = ["csaf-walker", "parser 2.0.0"]
"#;

    /// Only reachable package identities and relationships affect validation results.
    #[test]
    fn fingerprints_only_validation_dependencies() {
        let fingerprint = dependency_fingerprint(LOCKFILE).unwrap();
        for changed in [
            LOCKFILE.replace("0.1.0", "0.2.0"),
            LOCKFILE.replace("2.0.0", "3.0.0"),
            LOCKFILE.replace(
                "[\"csaf-rs\", \"walker-common\"]",
                "[\"walker-common\", \"csaf-rs\"]",
            ),
            LOCKFILE
                .split("[[package]]")
                .skip(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .fold(String::from("version = 4\n"), |mut result, package| {
                    result.push_str("[[package]]");
                    result.push_str(package);
                    result
                }),
        ] {
            assert_eq!(fingerprint, dependency_fingerprint(&changed).unwrap());
        }
        for changed in [
            LOCKFILE.replace("0.5.0", "0.6.0"),
            LOCKFILE.replace("1.0.0", "1.1.0"),
            LOCKFILE.replace("original", "updated"),
            LOCKFILE.replace("registry+https://example.com", "registry+https://other.com"),
            LOCKFILE.replace(
                "dependencies = [\"csaf-rs\", \"walker-common\"]",
                "dependencies = [\"csaf-rs\"]",
            ),
        ] {
            assert_ne!(fingerprint, dependency_fingerprint(&changed).unwrap());
        }
    }

    /// Missing and ambiguous references must fail instead of silently omitting dependencies.
    #[test]
    fn rejects_unresolved_dependencies() {
        for reference in ["parser", "parser 9.0.0", "missing"] {
            let changed =
                LOCKFILE.replace("parser 1.0.0 (registry+https://example.com)", reference);
            assert!(matches!(
                dependency_fingerprint(&changed),
                Err(FingerprintError::Dependency(_))
            ));
        }
    }

    /// The checked-in lockfile resolves successfully with the production roots.
    #[test]
    fn accepts_workspace_lockfile() {
        assert!(dependency_fingerprint(include_str!("../../../../Cargo.lock")).is_ok());
    }
}
