use crate::components::{
    alert::{Alert, AlertVariant},
    badge::{Badge, BadgeVariant},
    breadcrumb::{Breadcrumb, BreadcrumbCurrent, BreadcrumbItem},
    content_tabs::{ContentTab, ContentTabs},
    diff_view::DiffView,
    document_checks::CheckBadge,
    pagination::Pagination,
    section_heading::SubHeading,
    table::{Table, Tbody, Td, Th, Thead},
    tlp_badge::TlpBadge,
};
use crate::models::{
    DiffLineInfo, DocumentValidation, DocumentVersionInfo, HistoricalDocument, PaginatedVersions,
    RevisionEntry, encode_path_segment,
};
use csaf_trove_common::document_content::{
    DocumentContent, Note, ProductStatusCount, Publisher, Reference, Vulnerability,
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

/// Fetches the displayable content of the current document version, `None` if unavailable.
async fn fetch_content(
    domain: String,
    tracking_id: String,
) -> Result<Option<DocumentContent>, String> {
    let resp = gloo_net::http::Request::get(&format!(
        "/api/providers/{}/document/{tracking_id}/content",
        encode_path_segment(&domain)
    ))
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Ok(None);
    }
    resp.json().await.map(Some).map_err(|e| e.to_string())
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

/// Document detail page with Overview, Notes & References, Vulnerabilities, Validation,
/// Revision, and History tabs.
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

    // Needed by the Overview tab as well, so always loaded.
    let content = LocalResource::new(move || {
        let d = domain();
        let t = tracking_id();
        async move { fetch_content(d, t).await }
    });
    let loaded_content = Signal::derive(move || content.get().and_then(Result::ok).flatten());

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
                    active=Signal::derive(move || tab.get() == "notes")
                    on_click=Callback::new(move |_| {
                        navigate.run(format!("{}/notes", document_url()));
                    })
                >"Notes & References"</ContentTab>
                <ContentTab
                    active=Signal::derive(move || tab.get() == "vulnerabilities")
                    on_click=Callback::new(move |_| {
                        navigate.run(format!("{}/vulnerabilities", document_url()));
                    })
                >
                    "Vulnerabilities"
                    {move || loaded_content.get().map(|c| format!(" ({})", c.vulnerabilities.len()))}
                </ContentTab>
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
                let t = tab.get();
                if t == "notes" || t == "vulnerabilities" {
                    view! {
                        <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading..."</p> }>
                            {move || content.get().map(|result| match result {
                                Ok(Some(c)) if tab.get() == "notes" => view! {
                                    <NotesSection notes=c.notes />
                                    <ReferencesSection references=c.references />
                                }.into_any(),
                                Ok(Some(c)) => view! { <VulnerabilitiesTable vulnerabilities=c.vulnerabilities /> }.into_any(),
                                Ok(None) => view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Document content not available."</p> }.into_any(),
                                Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                            })}
                        </Suspense>
                    }.into_any()
                } else if t == "history" {
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
                                Ok(doc) => view! { <DocumentDetailContent doc=doc tab=tab content=loaded_content /> }.into_any(),
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
fn DocumentDetailContent(
    doc: DocumentValidation,
    tab: Signal<String>,
    /// Extracted document content, once loaded.
    content: Signal<Option<DocumentContent>>,
) -> impl IntoView {
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
                view! {
                    <SubHeading>"Document"</SubHeading>
                    <Table>
                        <Tbody>
                            <MetadataRow label="Title" value=Some(d.title) />
                            <MetadataRow label="Category" value=d.category />
                            {move || content.get().and_then(|c| c.tlp).map(|tlp| view! {
                                <tr>
                                    <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">"TLP"</Td>
                                    <Td><TlpBadge label=tlp /></Td>
                                </tr>
                            })}
                            {
                                let fallback = d.publisher_name.clone();
                                move || match content.get().and_then(|c| c.publisher) {
                                    Some(publisher) => view! { <PublisherRow publisher=publisher /> }.into_any(),
                                    None => view! { <MetadataRow label="Publisher" value=fallback.clone() /> }.into_any(),
                                }
                            }
                            <MetadataRow label="Severity" value=d.aggregate_severity />
                            <MetadataRow label="CSAF Version" value=d.csaf_version />
                            <tr>
                                <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">"URL"</Td>
                                <Td><a href={d.url.clone()} target="_blank">{d.url.clone()}</a></Td>
                            </tr>
                            {[
                                ("Retrieval", d.checks.retrieval),
                                ("Parsing", d.checks.parsing),
                                ("Signature", d.checks.signature),
                                ("Digests", d.checks.digest),
                            ].into_iter().map(|(label, outcome)| view! {
                                <tr>
                                    <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">{label}</Td>
                                    <Td>
                                        <CheckBadge outcome=outcome.clone() />
                                        {outcome.message.map(|message| view! { <span class="ml-2">{message}</span> })}
                                    </Td>
                                </tr>
                            }).collect::<Vec<_>>()}

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
        None => view! {
            <h3 class="text-base font-medium text-gray-800 dark:text-white mt-6 mb-3">
                {title}" "<Badge variant=BadgeVariant::Neutral>"Not evaluated"</Badge>
            </h3>
        }
        .into_any(),
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

/// Renders the publisher name with its category as a label, plus namespace and contact details.
#[component]
fn PublisherRow(publisher: Publisher) -> impl IntoView {
    let details = [publisher.contact_details, publisher.issuing_authority]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    view! {
        <tr>
            <Td class="text-xs font-semibold uppercase text-gray-500 dark:text-gray-400 w-48">"Publisher"</Td>
            <Td>
                <div class="flex flex-wrap items-center gap-2">
                    <span>{publisher.name}</span>
                    {publisher.category.map(|category| view! {
                        <Badge variant=BadgeVariant::Info>{category}</Badge>
                    })}
                    {publisher.namespace.map(|namespace| {
                        let href = namespace.clone();
                        view! { <a href=href target="_blank" rel="noopener noreferrer">{namespace}</a> }
                    })}
                </div>
                {details.into_iter().map(|detail| view! {
                    <div class="mt-1 text-xs text-gray-500 dark:text-gray-400 whitespace-pre-wrap">{detail}</div>
                }).collect::<Vec<_>>()}
            </Td>
        </tr>
    }
}

/// Renders a muted placeholder line for an empty section.
#[component]
fn EmptySection(message: &'static str) -> impl IntoView {
    view! { <p class="text-sm text-gray-500 dark:text-gray-400">{message}</p> }
}

/// Renders a list of notes with their title, category, and text.
#[component]
fn NoteList(notes: Vec<Note>) -> impl IntoView {
    view! {
        <div class="space-y-4">
            {notes.into_iter().map(|note| view! {
                <div class="p-4 border border-gray-200 rounded-lg dark:border-gray-700 bg-white dark:bg-gray-900">
                    <div class="flex flex-wrap items-center gap-2 mb-2">
                        {note.title.map(|title| view! {
                            <span class="font-medium text-gray-800 dark:text-white">{title}</span>
                        })}
                        {note.category.map(|category| view! {
                            <Badge variant=BadgeVariant::Neutral>{category}</Badge>
                        })}
                    </div>
                    <p class="text-sm text-gray-700 dark:text-gray-300 whitespace-pre-wrap">{note.text}</p>
                </div>
            }).collect::<Vec<_>>()}
        </div>
    }
}

/// Renders the document notes section.
#[component]
fn NotesSection(notes: Vec<Note>) -> impl IntoView {
    view! {
        <SubHeading>"Notes"</SubHeading>
        {if notes.is_empty() {
            view! { <EmptySection message="No notes." /> }.into_any()
        } else {
            view! { <NoteList notes=notes /> }.into_any()
        }}
    }
}

/// Renders a table of references with their category, summary, and link.
#[component]
fn ReferenceTable(references: Vec<Reference>) -> impl IntoView {
    view! {
        <Table>
            <Thead>
                <tr>
                    <Th>"Category"</Th>
                    <Th>"Summary"</Th>
                    <Th>"URL"</Th>
                </tr>
            </Thead>
            <Tbody>
                {references.into_iter().map(|reference| {
                    let href = reference.url.clone();
                    view! {
                    <tr>
                        <Td>{reference.category.map(|category| view! {
                            <Badge variant=BadgeVariant::Neutral>{category}</Badge>
                        })}</Td>
                        <Td>{reference.summary}</Td>
                        <Td class="break-all">
                            <a href=href target="_blank" rel="noopener noreferrer">{reference.url}</a>
                        </Td>
                    </tr>
                    }
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
}

/// Renders the document references section.
#[component]
fn ReferencesSection(references: Vec<Reference>) -> impl IntoView {
    view! {
        <SubHeading>"References"</SubHeading>
        {if references.is_empty() {
            view! { <EmptySection message="No references." /> }.into_any()
        } else {
            view! { <ReferenceTable references=references /> }.into_any()
        }}
    }
}

/// Maps a CVSS severity to a badge variant.
fn severity_variant(severity: &str) -> BadgeVariant {
    match severity.to_ascii_uppercase().as_str() {
        "CRITICAL" | "HIGH" => BadgeVariant::Danger,
        "MEDIUM" => BadgeVariant::Warning,
        "LOW" => BadgeVariant::Info,
        _ => BadgeVariant::Neutral,
    }
}

/// Maps a product status key to a short label and badge variant.
fn product_status_label(status: &str) -> (&str, BadgeVariant) {
    match status {
        "known_affected" => ("affected", BadgeVariant::Danger),
        "first_affected" => ("first affected", BadgeVariant::Danger),
        "last_affected" => ("last affected", BadgeVariant::Danger),
        "under_investigation" => ("under investigation", BadgeVariant::Warning),
        "fixed" => ("fixed", BadgeVariant::Success),
        "first_fixed" => ("first fixed", BadgeVariant::Success),
        "known_not_affected" => ("not affected", BadgeVariant::Success),
        "recommended" => ("recommended", BadgeVariant::Info),
        other => (other, BadgeVariant::Neutral),
    }
}

/// Renders product status counts as compact badges.
#[component]
fn ProductStatusBadges(statuses: Vec<ProductStatusCount>) -> impl IntoView {
    view! {
        <div class="flex flex-wrap gap-1">
            {statuses.into_iter().map(|s| {
                let (label, variant) = product_status_label(&s.status);
                let text = format!("{} {label}", s.count);
                view! { <Badge variant=variant>{text}</Badge> }
            }).collect::<Vec<_>>()}
        </div>
    }
}

/// Renders the vulnerabilities of a document as a table.
#[component]
fn VulnerabilitiesTable(vulnerabilities: Vec<Vulnerability>) -> impl IntoView {
    if vulnerabilities.is_empty() {
        return view! {
            <p class="text-gray-500 dark:text-gray-400 text-center py-12">"No vulnerabilities."</p>
        }
        .into_any();
    }
    view! {
        <Table>
            <Thead>
                <tr>
                    <Th>"ID"</Th>
                    <Th>"Title"</Th>
                    <Th>"CWE"</Th>
                    <Th>"Severity"</Th>
                    <Th>"Product Status"</Th>
                </tr>
            </Thead>
            <Tbody>
                {vulnerabilities.into_iter().map(|v| {
                    let severity = match (v.score, v.severity) {
                        (Some(score), Some(severity)) => Some((format!("{score:.1} {severity}"), severity_variant(&severity))),
                        (Some(score), None) => Some((format!("{score:.1}"), BadgeVariant::Neutral)),
                        (None, Some(severity)) => {
                            let variant = severity_variant(&severity);
                            Some((severity, variant))
                        }
                        (None, None) => None,
                    };
                    view! {
                        <tr>
                            <Td class="whitespace-nowrap">
                                {v.cve.map(|cve| view! { <div class="font-medium">{cve}</div> })}
                                {v.ids.into_iter().map(|id| view! {
                                    <div class="text-xs text-gray-500 dark:text-gray-400">{id}</div>
                                }).collect::<Vec<_>>()}
                            </Td>
                            <Td>{v.title.unwrap_or_default()}</Td>
                            <Td>
                                {v.cwes.into_iter().map(|cwe| {
                                    let name = cwe.name.unwrap_or_default();
                                    view! { <div class="whitespace-nowrap" title=name>{cwe.id}</div> }
                                }).collect::<Vec<_>>()}
                            </Td>
                            <Td class="whitespace-nowrap">
                                {severity.map(|(label, variant)| view! { <Badge variant=variant>{label}</Badge> })}
                            </Td>
                            <Td><ProductStatusBadges statuses=v.product_status /></Td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
    .into_any()
}
