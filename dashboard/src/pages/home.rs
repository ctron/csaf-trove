use leptos::prelude::*;

use crate::components::profile_badge::ProfileBadge;
use crate::models::ProviderSummary;

async fn fetch_providers() -> Result<Vec<ProviderSummary>, String> {
    let resp = gloo_net::http::Request::get("/api/providers")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[component]
pub fn HomePage() -> impl IntoView {
    let providers = LocalResource::new(fetch_providers);

    view! {
        <div>
            <h2>"CSAF Providers"</h2>
            <Suspense fallback=|| view! { <p class="text-muted text-center py-12">"Loading providers..."</p> }>
                {move || providers.get().map(|result| match result {
                    Ok(list) => view! { <ProviderTable providers=list /> }.into_any(),
                    Err(e) => view! { <p class="text-danger text-center py-12">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ProviderTable(providers: Vec<ProviderSummary>) -> impl IntoView {
    view! {
        <table>
            <thead>
                <tr>
                    <th>"Provider"</th>
                    <th>"Documents"</th>
                    <th>"Basic"</th>
                    <th>"Extended"</th>
                    <th>"Full"</th>
                    <th>"Last Validated"</th>
                </tr>
            </thead>
            <tbody>
                {providers.into_iter().map(|p| {
                    let domain = p.provider.clone();
                    let href = format!("/providers/{domain}");
                    let display_domain = domain.clone();
                    let validated_at = p.validated_at.clone();
                    view! {
                        <tr>
                            <td><a href={href}>{display_domain}</a></td>
                            <td>{p.document_count}</td>
                            <td><ProfileBadge profile=p.profiles.basic /></td>
                            <td><ProfileBadge profile=p.profiles.extended /></td>
                            <td><ProfileBadge profile=p.profiles.full /></td>
                            <td>{validated_at}</td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}
