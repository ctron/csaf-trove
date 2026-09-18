use std::cmp::Ordering;

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::components::{
    alert::{Alert, AlertVariant},
    badge::{Badge, BadgeVariant},
    breadcrumb::{Breadcrumb, BreadcrumbCurrent, BreadcrumbItem},
    content_tabs::{ContentTab, ContentTabs},
    section_heading::SubHeading,
    table::{Table, Tbody, Td, Th, Thead},
};
use crate::models::{
    DiffLineInfo, DiffTag, DocumentValidation, DocumentVersionInfo, HistoricalDocument,
    RevisionEntry, encode_path_segment,
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

/// Fetches a structured diff between a document version and its next newer version.
async fn fetch_diff(
    domain: String,
    tracking_id: String,
    commit_id: String,
) -> Result<Vec<DiffLineInfo>, String> {
    let resp = gloo_net::http::Request::get(&format!(
        "/api/providers/{}/document/{tracking_id}/versions/{commit_id}/diff",
        encode_path_segment(&domain)
    ))
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Err("Diff not available".to_string());
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
    let (tab, set_tab) = signal("overview".to_string());

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

    let diff = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        let v = selected_version.get();
        async move {
            match v {
                Some(commit_id) => Some(fetch_diff(d, t, commit_id).await),
                None => None,
            }
        }
    });

    view! {
        <div>
            <Breadcrumb>
                <BreadcrumbItem href=Signal::derive(|| "/".to_string())>"Providers"</BreadcrumbItem>
                <BreadcrumbItem href=Signal::derive(move || format!("/providers/{}", encode_path_segment(&domain())))>{move || domain()}</BreadcrumbItem>
                <BreadcrumbCurrent>{move || tracking_id()}</BreadcrumbCurrent>
            </Breadcrumb>

            {move || {
                if selected_version.get().is_some() {
                    view! {
                        <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading version..."</p> }>
                            {move || historical.get().map(|outer| match outer {
                                Some(Ok(doc)) => view! { <HistoricalDocumentView doc=doc tab=tab set_tab=set_tab /> }.into_any(),
                                Some(Err(e)) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                                None => view! { <span /> }.into_any(),
                            })}
                        </Suspense>
                        <Suspense fallback=|| view! { <span /> }>
                            {move || versions.get().map(|result| match result {
                                Ok(vs) => view! {
                                    <VersionSelector
                                        versions=vs
                                        selected=selected_version
                                        on_select=set_selected_version
                                    />
                                }.into_any(),
                                _ => view! { <span /> }.into_any(),
                            })}
                        </Suspense>
                        <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading diff..."</p> }>
                            {move || diff.get().map(|outer| match outer {
                                Some(Ok(lines)) => view! { <DiffView lines=lines /> }.into_any(),
                                Some(Err(e)) => view! { <p class="text-red-500 dark:text-red-400 text-sm py-4">"Diff unavailable: " {e}</p> }.into_any(),
                                None => view! { <span /> }.into_any(),
                            })}
                        </Suspense>
                    }.into_any()
                } else {
                    view! {
                        <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading..."</p> }>
                            {move || detail.get().map(|result| match result {
                                Ok(doc) => view! { <DocumentDetailView doc=doc tab=tab set_tab=set_tab /> }.into_any(),
                                Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                            })}
                        </Suspense>
                        <Suspense fallback=|| view! { <span /> }>
                            {move || versions.get().map(|result| match result {
                                Ok(vs) => view! {
                                    <VersionSelector
                                        versions=vs
                                        selected=selected_version
                                        on_select=set_selected_version
                                    />
                                }.into_any(),
                                _ => view! { <span /> }.into_any(),
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
        <div class="mb-2">
            <label class="text-sm text-gray-500 dark:text-gray-400 mr-2">"Version: "</label>
            <select
                class="bg-white dark:bg-gray-800 text-gray-700 dark:text-gray-200 border border-gray-200 dark:border-gray-700 rounded-lg px-3 py-2 text-sm cursor-pointer min-w-[300px]"
                on:change=move |ev| {
                    use wasm_bindgen::JsCast;
                    let target = ev.target().unwrap();
                    let val = target.unchecked_ref::<web_sys::HtmlSelectElement>().value();
                    if val == "latest" {
                        on_select.set(None);
                    } else {
                        on_select.set(Some(val));
                    }
                }
            >
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
fn HistoricalDocumentView(
    doc: HistoricalDocument,
    tab: ReadSignal<String>,
    set_tab: WriteSignal<String>,
) -> impl IntoView {
    let timestamp = doc.timestamp;
    let doc = StoredValue::new(doc);

    view! {
        <Alert variant=AlertVariant::Warning>
            "Showing version from " {format_timestamp(timestamp)}
            ". Validation results are only available for the current version."
        </Alert>

        <ContentTabs>
            <ContentTab
                active=Signal::derive(move || tab.get() != "history")
                on_click=Callback::new(move |_| set_tab.set("overview".into()))
            >"Overview"</ContentTab>
            <ContentTab
                active=Signal::derive(move || tab.get() == "history")
                on_click=Callback::new(move |_| set_tab.set("history".into()))
            >"History"</ContentTab>
        </ContentTabs>

        {move || {
            let d = doc.get_value();
            if tab.get() == "history" {
                view! {
                    <RevisionHistoryTable entries=d.revision_history />
                }.into_any()
            } else {
                view! {
                    <SubHeading>"Document"</SubHeading>
                    <Table>
                        <Tbody>
                            <MetadataRow label="Title" value=Some(d.title) />
                            <MetadataRow label="Category" value=d.category />
                            <MetadataRow label="Publisher" value=d.publisher_name />
                            <MetadataRow label="Severity" value=d.aggregate_severity />
                            <MetadataRow label="CSAF Version" value=d.csaf_version />
                        </Tbody>
                    </Table>

                    <SubHeading>"Tracking"</SubHeading>
                    <Table>
                        <Tbody>
                            <MetadataRow label="Status" value=d.status />
                            <MetadataRow label="Version" value=d.revision />
                            <MetadataRow label="Initial Release" value=d.initial_release_date />
                            <MetadataRow label="Current Release" value=d.current_release_date />
                        </Tbody>
                    </Table>
                }.into_any()
            }
        }}
    }
}

#[component]
fn DiffView(lines: Vec<DiffLineInfo>) -> impl IntoView {
    let additions = lines
        .iter()
        .filter(|l| matches!(l.tag, DiffTag::Insert))
        .count();
    let deletions = lines
        .iter()
        .filter(|l| matches!(l.tag, DiffTag::Delete))
        .count();

    view! {
        <SubHeading>"Changes (compared to next version)"</SubHeading>
        <p class="text-sm text-gray-500 dark:text-gray-400 mb-2">
            <span class="text-emerald-500">"+" {additions.to_string()} " added"</span>
            " "
            <span class="text-red-500">"-" {deletions.to_string()} " removed"</span>
        </p>
        <pre class="bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg overflow-x-auto text-xs p-0 mb-6">
            <code>
                {lines.into_iter().map(|line| {
                    let (class, prefix) = match line.tag {
                        DiffTag::Insert => ("bg-emerald-50 dark:bg-emerald-900/20 text-gray-800 dark:text-gray-200", "+"),
                        DiffTag::Delete => ("bg-red-50 dark:bg-red-900/20 text-gray-800 dark:text-gray-200", "-"),
                        DiffTag::Equal => ("text-gray-800 dark:text-gray-200", " "),
                    };
                    view! {
                        <div class={format!("px-3 py-0 whitespace-pre {class}")}>
                            {prefix}{" "}{line.content}
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </code>
        </pre>
    }
}

#[component]
fn DocumentDetailView(
    doc: DocumentValidation,
    tab: ReadSignal<String>,
    set_tab: WriteSignal<String>,
) -> impl IntoView {
    let doc = StoredValue::new(doc);

    view! {
        <ContentTabs>
            <ContentTab
                active=Signal::derive(move || tab.get() == "overview")
                on_click=Callback::new(move |_| set_tab.set("overview".into()))
            >"Overview"</ContentTab>
            <ContentTab
                active=Signal::derive(move || tab.get() == "validation")
                on_click=Callback::new(move |_| set_tab.set("validation".into()))
            >"Validation"</ContentTab>
            <ContentTab
                active=Signal::derive(move || tab.get() == "history")
                on_click=Callback::new(move |_| set_tab.set("history".into()))
            >"History"</ContentTab>
        </ContentTabs>

        {move || {
            let t = tab.get();
            let d = doc.get_value();
            if t == "validation" {
                view! {
                    <ProfileSection title="Basic" detail=d.profiles.basic />
                    <ProfileSection title="Extended" detail=d.profiles.extended />
                    <ProfileSection title="Full" detail=d.profiles.full />
                }.into_any()
            } else if t == "history" {
                view! {
                    <RevisionHistoryTable entries=d.revision_history />
                }.into_any()
            } else {
                let (sig_variant, sig_label) = if d.signature_error.is_some() {
                    (BadgeVariant::Danger, "Invalid")
                } else if d.signature_present {
                    (BadgeVariant::Success, "Valid")
                } else {
                    (BadgeVariant::Warning, "Missing")
                };
                view! {
                    <SubHeading>"Document"</SubHeading>
                    <Table>
                        <Tbody>
                            <MetadataRow label="Title" value=Some(d.title) />
                            <MetadataRow label="Category" value=d.category />
                            <MetadataRow label="Publisher" value=d.publisher_name />
                            <MetadataRow label="Severity" value=d.aggregate_severity />
                            <MetadataRow label="CSAF Version" value=d.csaf_version />
                            <tr>
                                <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">"URL"</Td>
                                <Td><a href={d.url.clone()} target="_blank">{d.url.clone()}</a></Td>
                            </tr>
                            <tr>
                                <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">"Signature"</Td>
                                <Td>
                                    <Badge variant=sig_variant>{sig_label}</Badge>
                                    {d.signature_error.map(|e| view! {
                                        <span class="text-sm text-red-500 dark:text-red-400 ml-2">{e}</span>
                                    })}
                                </Td>
                            </tr>
                        </Tbody>
                    </Table>

                    <SubHeading>"Tracking"</SubHeading>
                    <Table>
                        <Tbody>
                            <MetadataRow label="Status" value=d.status />
                            <MetadataRow label="Version" value=d.revision />
                            <MetadataRow label="Initial Release" value=d.initial_release_date />
                            <MetadataRow label="Current Release" value=d.current_release_date />
                        </Tbody>
                    </Table>
                }.into_any()
            }
        }}
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
            let (variant, badge_label) = if d.passed {
                (BadgeVariant::Success, "Pass".to_string())
            } else if d.error_count > 0 {
                (BadgeVariant::Danger, format!("{} errors", d.error_count))
            } else if d.warning_count > 0 {
                (
                    BadgeVariant::Warning,
                    format!("{} warnings", d.warning_count),
                )
            } else {
                (BadgeVariant::Info, format!("{} info", d.info_count))
            };

            view! {
                <h3 class="text-base font-medium text-gray-800 dark:text-white mt-6 mb-3">
                    {title}" "<Badge variant=variant>{badge_label}</Badge>
                </h3>
                {if d.failing_tests.is_empty() {
                    view! { <div /> }.into_any()
                } else {
                    let mut tests = d.failing_tests;
                    tests.sort_by(|a, b| numeric_test_id_cmp(&a.test_id, &b.test_id));
                    view! {
                        <Table>
                            <Thead>
                                <tr>
                                    <Th>"Severity"</Th>
                                    <Th>"Test ID"</Th>
                                    <Th>"Message"</Th>
                                </tr>
                            </Thead>
                            <Tbody>
                                {tests.into_iter().map(|f| {
                                    let variant = match f.severity.as_str() {
                                        "error" => BadgeVariant::Danger,
                                        "warning" => BadgeVariant::Warning,
                                        "info" => BadgeVariant::Info,
                                        _ => BadgeVariant::Neutral,
                                    };
                                    let sev_label = f.severity.clone();
                                    view! {
                                        <tr>
                                            <Td><Badge variant=variant>{sev_label}</Badge></Td>
                                            <Td>{f.test_id}</Td>
                                            <Td>{f.message}</Td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </Tbody>
                        </Table>
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
                <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">{label}</Td>
                <Td>{v}</Td>
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
        <SubHeading>"Revision History"</SubHeading>
        <Table>
            <Thead>
                <tr>
                    <Th>"Version"</Th>
                    <Th>"Date"</Th>
                    <Th>"Summary"</Th>
                </tr>
            </Thead>
            <Tbody>
                {entries.into_iter().map(|r| view! {
                    <tr>
                        <Td>{r.number}</Td>
                        <Td>{r.date}</Td>
                        <Td>{r.summary}</Td>
                    </tr>
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
    .into_any()
}
