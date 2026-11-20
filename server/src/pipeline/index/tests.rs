//! Index measurements use the same request, URL mapping, and parsing as discovery.

use super::*;
use actix_web::{App, HttpResponse as WebResponse, HttpServer, dev::ServerHandle, web};
use csaf_walker::{metadata::MetadataRetriever, model::metadata::TlpLabel, source::HttpOptions};
use std::{
    net::TcpListener,
    sync::atomic::{AtomicUsize, Ordering},
    time::SystemTime,
};

/// Complete ROLIE index with explicit advisory, hash, and signature links.
const FEED: &str = r#"{"feed":{"id":"feed","title":"Test","updated":"2026-01-01T00:00:00Z","entry":[{"id":"advisory","title":"Test","published":"2026-01-01T00:00:00Z","updated":"2026-01-02T00:00:00Z","format":{"schema":"csaf","version":"2.0"},"content":{"src":"https://example.com/fallback.json","type":"application/json"},"link":[{"rel":"self","href":"https://cdn.example/advisory.json"},{"rel":"hash","href":"https://cdn.example/advisory.json.sha256"},{"rel":"signature","href":"https://cdn.example/advisory.json.asc"}] }]}}"#;

/// CSV index pointing outside its directory and containing unchanged advisories.
const CSV: &str = "https://cdn.example/advisory.json,2026-01-01T00:00:00Z\n";

/// Serves an index and a redirect while counting every requested resource.
async fn server(body: &'static str) -> (String, ServerHandle, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let server = HttpServer::new(move || {
        let counter = counter.clone();
        App::new().default_service(web::to(move |request: actix_web::HttpRequest| {
            counter.fetch_add(1, Ordering::Relaxed);
            async move {
                match request.path() {
                    "/gzip/changes.csv" => WebResponse::Ok()
                        .insert_header(("Content-Encoding", "gzip"))
                        // A gzip-encoded newline: one decoded byte and an empty CSV index.
                        .body(&b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff\xe3\x02\x00\x93\x06\xd7\x32\x01\x00\x00\x00"[..]),
                    "/redirect" => WebResponse::Found()
                        .insert_header(("Location", "/feed"))
                        .finish(),
                    "/missing" => WebResponse::Forbidden().finish(),
                    _ => WebResponse::Ok().body(body),
                }
            }
        }))
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    tokio::spawn(server);
    (url, handle, requests)
}

/// Builds a measured source without downloading provider metadata.
fn source() -> MeasuredIndexSource {
    let fetcher = Fetcher::from(reqwest::Client::new());
    MeasuredIndexSource {
        inner: HttpSource::new(
            MetadataRetriever::new("example.com"),
            fetcher.clone(),
            HttpOptions::default(),
        ),
        fetcher,
        sizes: Arc::default(),
    }
}

/// Captures complete CSV bytes even when incremental filtering downloads no advisories.
#[tokio::test]
async fn csv_sizes_precede_incremental_filtering() {
    let (url, handle, requests) = server(CSV).await;
    let source = source();
    let sizes = source.sizes.clone();
    let membership = Arc::default();
    let source = crate::pipeline::membership::MembershipSource {
        inner: source,
        since: Some(SystemTime::now()),
        retry: Arc::default(),
        membership,
    };
    let context = DistributionContext::Directory(Url::parse(&format!("{url}/advisories")).unwrap());
    assert!(source.load_index(context.clone()).await.unwrap().is_empty());
    assert_eq!(
        sizes.lock()[context.url().as_str()],
        DistributionResourceSize {
            download_size_bytes: Some(CSV.len() as u64),
            decoded_size_bytes: CSV.len() as u64,
        }
    );
    assert!(
        source.membership.lock()[context.url().as_str()]
            .contains("https://cdn.example/advisory.json")
    );
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    handle.stop(true).await;
}

/// ROLIE redirects preserve original distribution keys and integrity URLs.
#[tokio::test]
async fn rolie_redirect_and_failed_index() {
    let (url, handle, requests) = server(FEED).await;
    let source = source();
    let context = DistributionContext::Feed {
        url: Url::parse(&format!("{url}/redirect")).unwrap(),
        tlp_label: TlpLabel::Clear,
    };
    let advisories = source.load_index(context.clone()).await.unwrap();
    assert_eq!(advisories.len(), 1);
    assert_eq!(
        advisories[0].url.as_str(),
        "https://cdn.example/advisory.json"
    );
    assert_eq!(
        advisories[0].digest.as_ref().unwrap().as_str(),
        "https://cdn.example/advisory.json.sha256"
    );
    assert_eq!(
        advisories[0].signature.as_ref().unwrap().as_str(),
        "https://cdn.example/advisory.json.asc"
    );
    assert_eq!(
        source.sizes.lock()[context.url().as_str()].decoded_size_bytes,
        FEED.len() as u64
    );
    assert_eq!(requests.load(Ordering::Relaxed), 2);
    let missing = DistributionContext::Feed {
        url: Url::parse(&format!("{url}/missing")).unwrap(),
        tlp_label: TlpLabel::Green,
    };
    assert!(source.load_index(missing.clone()).await.is_err());
    assert!(!source.sizes.lock().contains_key(missing.url().as_str()));
    handle.stop(true).await;
}

/// Missing/invalid Content-Length is unknown; decoded sizes count bytes, not characters.
#[tokio::test]
async fn measures_bytes_before_text_conversion() {
    for header in [None, Some("invalid"), Some("0")] {
        let mut response = HttpResponse::new("é");
        if let Some(header) = header {
            response
                .headers_mut()
                .insert(CONTENT_LENGTH, header.parse().unwrap());
        }
        let (text, size) = Measured::<String>(PhantomData)
            .process(response.into())
            .await
            .unwrap();
        assert_eq!(text, "é");
        assert_eq!(size.decoded_size_bytes, 2);
        assert_eq!(
            size.download_size_bytes,
            header.and_then(|value| value.parse().ok())
        );
    }
    let (url, handle, _) = server("").await;
    let source = source();
    let context = DistributionContext::Directory(Url::parse(&url).unwrap());
    assert!(source.load_index(context.clone()).await.unwrap().is_empty());
    assert_eq!(
        source.sizes.lock()[context.url().as_str()].decoded_size_bytes,
        0
    );
    handle.stop(true).await;
}

/// Automatic gzip decoding hides Content-Length and measures the decoded body.
#[tokio::test]
async fn compressed_index_has_unknown_download_size() {
    let (url, handle, requests) = server("").await;
    let source = source();
    let context = DistributionContext::Directory(Url::parse(&format!("{url}/gzip/")).unwrap());
    assert!(source.load_index(context.clone()).await.unwrap().is_empty());
    assert_eq!(
        source.sizes.lock()[context.url().as_str()],
        DistributionResourceSize {
            download_size_bytes: None,
            decoded_size_bytes: 1,
        }
    );
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    handle.stop(true).await;
}
