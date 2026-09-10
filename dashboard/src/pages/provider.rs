use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::components::profile_badge::ProfileBadge;
use crate::models::ProviderDetail;

async fn fetch_provider(domain: String) -> Result<ProviderDetail, String> {
    let resp = gloo_net::http::Request::get(&format!("/api/providers/{domain}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[component]
pub fn ProviderPage() -> impl IntoView {
    let params = use_params_map();
    let domain = move || params.read().get("domain").unwrap_or_default();

    let detail = LocalResource::new(move || {
        let d = domain();
        async move { fetch_provider(d).await }
    });

    view! {
        <div>
            <h2>{move || format!("Provider: {}", domain())}</h2>
            <Suspense fallback=|| view! { <p class="loading">"Loading..."</p> }>
                {move || detail.get().map(|result| match result {
                    Ok(d) => view! { <ProviderDetailView detail=d /> }.into_any(),
                    Err(e) => view! { <p class="error">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ProviderDetailView(detail: ProviderDetail) -> impl IntoView {
    let summary = detail.summary;
    let tests = summary.top_failing_tests;

    view! {
        <div class="cards">
            <div class="card">
                <h3>"Documents"</h3>
                <div class="value">{summary.document_count}</div>
            </div>
            <div class="card">
                <h3>"Basic"</h3>
                <div class="value"><ProfileBadge profile=summary.profiles.basic /></div>
            </div>
            <div class="card">
                <h3>"Extended"</h3>
                <div class="value"><ProfileBadge profile=summary.profiles.extended /></div>
            </div>
            <div class="card">
                <h3>"Full"</h3>
                <div class="value"><ProfileBadge profile=summary.profiles.full /></div>
            </div>
        </div>

        <h3>"Top Failing Tests"</h3>
        <table>
            <thead>
                <tr>
                    <th>"Test ID"</th>
                    <th>"Count"</th>
                    <th>"Severity"</th>
                </tr>
            </thead>
            <tbody>
                {tests.into_iter().map(|t| {
                    let test_id = t.test_id.clone();
                    let severity = t.severity.clone();
                    view! {
                        <tr>
                            <td>{test_id}</td>
                            <td>{t.count}</td>
                            <td>{severity}</td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}
