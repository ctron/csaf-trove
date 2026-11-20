//! End-to-end tests of the MCP endpoint over HTTP.

use super::{McpConfig, service};
use crate::{
    AppState,
    storage::cache::test::{document, provider},
};
use actix_web::{
    App,
    dev::{Service, ServiceResponse},
    http::StatusCode,
    test, web,
};
use serde_json::{Value, json};

/// Builds a JSON-RPC request to the MCP endpoint.
fn rpc(host: &str, body: Value) -> test::TestRequest {
    test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("host", host))
        .insert_header(("accept", "application/json, text/event-stream"))
        .insert_header(("mcp-protocol-version", "2025-06-18"))
        .set_json(body)
}

/// Sends a request and returns the status and parsed JSON body.
async fn send<S, R>(app: &S, req: R) -> (StatusCode, Value)
where
    S: Service<R, Response = ServiceResponse, Error = actix_web::Error>,
{
    let resp = test::call_service(app, req).await;
    let status = resp.status();
    let bytes = test::read_body(resp).await;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
    (status, body)
}

#[actix_web::test]
async fn serves_read_only_tools() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_test(dir.path());
    provider(
        &state.storage,
        "a.example",
        &[document("A-1", "2024-01-01T00:00:00Z", &["CVE-2024-3094"])],
    )
    .await;

    let mcp = service(state.clone(), &McpConfig::default()).unwrap();
    let app = test::init_service(App::new().service(web::scope("/mcp").service(mcp.scope()))).await;

    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}
            }}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["serverInfo"]["name"], "csaf-trove");

    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tools = body["result"]["tools"].as_array().unwrap();
    assert!(
        tools
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true),
        "{body}"
    );
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"find_advisories_by_cve"), "{names:?}");

    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "find_advisories_by_cve", "arguments": {"cve": "cve-2024-3094"}
            }}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = body["result"]["content"][0]["text"].as_str().unwrap();
    let result: Value = serde_json::from_str(text).unwrap();
    assert_eq!(result["hits"][0]["domain"], "a.example");
    assert_eq!(result["hits"][0]["tracking_id"], "A-1");

    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
                "name": "get_provider", "arguments": {"domain": "unknown.example"}
            }}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["isError"], true, "{body}");
}

#[actix_web::test]
async fn rejects_unlisted_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_test(dir.path());
    let config = McpConfig {
        enabled: true,
        allowed_hosts: Some(vec!["csaf.example.com".into()]),
    };
    let mcp = service(state, &config).unwrap();
    let app = test::init_service(App::new().service(web::scope("/mcp").service(mcp.scope()))).await;

    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "test", "version": "1"}
    }});
    let (status, _) = send(&app, rpc("evil.example", init.clone()).to_request()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = send(&app, rpc("csaf.example.com", init).to_request()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[actix_web::test]
async fn rejects_empty_allowed_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_test(dir.path());
    let config = McpConfig {
        enabled: true,
        allowed_hosts: Some(vec![]),
    };
    assert!(service(state, &config).is_err());
}

/// Providers that disallow mirroring get an error instead of raw JSON or diffs.
#[actix_web::test]
async fn raw_json_and_diffs_blocked_when_mirroring_disallowed() {
    use crate::storage::ProviderInfo;

    let info = |mirror_on_aggregators| ProviderInfo {
        canonical_url: "https://a.example/provider-metadata.json".into(),
        publisher_name: "A".into(),
        publisher_category: "vendor".into(),
        publisher_namespace: "https://a.example".into(),
        role: Some("csaf_provider".into()),
        list_on_aggregators: true,
        mirror_on_aggregators,
        last_updated: "2024-01-01T00:00:00Z".into(),
    };

    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_test(dir.path());
    provider(
        &state.storage,
        "a.example",
        &[document("A-1", "2024-01-01T00:00:00Z", &["CVE-2024-3094"])],
    )
    .await;
    state
        .storage
        .save_provider_info("a.example", &info(false))
        .await
        .unwrap();

    let mcp = service(state.clone(), &McpConfig::default()).unwrap();
    let app = test::init_service(App::new().service(web::scope("/mcp").service(mcp.scope()))).await;

    // The raw advisory resource refuses to serve the full document.
    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": {"uri": "csaf://a.example/A-1"}}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("mirror_on_CSAF_aggregators"), "{body}");

    // The diff tool refuses as well, before touching the git repo.
    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
                "name": "diff_document_version",
                "arguments": {"domain": "a.example", "tracking_id": "A-1", "commit_id": "abc123"}
            }}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["isError"], true, "{body}");
    let text = body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("mirror_on_CSAF_aggregators"), "{text}");

    // With mirroring allowed, the resource passes the guard and only fails because no
    // git blob exists in this fixture.
    state
        .storage
        .save_provider_info("a.example", &info(true))
        .await
        .unwrap();
    let (status, body) = send(
        &app,
        rpc(
            "localhost",
            json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "csaf://a.example/A-1"}}),
        )
        .to_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("No advisory"), "{body}");
}
