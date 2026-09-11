use std::cmp::Ordering;

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::models::{
    DocumentValidation, DocumentVersionInfo, HistoricalDocument, encode_path_segment,
};

/// Compares dotted-numeric test IDs (e.g. `6.1.27.5`) segment by segment.
fn numeric_test_id_cmp(a: &str, b: &str) -> Ordering {
    let mut a_parts = a.split('.');
    let mut b_parts = b.split('.');
    loop {
        match (a_parts.next(), b_parts.next()) {
            (Some(a_seg), Some(b_seg)) => {
                let ord = match (a_seg.parse::<u64>(), b_seg.parse::<u64>()) {
                    (Ok(an), Ok(bn)) => an.cmp(&bn),
                    _ => a_seg.cmp(b_seg),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
}

async fn fetch_document(domain: String, tracking_id: String) -> Result<DocumentValidation, String> {
    let resp = gloo_net::http::Request::get(&format!(
        "/api/providers/{}/document/{tracking_id}",
        encode_path_segment(&domain)
    ))
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Err("Document not found".to_string());
    }
    resp.json().await.map_err(|e| e.to_string())
}

async fn fetch_versions(
    domain: String,
    tracking_id: String,
) -> Result<Vec<DocumentVersionInfo>, String> {
    let resp = gloo_net::http::Request::get(&format!(
        "/api/providers/{}/document/{tracking_id}/versions",
        encode_path_segment(&domain)
    ))
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Ok(vec![]);
    }
    resp.json().await.map_err(|e| e.to_string())
}

async fn fetch_historical_document(
    domain: String,
    tracking_id: String,
    commit_id: String,
) -> Result<HistoricalDocument, String> {
    let resp = gloo_net::http::Request::get(&format!(
        "/api/providers/{}/document/{tracking_id}/versions/{commit_id}",
        encode_path_segment(&domain)
    ))
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Err("Version not found".to_string());
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Formats a Unix timestamp as a human-readable date string.
fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

#[component]
pub fn DocumentPage() -> impl IntoView {
    let params = use_params_map();
    let domain = move || params.read().get("domain").unwrap_or_default();
    let tracking_id = move || params.read().get("tracking_id").unwrap_or_default();

    let (selected_version, set_selected_version) = signal(Option::<String>::None);

    let detail = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        async move { fetch_document(d, t).await }
    });

    let versions = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        async move { fetch_versions(d, t).await }
    });

    let historical = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        let v = selected_version.get();
        async move {
            match v {
                Some(commit_id) => Some(fetch_historical_document(d, t, commit_id).await),
                None => None,
            }
        }
    });

    view! {
        <div>
            <p><a href={move || format!("/providers/{}", encode_path_segment(&domain()))}>"Back to provider"</a></p>

            <Suspense fallback=|| view! { <span /> }>
                {move || versions.get().map(|result| match result {
                    Ok(vs) if vs.len() > 1 => view! {
                        <VersionSelector
                            versions=vs
                            selected=selected_version
                            on_select=set_selected_version
                        />
                    }.into_any(),
                    _ => view! { <span /> }.into_any(),
                })}
            </Suspense>

            {move || {
                if selected_version.get().is_some() {
                    view! {
                        <Suspense fallback=|| view! { <p class="text-muted text-center py-12">"Loading version..."</p> }>
                            {move || historical.get().map(|outer| match outer {
                                Some(Ok(doc)) => view! { <HistoricalDocumentView doc=doc /> }.into_any(),
                                Some(Err(e)) => view! { <p class="text-danger text-center py-12">{e}</p> }.into_any(),
                                None => view! { <span /> }.into_any(),
                            })}
                        </Suspense>
                    }.into_any()
                } else {
                    view! {
                        <Suspense fallback=|| view! { <p class="text-muted text-center py-12">"Loading..."</p> }>
                            {move || detail.get().map(|result| match result {
                                Ok(doc) => view! { <DocumentDetailView doc=doc /> }.into_any(),
                                Err(e) => view! { <p class="text-danger text-center py-12">{e}</p> }.into_any(),
                            })}
                        </Suspense>
                    }.into_any()
                }
            }}
        </div>
    }
}

#[component]
fn VersionSelector(
    versions: Vec<DocumentVersionInfo>,
    selected: ReadSignal<Option<String>>,
    on_select: WriteSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="mb-4">
            <label class="text-sm text-muted mr-2">"Version: "</label>
            <select class="bg-surface text-foreground border border-border rounded-md px-3 py-2 text-sm cursor-pointer min-w-[300px]" on:change=move |ev| {
                use wasm_bindgen::JsCast;
                let target = ev.target().unwrap();
                let val = target.unchecked_ref::<web_sys::HtmlSelectElement>().value();
                if val == "latest" {
                    on_select.set(None);
                } else {
                    on_select.set(Some(val));
                }
            }>
                {versions.into_iter().map(|v| {
                    let label = if v.is_latest {
                        format!("{} (current)", format_timestamp(v.timestamp))
                    } else {
                        format_timestamp(v.timestamp)
                    };
                    let value = if v.is_latest {
                        "latest".to_string()
                    } else {
                        v.commit_id.clone()
                    };
                    let value_for_closure = value.clone();
                    let is_selected = move || {
                        match selected.get() {
                            None => value_for_closure == "latest",
                            Some(ref id) => *id == value_for_closure,
                        }
                    };
                    view! {
                        <option value={value} selected=is_selected>
                            {label}
                        </option>
                    }
                }).collect::<Vec<_>>()}
            </select>
        </div>
    }
}

