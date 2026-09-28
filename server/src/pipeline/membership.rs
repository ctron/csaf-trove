//! Captures complete distribution indexes before incremental download filtering.

use csaf_walker::{
    discover::{DiscoveredAdvisory, DistributionContext},
    model::metadata::ProviderMetadata,
    retrieve::RetrievedAdvisory,
    source::Source,
};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::SystemTime,
};

/// Complete indexes for distributions successfully discovered during a sync.
pub type DistributionMembership = BTreeMap<String, BTreeSet<String>>;

/// Records membership even for advisories that do not need downloading.
#[derive(Clone, Debug)]
pub struct MembershipSource<S> {
    /// Underlying source configured to return unfiltered indexes.
    pub inner: S,
    /// Earliest modification time requiring a download.
    pub since: Option<SystemTime>,
    /// Shared snapshots, including successful empty indexes.
    pub membership: Arc<Mutex<DistributionMembership>>,
}

impl<S: Source> walker_common::source::Source for MembershipSource<S> {
    type Error = S::Error;
    type Retrieved = RetrievedAdvisory;
}

impl<S: Source> Source for MembershipSource<S> {
    /// Delegates metadata discovery without modifying distribution URLs.
    async fn load_metadata(&self) -> Result<ProviderMetadata, Self::Error> {
        self.inner.load_metadata().await
    }

    /// Captures the full successful index, then selects changed documents for retrieval.
    async fn load_index(
        &self,
        context: DistributionContext,
    ) -> Result<Vec<DiscoveredAdvisory>, Self::Error> {
        let url = context.url().to_string();
        let mut advisories = self.inner.load_index(context).await?;
        self.membership
            .lock()
            .insert(url, advisories.iter().map(|a| a.url.to_string()).collect());
        if let Some(since) = self.since {
            advisories.retain(|a| a.modified >= since);
        }
        Ok(advisories)
    }

    /// Delegates advisory retrieval to the wrapped source.
    async fn load_advisory(
        &self,
        advisory: DiscoveredAdvisory,
    ) -> Result<RetrievedAdvisory, Self::Error> {
        self.inner.load_advisory(advisory).await
    }
}

#[cfg(test)]
mod tests;
