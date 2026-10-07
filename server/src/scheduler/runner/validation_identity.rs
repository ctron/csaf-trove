//! Fingerprints the locked dependencies that implement document validation.

use serde::{Deserialize, Serialize};
use serde_json::{Error as JsonError, to_vec};
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml::{de::Error as TomlError, from_str};

/// Crates whose exact versions determine validation results.
///
/// Their transitive dependencies are intentionally excluded: routine lockfile refreshes would
/// otherwise force a full revalidation of every provider on each release.
const VALIDATION_CRATES: [&str; 4] = ["csaf-rs", "csaf-walker", "walker-common", "sequoia-openpgp"];

/// Invalid embedded lockfile or dependency reference.
#[derive(Debug, Error)]
pub(super) enum FingerprintError {
    /// The lockfile could not be decoded.
    #[error("Invalid validation lockfile: {0}")]
    Lockfile(#[from] TomlError),
    /// A validation crate must be locked exactly once.
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

/// Locked identity of one package.
#[derive(Deserialize, Serialize)]
struct Package {
    /// Cargo package name.
    name: String,
    /// Exact locked version.
    version: String,
    /// Registry or Git source; absent for workspace packages.
    source: Option<String>,
    /// Registry content checksum, when present.
    checksum: Option<String>,
}

/// Finds the single locked package with the given name.
fn resolve<'a>(packages: &'a [Package], name: &str) -> Result<&'a Package, FingerprintError> {
    let mut matches = packages.iter().filter(|package| package.name == name);
    match (matches.next(), matches.next()) {
        (Some(package), None) => Ok(package),
        _ => Err(FingerprintError::Dependency(name.to_owned())),
    }
}

/// Hashes the exact locked versions of the validation crates, independently of lockfile order.
pub(super) fn dependency_fingerprint(lockfile: &str) -> Result<String, FingerprintError> {
    let lockfile: Lockfile = from_str(lockfile)?;
    let selected = VALIDATION_CRATES
        .into_iter()
        .map(|name| resolve(&lockfile.package, name))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(hex::encode(Sha256::digest(to_vec(&selected)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small lockfile with validation crates, a transitive dependency and a workspace package.
    const LOCKFILE: &str = r#"
version = 4
[[package]]
name = "csaf-rs"
version = "0.5.0"
dependencies = ["parser 1.0.0 (registry+https://example.com)", "walker-common"]
[[package]]
name = "csaf-walker"
version = "0.19.0"
dependencies = ["csaf-rs", "walker-common", "sequoia-openpgp"]
[[package]]
name = "walker-common"
version = "0.19.0"
[[package]]
name = "sequoia-openpgp"
version = "2.0.0"
source = "registry+https://example.com"
checksum = "signing"
[[package]]
name = "parser"
version = "1.0.0"
source = "registry+https://example.com"
checksum = "original"
[[package]]
name = "csaf-trove-server"
version = "0.1.0"
dependencies = ["csaf-walker", "parser 1.0.0"]
"#;

    /// Only the validation crates' own identities affect the fingerprint.
    #[test]
    fn fingerprints_only_validation_dependencies() {
        let fingerprint = dependency_fingerprint(LOCKFILE).unwrap();
        for changed in [
            LOCKFILE.replace("0.1.0", "0.2.0"),
            LOCKFILE.replace("1.0.0", "1.1.0"),
            LOCKFILE.replace("original", "updated"),
            LOCKFILE.replace(
                "[\"csaf-rs\", \"walker-common\", \"sequoia-openpgp\"]",
                "[\"walker-common\", \"csaf-rs\", \"sequoia-openpgp\"]",
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
            LOCKFILE.replace("0.19.0", "0.20.0"),
            LOCKFILE.replace("2.0.0", "2.1.0"),
            LOCKFILE.replace("signing", "updated"),
            LOCKFILE.replace("registry+https://example.com", "registry+https://other.com"),
        ] {
            assert_ne!(fingerprint, dependency_fingerprint(&changed).unwrap());
        }
    }

    /// Missing and ambiguous validation crates must fail instead of being silently omitted.
    #[test]
    fn rejects_unresolved_dependencies() {
        for changed in [
            LOCKFILE.replace("name = \"sequoia-openpgp\"", "name = \"other\""),
            LOCKFILE.replace("name = \"parser\"", "name = \"walker-common\""),
        ] {
            assert!(matches!(
                dependency_fingerprint(&changed),
                Err(FingerprintError::Dependency(_))
            ));
        }
    }

    /// The checked-in lockfile resolves successfully with the production crates.
    #[test]
    fn accepts_workspace_lockfile() {
        assert!(dependency_fingerprint(include_str!("../../../../Cargo.lock")).is_ok());
    }
}
