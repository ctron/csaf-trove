use super::*;
use crate::{
    pipeline::store::TroveStoreVisitor,
    storage::git_repo::{commit_all, prepare_worktree, read_head_blob},
};
use csaf_walker::retrieve::RetrievedVisitor;
use sha2::{Digest, Sha256, Sha512};
use std::{fs, time::SystemTime};
use tempfile::tempdir;
use walker_common::retrieve::RetrievedDigest;

/// Creates a downloaded advisory with checksums over its original bytes.
fn advisory(data: Bytes) -> RetrievedAdvisory {
    RetrievedAdvisory {
        discovered: DiscoveredAdvisory {
            context: Arc::new(DistributionContext::Directory(
                Url::parse("https://example.com/advisories/").unwrap(),
            )),
            url: Url::parse("https://example.com/advisories/doc.json").unwrap(),
            modified: SystemTime::now(),
            digest: None,
            signature: None,
        },
        signature: Some("original detached signature\n".into()),
        sha256: Some(RetrievedDigest {
            expected: hex::encode(Sha256::digest(&data)),
            actual: Sha256::digest(&data),
        }),
        sha512: Some(RetrievedDigest {
            expected: hex::encode(Sha512::digest(&data)),
            actual: Sha512::digest(&data),
        }),
        data,
        metadata: RetrievalMetadata {
            last_modification: Some(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()),
            etag: None,
        },
    }
}

/// Exercises download, validation input, Git publication, and compressed full checkout.
#[tokio::test]
async fn compressed_advisories_preserve_bytes_paths_and_integrity() {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    let work = dir.path().join("work");
    let prepared = prepare_worktree(&repo, &work, false).unwrap();
    let data = Bytes::from(format!(
        "{{\n  \"description\": {:?}\n}}\n",
        "repeated text ".repeat(10_000)
    ));
    let expected_time = advisory(data.clone()).metadata.last_modification;
    let store = TroveStoreVisitor::new(&work);
    <TroveStoreVisitor as RetrievedVisitor<TroveFileSource>>::visit_advisory(
        &store,
        &(),
        Ok(advisory(data.clone())),
    )
    .await
    .unwrap();

    let relative = Path::new("example.com/advisories/doc.json");
    let logical = work.join(relative);
    let compressed = scratch::compressed_path(&logical);
    assert!(!logical.exists());
    assert!(fs::metadata(&compressed).unwrap().len() < data.len() as u64 / 10);
    assert_eq!(scratch::read(&compressed).unwrap(), data);
    assert!(logical.with_added_extension("sha256").exists());
    assert!(logical.with_added_extension("sha512").exists());
    assert!(logical.with_added_extension("asc").exists());
    assert!(!compressed.with_added_extension("asc").exists());

    // These files remain plain because metadata and key loading use their original paths.
    scratch::write(&work, Path::new("metadata/provider-metadata.json"), b"{}").unwrap();
    scratch::write(&work, Path::new("metadata/keys/0.key"), b"public key").unwrap();

    for checkout in [false, true] {
        if checkout {
            assert!(commit_all(&prepared, "initial").unwrap());
            assert_eq!(
                read_head_blob(&repo, relative.to_str().unwrap())
                    .unwrap()
                    .unwrap(),
                data
            );
            assert!(
                read_head_blob(&repo, "example.com/advisories/doc.json.zst")
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                read_head_blob(&repo, "example.com/advisories/doc.json.asc")
                    .unwrap()
                    .unwrap(),
                b"original detached signature\n"
            );
            let full = prepare_worktree(&repo, &work, false).unwrap();
            // A full download replaces an existing compressed checkout without staging zstd bytes.
            <TroveStoreVisitor as RetrievedVisitor<TroveFileSource>>::visit_advisory(
                &store,
                &(),
                Ok(advisory(data.clone())),
            )
            .await
            .unwrap();
            assert!(!logical.exists());
            assert!(compressed.exists());
            assert_eq!(
                fs::read(work.join("metadata/provider-metadata.json")).unwrap(),
                b"{}"
            );
            assert_eq!(
                fs::read(work.join("metadata/keys/0.key")).unwrap(),
                b"public key"
            );
            assert!(!commit_all(&full, "unchanged").unwrap());
        }
        let source = TroveFileSource::new(&work).unwrap();
        let context = DistributionContext::Directory(
            Url::from_directory_path(logical.parent().unwrap()).unwrap(),
        );
        let mut entries = source.load_index(context).await.unwrap();
        assert_eq!(entries.len(), 1);
        let discovered = entries.pop().unwrap();
        assert_eq!(discovered.url.to_file_path().unwrap(), logical);
        let retrieved = source.load_advisory(discovered).await.unwrap();
        assert_eq!(retrieved.data, data);
        assert_eq!(
            retrieved.signature.as_deref(),
            Some("original detached signature\n")
        );
        retrieved.sha256.unwrap().validate().unwrap();
        retrieved.sha512.unwrap().validate().unwrap();
        if !checkout {
            assert_eq!(retrieved.metadata.last_modification, expected_time);
        }
    }
}

/// Legacy scratch remains readable, while damaged zstd data fails validation and publication.
#[tokio::test]
async fn legacy_scratch_and_corrupt_compressed_advisories() {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    let work = dir.path().join("work");
    let prepared = prepare_worktree(&repo, &work, false).unwrap();
    let relative = Path::new("example.com/advisories/doc.json");
    let logical = work.join(relative);
    fs::create_dir_all(logical.parent().unwrap()).unwrap();
    fs::write(&logical, b"{\"legacy\":true}").unwrap();
    let source = TroveFileSource::new(&work).unwrap();
    let context = DistributionContext::Directory(
        Url::from_directory_path(logical.parent().unwrap()).unwrap(),
    );
    let discovered = source
        .load_index(context.clone())
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        source.load_advisory(discovered).await.unwrap().data,
        b"{\"legacy\":true}"[..]
    );
    commit_all(&prepared, "legacy").unwrap();

    let full = prepare_worktree(&repo, &work, false).unwrap();
    fs::write(scratch::compressed_path(&logical), b"broken zstd").unwrap();
    let discovered = source.load_index(context).await.unwrap().pop().unwrap();
    assert!(source.load_advisory(discovered).await.is_err());
    assert!(commit_all(&full, "corrupt").is_err());
    assert_eq!(
        read_head_blob(&repo, relative.to_str().unwrap())
            .unwrap()
            .unwrap(),
        b"{\"legacy\":true}"
    );
}
