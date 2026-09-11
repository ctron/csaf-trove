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
    let sig_error = doc.signature_error.clone();
    let category = doc.category.clone();
    let publisher_name = doc.publisher_name.clone();
    let initial_release_date = doc.initial_release_date.clone();
    let current_release_date = doc.current_release_date.clone();
    let status = doc.status.clone();
    let revision = doc.revision.clone();
    let aggregate_severity = doc.aggregate_severity.clone();
    let csaf_version = doc.csaf_version.clone();

    view! {
        <h2>{tracking_id}</h2>

        <div class="cards">
            <div class="card">
                <h3>"Title"</h3>
                <div class="value">{title}</div>
            </div>
            {category.map(|c| view! {
                <div class="card">
                    <h3>"Category"</h3>
                    <div class="value">{c}</div>
                </div>
            })}
            {publisher_name.map(|p| view! {
                <div class="card">
                    <h3>"Publisher"</h3>
                    <div class="value">{p}</div>
                </div>
            })}
            {aggregate_severity.map(|s| view! {
                <div class="card">
                    <h3>"Severity"</h3>
                    <div class="value">{s}</div>
                </div>
            })}
            {status.map(|s| view! {
                <div class="card">
                    <h3>"Status"</h3>
                    <div class="value">{s}</div>
                </div>
            })}
            {revision.map(|r| view! {
                <div class="card">
                    <h3>"Revision"</h3>
                    <div class="value">{r}</div>
                </div>
            })}
            {initial_release_date.map(|d| view! {
                <div class="card">
                    <h3>"Initial Release"</h3>
                    <div class="value">{d}</div>
                </div>
            })}
            {current_release_date.map(|d| view! {
                <div class="card">
                    <h3>"Current Release"</h3>
                    <div class="value">{d}</div>
                </div>
            })}
            {csaf_version.map(|v| view! {
                <div class="card">
                    <h3>"CSAF Version"</h3>
                    <div class="value">{v}</div>
                </div>
            })}
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
