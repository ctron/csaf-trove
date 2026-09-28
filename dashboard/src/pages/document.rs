use crate::components::{
    alert::{Alert, AlertVariant},
    badge::{Badge, BadgeVariant},
    breadcrumb::{Breadcrumb, BreadcrumbCurrent, BreadcrumbItem},
    content_tabs::{ContentTab, ContentTabs},
    diff_view::DiffView,
    pagination::Pagination,
    section_heading::SubHeading,
    table::{Table, Tbody, Td, Th, Thead},
};
use crate::models::{
    DiffLineInfo, DocumentValidation, DocumentVersionInfo, HistoricalDocument, PaginatedVersions,
    RevisionEntry, encode_path_segment,
};
use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use std::cmp::Ordering;

/// Number of versions shown per history page.
const VERSIONS_PAGE_SIZE: u64 = 50;

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

/// Fetches the current document detail from the API.
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

/// Fetches a page of the version history for a document, newest first.
async fn fetch_versions(
    domain: String,
    tracking_id: String,
    offset: u64,
) -> Result<PaginatedVersions, String> {
    let resp = gloo_net::http::Request::get(&format!(
        "/api/providers/{}/document/{tracking_id}/versions?offset={offset}&limit={VERSIONS_PAGE_SIZE}",
        encode_path_segment(&domain)
    ))
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Ok(PaginatedVersions {
            items: vec![],
            total: 0,
            offset,
            limit: VERSIONS_PAGE_SIZE,
        });
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Fetches a historical version of a document by commit ID.
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

/// Document detail page with Overview, Validation, Revision, and History tabs.
#[component]
pub fn DocumentPage() -> impl IntoView {
    let params = use_params_map();
    let domain = move || params.read().get("domain").unwrap_or_default();
    let tracking_id = move || params.read().get("tracking_id").unwrap_or_default();

    let query = use_query_map();
    let navigate = use_navigate();
    let navigate = Callback::new(move |url: String| navigate(&url, Default::default()));
    let selected_version = Signal::derive(move || params.read().get("commit_id"));
    let tab = Signal::derive(move || {
        params
            .read()
            .get("tab")
            .unwrap_or_else(|| "overview".into())
    });
    let document_url = move || {
        format!(
            "/providers/{}/documents/{}",
            encode_path_segment(&domain()),
            encode_path_segment(&tracking_id()),
        )
    };
    let versions_offset = Signal::derive(move || {
        query
            .read()
            .get("offset")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
    });
    let history_url = move || {
        format!(
            "{}/history?offset={}",
            document_url(),
            versions_offset.get()
        )
    };

    let detail = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        async move { fetch_document(d, t).await }
    });

    // Only load history once the tab is opened.
    let versions = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        let active = tab.get() == "history";
        let offset = versions_offset.get();
        async move {
            if active {
                Some(fetch_versions(d, t, offset).await)
            } else {
                None
            }
        }
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
                {move || if tab.get() == "history" {
                    view! {
                        <BreadcrumbItem href=Signal::derive(document_url)>{move || tracking_id()}</BreadcrumbItem>
                        {move || if let Some(commit_id) = selected_version.get() {
                            view! {
                                <BreadcrumbItem href=Signal::derive(history_url)>"History"</BreadcrumbItem>
                                <BreadcrumbCurrent>{commit_id}</BreadcrumbCurrent>
                            }.into_any()
                        } else {
                            view! { <BreadcrumbCurrent>"History"</BreadcrumbCurrent> }.into_any()
                        }}
                    }.into_any()
                } else {
                    view! { <BreadcrumbCurrent>{move || tracking_id()}</BreadcrumbCurrent> }.into_any()
                }}
            </Breadcrumb>

            <ContentTabs>
                <ContentTab
                    active=Signal::derive(move || tab.get() == "overview")
                    on_click=Callback::new(move |_| {
                        navigate.run(document_url());
                    })
                >"Overview"</ContentTab>
                <ContentTab
                    active=Signal::derive(move || tab.get() == "validation")
                    on_click=Callback::new(move |_| {
                        navigate.run(format!("{}/validation", document_url()));
                    })
                >"Validation"</ContentTab>
                <ContentTab
                    active=Signal::derive(move || tab.get() == "revision")
                    on_click=Callback::new(move |_| {
                        navigate.run(format!("{}/revision", document_url()));
                    })
                >"Revision"</ContentTab>
                <ContentTab
                    active=Signal::derive(move || tab.get() == "history")
                    on_click=Callback::new(move |_| navigate.run(history_url()))
                >"History"</ContentTab>
            </ContentTabs>

            {move || {
                if tab.get() == "history" {
                    view! {
                        {move || {
                            if selected_version.get().is_some() {
                                view! {
                                    <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading version..."</p> }>
                                        {move || historical.get().map(|outer| match outer {
                                            Some(Ok(doc)) => view! { <HistoricalVersionDetail doc=doc /> }.into_any(),
                                            Some(Err(e)) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                                            None => view! { <span /> }.into_any(),
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
                                    <Transition fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading versions..."</p> }>
                                        {move || versions.get().flatten().map(|result| match result {
                                            Ok(page) => {
                                                let total = page.total;
                                                let count = page.items.len() as u64;
                                                view! {
                                                    <VersionListTable
                                                        versions=page.items
                                                        on_select=Callback::new(move |commit_id: String| navigate.run(format!(
                                                            "{}/history/{}?offset={}", document_url(), encode_path_segment(&commit_id), versions_offset.get()
                                                        )))
                                                    />
                                                    {(total > VERSIONS_PAGE_SIZE).then(|| view! {
                                                        <Pagination
                                                            offset=versions_offset
                                                            limit=VERSIONS_PAGE_SIZE
                                                            total=total
                                                            count=count
                                                            on_change=Callback::new(move |o: u64| navigate.run(format!("{}/history?offset={o}", document_url())))
                                                        />
                                                    })}
                                                }.into_any()
                                            }
                                            Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                                        })}
                                    </Transition>
                                }.into_any()
                            }
                        }}
                    }.into_any()
                } else {
                    view! {
                        <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading..."</p> }>
                            {move || detail.get().map(|result| match result {
                                Ok(doc) => view! { <DocumentDetailContent doc=doc tab=tab /> }.into_any(),
                                Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                            })}
                        </Suspense>
                    }.into_any()
                }
            }}
        </div>
    }
}

/// Renders the version history as a table with clickable rows.
#[component]
fn VersionListTable(
    versions: Vec<DocumentVersionInfo>,
    on_select: Callback<String>,
) -> impl IntoView {
    if versions.is_empty() {
        return view! {
            <p class="text-gray-500 dark:text-gray-400 text-center py-12">"No version history available."</p>
        }
        .into_any();
    }
    view! {
        <Table>
            <Thead>
                <tr>
                    <Th>"Date"</Th>
                    <Th>"Status"</Th>
                    <Th>"Version"</Th>
                    <Th>"Current Release"</Th>
                    <Th>"Message"</Th>
                    <Th>" "</Th>
                </tr>
            </Thead>
            <Tbody>
                {versions.into_iter().map(|v| {
                    let date = format_timestamp(v.timestamp);
                    let status = v.status.clone();
                    let version = v.version.clone().unwrap_or_else(|| "—".to_string());
                    let release = v.current_release_date.clone();
                    let message = v.message.lines().next().unwrap_or("").to_string();
                    let is_latest = v.is_latest;
                    let commit_id = v.commit_id.clone();
                    let row_class = if is_latest {
                        ""
                    } else {
                        "cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800/50"
                    };
                    view! {
                        <tr
                            class=row_class
                            on:click=move |_| {
                                if !is_latest {
                                    on_select.run(commit_id.clone());
                                }
                            }
                        >
                            <Td>{date}</Td>
                            <Td><TrackingStatusBadge status=status /></Td>
                            <Td>{version}</Td>
                            <Td><ReleaseDate value=release /></Td>
                            <Td>{message}</Td>
                            <Td>
                                {is_latest.then(|| view! {
                                    <Badge variant=BadgeVariant::Success>"Current"</Badge>
                                })}
                            </Td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
    .into_any()
}

/// Renders a CSAF tracking status as a colored badge, or a dash when unknown.
#[component]
fn TrackingStatusBadge(status: Option<String>) -> impl IntoView {
    let Some(status) = status else {
        return view! { <span>"—"</span> }.into_any();
    };
    let variant = match status.as_str() {
        "final" => BadgeVariant::Success,
        "interim" => BadgeVariant::Warning,
        _ => BadgeVariant::Neutral,
    };
    view! { <Badge variant=variant>{status}</Badge> }.into_any()
}

/// Shows the date part of a release timestamp, with the full value as a tooltip.
#[component]
fn ReleaseDate(value: Option<String>) -> impl IntoView {
    let Some(value) = value else {
        return view! { <span>"—"</span> }.into_any();
    };
    let date = value.split('T').next().unwrap_or(&value).to_string();
    view! { <span title=value>{date}</span> }.into_any()
}

/// Displays metadata for a historical document version.
#[component]
fn HistoricalVersionDetail(doc: HistoricalDocument) -> impl IntoView {
    view! {
        <Alert variant=AlertVariant::Warning>
            "Showing version from " {format_timestamp(doc.timestamp)}
            ". Validation results are only available for the current version."
        </Alert>

        <SubHeading>"Document"</SubHeading>
        <Table>
            <Tbody>
                <MetadataRow label="Title" value=Some(doc.title) />
                <MetadataRow label="Category" value=doc.category />
                <MetadataRow label="Publisher" value=doc.publisher_name />
                <MetadataRow label="Severity" value=doc.aggregate_severity />
                <MetadataRow label="CSAF Version" value=doc.csaf_version />
            </Tbody>
        </Table>

        <SubHeading>"Tracking"</SubHeading>
        <Table>
            <Tbody>
                <MetadataRow label="Status" value=doc.status />
                <MetadataRow label="Version" value=doc.revision />
                <MetadataRow label="Initial Release" value=doc.initial_release_date />
                <MetadataRow label="Current Release" value=doc.current_release_date />
            </Tbody>
        </Table>
    }
}

/// Renders document content for the Overview, Validation, and Revision tabs.
#[component]
fn DocumentDetailContent(doc: DocumentValidation, tab: Signal<String>) -> impl IntoView {
    let doc = StoredValue::new(doc);

    view! {
        {move || {
            let t = tab.get();
            let d = doc.get_value();
            if t == "validation" {
                view! {
                    <ProfileSection title="Basic" detail=d.profiles.basic />
                    <ProfileSection title="Extended" detail=d.profiles.extended />
                    <ProfileSection title="Full" detail=d.profiles.full />
                }.into_any()
            } else if t == "revision" {
                view! {
                    <RevisionHistoryTable entries=d.revision_history />
                }.into_any()
            } else {
                let (sig_variant, sig_label) = if d.signature_error.is_some() {
                    (BadgeVariant::Danger, "Invalid")
                } else if d.signature_warning.is_some() {
                                        (BadgeVariant::Warning, "Valid with warnings")
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
                                <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">"Integrity"</Td>
                                <Td>
                                    <Badge variant=sig_variant>{sig_label}</Badge>
                                    {d.signature_warning.map(|e| view! {
                                        <span class="text-sm text-amber-600 dark:text-amber-400 ml-2">{e}</span>
                                    })}
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

/// Renders a validation profile section with pass/fail badge and failing tests.
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

/// Renders a single metadata key-value row.
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

/// Renders the CSAF document revision history as a table.
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
