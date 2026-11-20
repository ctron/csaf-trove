//! Measures distribution indexes while keeping advisory retrieval in the walker.

use crate::models::result::DistributionResourceSize;
use csaf_walker::{
    discover::{DiscoveredAdvisory, DistributionContext},
    model::metadata::ProviderMetadata,
    retrieve::RetrievedAdvisory,
    rolie::RolieFeed,
    source::{HttpSource, HttpSourceError, Source},
};
use csv::ReaderBuilder;
use http::{Response as HttpResponse, header::CONTENT_LENGTH};
use parking_lot::Mutex;
use reqwest::{Response, ResponseBuilderExt};
use std::{collections::BTreeMap, marker::PhantomData, sync::Arc};
use url::Url;
use walker_common::{
    changes::ChangeEntry,
    fetcher::{Data, DataProcessor, Fetcher, Json},
    source as walker_source,
    utils::url::ensure_slash,
};

/// Captures successful index sizes before membership and incremental filtering.
#[derive(Clone, Debug)]
pub(super) struct MeasuredIndexSource {
    /// Walker source retained for metadata and advisory retrieval.
    pub inner: HttpSource,
    /// Same fetcher used by the walker, including its retry policy.
    pub fetcher: Fetcher,
    /// Measurements keyed by the metadata's original distribution URL.
    pub sizes: Arc<Mutex<BTreeMap<String, DistributionResourceSize>>>,
}

impl walker_source::Source for MeasuredIndexSource {
    type Error = HttpSourceError;
    type Retrieved = RetrievedAdvisory;
}

impl Source for MeasuredIndexSource {
    /// Leaves metadata discovery to the walker.
    async fn load_metadata(&self) -> Result<ProviderMetadata, Self::Error> {
        self.inner.load_metadata().await
    }

    /// Loads the complete index and records sizes only after successful discovery.
    async fn load_index(
        &self,
        context: DistributionContext,
    ) -> Result<Vec<DiscoveredAdvisory>, Self::Error> {
        let context = Arc::new(context);
        // shortcut: mirror the walker's index mapping until it exposes resource size observations.
        let (advisories, size) = match context.as_ref() {
            DistributionContext::Directory(base) => {
                let base = ensure_slash(base.clone());
                let (text, size) = self
                    .fetcher
                    .fetch_processed(base.join("changes.csv")?, Measured::<String>(PhantomData))
                    .await?;
                let advisories = ReaderBuilder::new()
                    .has_headers(false)
                    .from_reader(text.as_bytes())
                    .into_deserialize::<ChangeEntry>()
                    .map(|entry| {
                        let entry = entry?;
                        Ok(DiscoveredAdvisory {
                            context: context.clone(),
                            url: base.join(&entry.file)?,
                            modified: entry.timestamp.into(),
                            digest: None,
                            signature: None,
                        })
                    })
                    .collect::<Result<Vec<_>, HttpSourceError>>()?;
                (advisories, size)
            }
            DistributionContext::Feed { url, .. } => {
                let (Json(feed), size) = self
                    .fetcher
                    .fetch_processed(url.clone(), Measured::<Json<RolieFeed>>(PhantomData))
                    .await?;
                let advisories = feed
                    .feed
                    .entry
                    .into_iter()
                    .map(|entry| {
                        let mut file = entry.content.src;
                        let mut signature = None;
                        let mut digest = None;
                        for link in entry.link {
                            match link.rel.as_str() {
                                "self" => file = link.href,
                                "signature" => signature = Some(link.href),
                                "hash" => digest = Some(link.href),
                                _ => {}
                            }
                        }
                        Ok(DiscoveredAdvisory {
                            context: context.clone(),
                            url: Url::parse(&file)?,
                            modified: entry.updated.into(),
                            digest: digest.map(|value| Url::parse(&value)).transpose()?,
                            signature: signature.map(|value| Url::parse(&value)).transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, HttpSourceError>>()?;
                (advisories, size)
            }
        };
        self.sizes.lock().insert(context.url().to_string(), size);
        Ok(advisories)
    }

    /// Leaves advisory retrieval and integrity metadata to the walker.
    async fn load_advisory(
        &self,
        advisory: DiscoveredAdvisory,
    ) -> Result<RetrievedAdvisory, Self::Error> {
        self.inner.load_advisory(advisory).await
    }
}

/// Measures bytes before delegating text/JSON conversion to the existing processor.
struct Measured<D>(PhantomData<fn() -> D>);

impl<D: Data> DataProcessor for Measured<D> {
    type Type = (D, DistributionResourceSize);

    /// Keeps decoding errors within the fetcher's existing retry handling.
    async fn process(&self, response: Response) -> Result<Self::Type, reqwest::Error> {
        let response = response.error_for_status()?;
        let headers = response.headers().clone();
        let url = response.url().clone();
        let download_size_bytes = headers
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let data = response.bytes().await?;
        let size = DistributionResourceSize {
            download_size_bytes,
            decoded_size_bytes: data.len() as u64,
        };
        let mut response = HttpResponse::new(data);
        if let Some(extensions) = HttpResponse::builder().url(url).extensions_ref() {
            *response.extensions_mut() = extensions.clone();
        }
        *response.headers_mut() = headers;
        Ok((D::from_response(response.into()).await?, size))
    }
}

#[cfg(test)]
mod tests;
