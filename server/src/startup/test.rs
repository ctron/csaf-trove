//! Tests for the startup gate.

use super::{Startup, middleware, status};
use crate::AppState;
use actix_web::{
    App, HttpResponse,
    http::{StatusCode, header::RETRY_AFTER},
    middleware::from_fn,
    test::{self, TestRequest},
    web,
};
use csaf_trove_common::startup::StartupStatus;

#[test]
fn tracks_tasks_and_progress() {
    let startup = Startup::default();
    assert!(!startup.is_ready());

    startup.begin("first");
    startup.progress(1, 3);
    startup.begin("second");
    startup.progress(2, 5);
    assert_eq!(
        startup.snapshot(),
        StartupStatus {
            ready: false,
            task: Some("second".into()),
            current: 2,
            total: 5,
            completed: vec!["first".into()],
        }
    );

    startup.finish();
    assert_eq!(
        startup.snapshot(),
        StartupStatus {
            ready: true,
            task: None,
            current: 0,
            total: 0,
            completed: vec!["first".into(), "second".into()],
        }
    );
}

#[tokio::test]
async fn gates_data_endpoints_until_ready() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::for_test(dir.path());
    let state_mut = std::sync::Arc::get_mut(&mut state).unwrap();
    state_mut.startup = Startup::default();
    state_mut.startup.begin("Loading");

    let app = test::init_service(
        App::new()
            .wrap(from_fn(middleware))
            .app_data(web::Data::from(state.clone()))
            .route("/api/startup", web::get().to(status))
            .route("/api/providers", web::get().to(HttpResponse::Ok))
            .route("/mcp", web::post().to(HttpResponse::Ok))
            .route("/", web::get().to(HttpResponse::Ok)),
    )
    .await;

    let resp =
        test::call_service(&app, TestRequest::get().uri("/api/providers").to_request()).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(resp.headers().contains_key(RETRY_AFTER));

    let resp = test::call_service(&app, TestRequest::post().uri("/mcp").to_request()).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let resp = test::call_service(&app, TestRequest::get().uri("/").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp: StartupStatus =
        test::call_and_read_body_json(&app, TestRequest::get().uri("/api/startup").to_request())
            .await;
    assert!(!resp.ready);
    assert_eq!(resp.task.as_deref(), Some("Loading"));

    state.startup.finish();

    let resp =
        test::call_service(&app, TestRequest::get().uri("/api/providers").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
