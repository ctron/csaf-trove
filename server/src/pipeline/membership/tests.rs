//! Regression coverage for discovery before incremental download filtering.

use super::*;
use anyhow::{Result, anyhow, bail};
use csaf_walker::model::metadata::TlpLabel;
use std::time::Duration;
use url::Url;

/// Test source exposing complete indexes and failing missing distributions.
#[derive(Clone, Debug, Default)]
struct IndexSource {
    /// Index contents keyed by their exact discovery URLs.
    indexes: BTreeMap<String, Vec<DiscoveredAdvisory>>,
}

impl walker_common::source::Source for IndexSource {
    type Error = anyhow::Error;
    type Retrieved = RetrievedAdvisory;
}

impl Source for IndexSource {
    /// Metadata is not needed for the index-level regression.
    async fn load_metadata(&self) -> Result<ProviderMetadata> {
        bail!("No test metadata")
    }

    /// Returns the complete configured index, including duplicate URLs.
    async fn load_index(&self, context: DistributionContext) -> Result<Vec<DiscoveredAdvisory>> {
        self.indexes
            .get(context.url().as_str())
            .cloned()
            .ok_or_else(|| anyhow!("Unavailable distribution"))
    }

    /// Fails if index discovery accidentally attempts document retrieval.
    async fn load_advisory(&self, _advisory: DiscoveredAdvisory) -> Result<RetrievedAdvisory> {
        bail!("Index discovery must not retrieve documents")
    }
}

/// Builds a discovered URL that need not share the distribution's host or path.
fn advisory(context: &DistributionContext, url: &str, seconds: u64) -> DiscoveredAdvisory {
    DiscoveredAdvisory {
        context: Arc::new(context.clone()),
        url: Url::parse(url).unwrap(),
        modified: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
        digest: None,
        signature: None,
    }
}

/// Feeds in the same directory remain distinct and include unchanged, external advisories.
#[tokio::test]
async fn membership_precedes_incremental_filtering() {
    let first = DistributionContext::Feed {
        url: Url::parse("https://feeds.example/.well-known/csaf/white.json").unwrap(),
        tlp_label: TlpLabel::Clear,
    };
    let second = DistributionContext::Feed {
        url: Url::parse("https://feeds.example/.well-known/csaf/green.json").unwrap(),
        tlp_label: TlpLabel::Green,
    };
    let directory =
        DistributionContext::Directory(Url::parse("https://feeds.example/advisories/").unwrap());
    let old = advisory(&first, "https://cdn.example/csaf/white/2026/old.json", 10);
    let new = advisory(&first, "https://cdn.example/csaf/white/2026/new.json", 20);
    let shared = advisory(&second, old.url.as_str(), 10);
    let mut source = MembershipSource {
        inner: IndexSource {
            indexes: BTreeMap::from([
                (
                    first.url().to_string(),
                    vec![old.clone(), old.clone(), new.clone()],
                ),
                (second.url().to_string(), vec![shared.clone()]),
                (directory.url().to_string(), vec![shared]),
            ]),
        },
        since: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(20)),
        membership: Arc::default(),
    };
    assert_eq!(
        source.load_index(first.clone()).await.unwrap(),
        vec![new.clone()]
    );
    assert!(source.load_index(second.clone()).await.unwrap().is_empty());
    assert!(
        source
            .load_index(directory.clone())
            .await
            .unwrap()
            .is_empty()
    );
    let recorded = source.membership.lock().clone();
    assert_eq!(
        recorded[first.url().as_str()],
        BTreeSet::from([old.url.to_string(), new.url.to_string()])
    );
    assert_eq!(
        recorded[second.url().as_str()],
        BTreeSet::from([old.url.to_string()])
    );
    assert_eq!(
        recorded[directory.url().as_str()],
        recorded[second.url().as_str()]
    );

    // A later successful empty index clears membership even with no downloaded documents.
    source.inner.indexes.insert(first.url().to_string(), vec![]);
    assert!(source.load_index(first.clone()).await.unwrap().is_empty());
    assert!(source.membership.lock()[first.url().as_str()].is_empty());

    // Failed discovery must not publish an empty replacement for the previous index.
    source.inner.indexes.remove(second.url().as_str());
    assert!(source.load_index(second.clone()).await.is_err());
    assert_eq!(
        source.membership.lock()[second.url().as_str()],
        recorded[second.url().as_str()]
    );
}