#[component]
fn HistoricalDocumentView(doc: HistoricalDocument) -> impl IntoView {
    view! {
        <h2>{doc.tracking_id.clone()}</h2>

        <p class="bg-warning-subtle text-warning rounded-md px-4 py-2 text-sm mb-4">
            "Showing version from " {format_timestamp(doc.timestamp)}
            ". Validation results are only available for the current version."
        </p>

        <div class="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-4 mb-6">
            <div class="bg-surface border border-border rounded-lg p-4">
                <h3 class="text-sm text-muted mb-2">"Title"</h3>
                <div class="text-3xl font-semibold">{doc.title.clone()}</div>
            </div>
            {doc.category.clone().map(|c| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Category"</h3>
                    <div class="text-3xl font-semibold">{c}</div>
                </div>
            })}
            {doc.publisher_name.clone().map(|p| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Publisher"</h3>
                    <div class="text-3xl font-semibold">{p}</div>
                </div>
            })}
            {doc.aggregate_severity.clone().map(|s| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Severity"</h3>
                    <div class="text-3xl font-semibold">{s}</div>
                </div>
            })}
            {doc.status.clone().map(|s| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Status"</h3>
                    <div class="text-3xl font-semibold">{s}</div>
                </div>
            })}
            {doc.revision.clone().map(|r| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Revision"</h3>
                    <div class="text-3xl font-semibold">{r}</div>
                </div>
            })}
            {doc.initial_release_date.clone().map(|d| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Initial Release"</h3>
                    <div class="text-3xl font-semibold">{d}</div>
                </div>
            })}
            {doc.current_release_date.clone().map(|d| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Current Release"</h3>
                    <div class="text-3xl font-semibold">{d}</div>
                </div>
            })}
            {doc.csaf_version.clone().map(|v| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"CSAF Version"</h3>
                    <div class="text-3xl font-semibold">{v}</div>
                </div>
            })}
        </div>
    }
}

#[component]
fn DocumentDetailView(doc: DocumentValidation) -> impl IntoView {
    let sig_class = if doc.signature_error.is_some() {
        "badge badge-danger"
    } else if doc.signature_present {
        "badge badge-success"
    } else {
        "badge badge-warning"
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

        <div class="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-4 mb-6">
            <div class="bg-surface border border-border rounded-lg p-4">
                <h3 class="text-sm text-muted mb-2">"Title"</h3>
                <div class="text-3xl font-semibold">{title}</div>
            </div>
            {category.map(|c| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Category"</h3>
                    <div class="text-3xl font-semibold">{c}</div>
                </div>
            })}
            {publisher_name.map(|p| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Publisher"</h3>
                    <div class="text-3xl font-semibold">{p}</div>
                </div>
            })}
            {aggregate_severity.map(|s| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Severity"</h3>
                    <div class="text-3xl font-semibold">{s}</div>
                </div>
            })}
            {status.map(|s| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Status"</h3>
                    <div class="text-3xl font-semibold">{s}</div>
                </div>
            })}
            {revision.map(|r| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Revision"</h3>
                    <div class="text-3xl font-semibold">{r}</div>
                </div>
            })}
            {initial_release_date.map(|d| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Initial Release"</h3>
                    <div class="text-3xl font-semibold">{d}</div>
                </div>
            })}
            {current_release_date.map(|d| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"Current Release"</h3>
                    <div class="text-3xl font-semibold">{d}</div>
                </div>
            })}
            {csaf_version.map(|v| view! {
                <div class="bg-surface border border-border rounded-lg p-4">
                    <h3 class="text-sm text-muted mb-2">"CSAF Version"</h3>
                    <div class="text-3xl font-semibold">{v}</div>
                </div>
            })}
            <div class="bg-surface border border-border rounded-lg p-4">
                <h3 class="text-sm text-muted mb-2">"Signature"</h3>
                <div class="text-3xl font-semibold">
                    <span class={sig_class}>{sig_label}</span>
                    {sig_error.map(|e| view! {
                        <p class="text-sm text-danger mt-1">{e}</p>
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
            let (badge_class, badge_label) = if d.passed {
                ("badge badge-success", "Pass".to_string())
            } else if d.error_count > 0 {
                ("badge badge-danger", format!("{} errors", d.error_count))
            } else if d.warning_count > 0 {
                (
                    "badge badge-warning",
                    format!("{} warnings", d.warning_count),
                )
            } else {
                ("badge badge-info", format!("{} info", d.info_count))
            };

            view! {
                <h3>{title}" "<span class={badge_class}>{badge_label}</span></h3>
                {if d.failing_tests.is_empty() {
                    view! { <div /> }.into_any()
                } else {
                    let mut tests = d.failing_tests;
                    tests.sort_by(|a, b| numeric_test_id_cmp(&a.test_id, &b.test_id));
                    view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"Severity"</th>
                                    <th>"Test ID"</th>
                                    <th>"Message"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {tests.into_iter().map(|f| {
                                    let sev_class = match f.severity.as_str() {
                                        "error" => "badge badge-danger",
                                        "warning" => "badge badge-warning",
                                        "info" => "badge badge-info",
                                        _ => "badge",
                                    };
                                    let sev_label = f.severity.clone();
                                    view! {
                                        <tr>
                                            <td><span class={sev_class}>{sev_label}</span></td>
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
