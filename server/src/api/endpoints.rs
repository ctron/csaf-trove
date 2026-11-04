use crate::AppState;
use actix_web::{HttpResponse, web};
use csaf_trove_common::endpoints::Endpoints;

/// Reports enabled public endpoints without exposing private configuration.
pub async fn get(state: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(Endpoints {
        mcp: state.config.mcp.enabled,
        aggregator: state.config.aggregator.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AggregatorCategory, AggregatorConfig};
    use actix_web::{App, test, test::TestRequest};
    use std::sync::Arc;

    /// Reports each combination of independently enabled endpoints.
    #[actix_web::test]
    async fn reports_enabled_endpoints() {
        for mcp in [false, true] {
            for aggregator in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let mut state = AppState::for_test(dir.path());
                let config = &mut Arc::get_mut(&mut state).unwrap().config;
                config.mcp.enabled = mcp;
                config.aggregator = aggregator.then(|| AggregatorConfig {
                    category: AggregatorCategory::Lister,
                    name: "Test".into(),
                    namespace: "https://example.com".into(),
                    canonical_url: "https://example.com/aggregator.json".into(),
                    contact_details: None,
                    issuing_authority: None,
                });
                let app = test::init_service(
                    App::new()
                        .app_data(web::Data::from(state))
                        .configure(super::super::config),
                )
                .await;
                let response: Endpoints = test::call_and_read_body_json(
                    &app,
                    TestRequest::get().uri("/endpoints").to_request(),
                )
                .await;
                assert_eq!(response, Endpoints { mcp, aggregator });
            }
        }
    }
}
