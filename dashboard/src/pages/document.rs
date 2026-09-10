use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::models::DocumentValidation;

async fn fetch_document(domain: String, tracking_id: String) -> Result<DocumentValidation, String> {
    let resp =
        gloo_net::http::Request::get(&format!("/api/providers/{domain}/document/{tracking_id}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Err("Document not found".to_string());
    }
    resp.json().await.map_err(|e| e.to_string())
}

#[component]
pub fn DocumentPage() -> impl IntoView {
    let params = use_params_map();
    let domain = move || params.read().get("domain").unwrap_or_default();
    let tracking_id = move || params.read().get("tracking_id").unwrap_or_default();

    let detail = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        async move { fetch_document(d, t).await }
    });

    view! {
        <div>
            <p><a href={move || format!("/providers/{}", domain())}>"Back to provider"</a></p>
            <Suspense fallback=|| view! { <p class="loading">"Loading..."</p> }>
                {move || detail.get().map(|result| match result {
                    Ok(doc) => view! { <DocumentDetailView doc=doc /> }.into_any(),
                    Err(e) => view! { <p class="error">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn DocumentDetailView(doc: DocumentValidation) -> impl IntoView {
    let sig_class = if doc.signature_error.is_some() {
        "badge badge-red"
    } else if doc.signature_present {
        "badge badge-green"
    } else {
        "badge badge-yellow"
    };
    let sig_label = if doc.signature_error.is_some() {
        "Invalid"
    } else if doc.signature_present {
        "Valid"
    } else {
        "Missing"
    };

    let tracking_id = doc.tracking_id.clone();
    let title = doc.title.clone();
    let url = doc.url.clone();
    let url2 = doc.url.clone();
    let sig_error = doc.signature_error.clone();

    view! {
        <h2>{tracking_id}</h2>

        <div class="cards">
            <div class="card">
                <h3>"Title"</h3>
                <div class="value">{title}</div>
            </div>
            <div class="card">
                <h3>"URL"</h3>
                <div class="value"><a href={url} target="_blank">{url2}</a></div>
            </div>
            <div class="card">
                <h3>"Signature"</h3>
                <div class="value">
                    <span class={sig_class}>{sig_label}</span>
                    {sig_error.map(|e| view! {
                        <p class="error-detail">{e}</p>
                    })}
                </div>
            </div>
        </div>

        <ProfileSection title="Basic" detail=doc.profiles.basic />
        <ProfileSection title="Extended" detail=doc.profiles.extended />
        <ProfileSection title="Full" detail=doc.profiles.full />
    }
}

#[component]
fn ProfileSection(
    title: &'static str,
    detail: Option<crate::models::DocumentProfileDetail>,
) -> impl IntoView {
    match detail {
        None => view! { <div /> }.into_any(),
        Some(d) => {
            let badge_class = if d.passed {
                "badge badge-green"
            } else {
                "badge badge-red"
            };
            let badge_label = if d.passed {
                "Pass".to_string()
            } else {
                format!("{} errors", d.error_count)
            };

            view! {
                <h3>{title}" "<span class={badge_class}>{badge_label}</span></h3>
                {if d.failing_tests.is_empty() {
                    view! { <div /> }.into_any()
                } else {
                    view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"Test ID"</th>
                                    <th>"Message"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {d.failing_tests.into_iter().map(|f| {
                                    view! {
                                        <tr>
                                            <td>{f.test_id}</td>
                                            <td>{f.message}</td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>
                    }.into_any()
                }}
            }
            .into_any()
        }
    }
}
