use std::cmp::Ordering;

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::models::{
    DocumentValidation, DocumentVersionInfo, HistoricalDocument, RevisionEntry, encode_path_segment,
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

        <h3>"Document"</h3>
        <table class="mb-6">
            <tbody>
                <MetadataRow label="Title" value=Some(doc.title.clone()) />
                <MetadataRow label="Category" value=doc.category.clone() />
                <MetadataRow label="Publisher" value=doc.publisher_name.clone() />
                <MetadataRow label="Severity" value=doc.aggregate_severity.clone() />
                <MetadataRow label="CSAF Version" value=doc.csaf_version.clone() />
            </tbody>
        </table>

        <h3>"Tracking"</h3>
        <table class="mb-6">
            <tbody>
                <MetadataRow label="Status" value=doc.status.clone() />
                <MetadataRow label="Version" value=doc.revision.clone() />
                <MetadataRow label="Initial Release" value=doc.initial_release_date.clone() />
                <MetadataRow label="Current Release" value=doc.current_release_date.clone() />
            </tbody>
        </table>

        <RevisionHistoryTable entries=doc.revision_history />
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

    let sig_error = doc.signature_error.clone();
    let url_href = doc.url.clone();
    let url_label = doc.url.clone();

    view! {
        <h2>{doc.tracking_id.clone()}</h2>

        <h3>"Document"</h3>
        <table class="mb-6">
            <tbody>
                <MetadataRow label="Title" value=Some(doc.title.clone()) />
                <MetadataRow label="Category" value=doc.category.clone() />
                <MetadataRow label="Publisher" value=doc.publisher_name.clone() />
                <MetadataRow label="Severity" value=doc.aggregate_severity.clone() />
                <MetadataRow label="CSAF Version" value=doc.csaf_version.clone() />
                <tr>
                    <td class="text-xs font-semibold uppercase text-muted w-48">"URL"</td>
                    <td><a href={url_href} target="_blank">{url_label}</a></td>
                </tr>
                <tr>
                    <td class="text-xs font-semibold uppercase text-muted w-48">"Signature"</td>
                    <td>
                        <span class={sig_class}>{sig_label}</span>
                        {sig_error.map(|e| view! {
                            <span class="text-sm text-danger ml-2">{e}</span>
                        })}
                    </td>
                </tr>
            </tbody>
        </table>

        <h3>"Tracking"</h3>
        <table class="mb-6">
            <tbody>
                <MetadataRow label="Status" value=doc.status.clone() />
                <MetadataRow label="Version" value=doc.revision.clone() />
                <MetadataRow label="Initial Release" value=doc.initial_release_date.clone() />
                <MetadataRow label="Current Release" value=doc.current_release_date.clone() />
            </tbody>
        </table>

        <RevisionHistoryTable entries=doc.revision_history />

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

#[component]
fn MetadataRow(label: &'static str, value: Option<String>) -> impl IntoView {
    value.map(|v| {
        view! {
            <tr>
                <td class="text-xs font-semibold uppercase text-muted w-48">{label}</td>
                <td>{v}</td>
            </tr>
        }
    })
}

#[component]
fn RevisionHistoryTable(entries: Vec<RevisionEntry>) -> impl IntoView {
    if entries.is_empty() {
        return view! { <div /> }.into_any();
    }
    view! {
        <h3>"Revision History"</h3>
        <table class="mb-6">
            <thead>
                <tr>
                    <th>"Version"</th>
                    <th>"Date"</th>
                    <th>"Summary"</th>
                </tr>
            </thead>
            <tbody>
                {entries.into_iter().map(|r| view! {
                    <tr>
                        <td>{r.number}</td>
                        <td>{r.date}</td>
                        <td>{r.summary}</td>
                    </tr>
                }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
    .into_any()
}
