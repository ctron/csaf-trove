use leptos::prelude::*;
use leptos_router::{
    NavigateOptions,
    hooks::{query_signal_with_options, use_params_map},
};

use crate::components::{
    badge::{Badge, BadgeVariant},
    breadcrumb::{Breadcrumb, BreadcrumbCurrent, BreadcrumbItem},
    content_tabs::{ContentTab, ContentTabs},
    doc_profile_badge::DocProfileBadge,
    empty_state::EmptyState,
    pagination::Pagination,
    progress_bar::{ProgressBar, ProgressColor, color_for_pass_rate},
    table::{Table, Tbody, Td, Th, Thead},
    tabs::{Tab, Tabs},
    tlp_badge::TlpBadge,
};
use crate::models::{
    DistributionHealth, PaginatedDocuments, ProfileSummary, ProviderDetail, encode_path_segment,
};

/// Fetches provider detail from the API. Returns `None` for 404 (no sync yet).
async fn fetch_provider(domain: String) -> Result<Option<ProviderDetail>, String> {
    let resp =
        gloo_net::http::Request::get(&format!("/api/providers/{}", encode_path_segment(&domain)))
            .send()
            .await
            .map_err(|e| e.to_string())?;
    if resp.status() == 404 {
        return Ok(None);
    }
    if !resp.ok() {
        return Err(format!("Failed to load provider (HTTP {})", resp.status()));
    }
    resp.json().await.map(Some).map_err(|e| e.to_string())
}

/// Provider detail page with grouped cards, content tabs, and documents table.
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
            <Breadcrumb>
                <BreadcrumbItem href=Signal::derive(|| "/".to_string())>"Providers"</BreadcrumbItem>
                <BreadcrumbCurrent>{move || domain()}</BreadcrumbCurrent>
            </Breadcrumb>

            <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading..."</p> }>
                {move || detail.get().map(|result| match result {
                    Ok(Some(d)) => view! { <ProviderDetailView detail=d /> }.into_any(),
                    Ok(None) => view! {
                        <EmptyState
                            title="No data available yet".to_string()
                            message="This provider has not been synced yet. Data will appear here after the first successful sync.".to_string()
                            action_href=format!("/sync/{}", encode_path_segment(&domain()))
                            action_label="View Sync Status".to_string()
                        />
                    }.into_any(),
                    Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

/// Renders a single profile row with label, pass rate, progress bar, and counts.
fn profile_row(label: &'static str, profile: Option<ProfileSummary>) -> impl IntoView {
    match profile {
        Some(p) => {
            let pct = p.pass_rate * 100.0;
            let color = color_for_pass_rate(p.pass_rate);
            let rate_label = format!("{pct:.1}%");
            let detail = format!("{} valid \u{00b7} {} invalid", p.valid, p.invalid);
            view! {
                <div class="mb-4 last:mb-0">
                    <div class="flex items-center justify-between mb-1">
                        <span class="text-sm font-medium text-gray-700 dark:text-gray-300">{label}</span>
                        <span class="text-sm font-medium text-gray-700 dark:text-gray-300">{rate_label}</span>
                    </div>
                    <ProgressBar percentage=pct color=color />
                    <p class="text-xs text-gray-500 dark:text-gray-400 mt-1">{detail}</p>
                </div>
            }
            .into_any()
        }
        None => view! {
            <div class="mb-4 last:mb-0">
                <div class="flex items-center justify-between mb-1">
                    <span class="text-sm font-medium text-gray-700 dark:text-gray-300">{label}</span>
                    <span class="text-sm text-gray-400 dark:text-gray-500">"not tested"</span>
                </div>
            </div>
        }
        .into_any(),
    }
}

/// A single problem badge entry for [`HealthRow`].
struct HealthBadge {
    count: u64,
    label: &'static str,
    variant: BadgeVariant,
}

/// A bordered health row: shows a green ok count plus any non-zero problem badges.
#[component]
fn HealthRow(label: &'static str, ok_count: u64, badges: Vec<HealthBadge>) -> impl IntoView {
    let problems: Vec<_> = badges.into_iter().filter(|b| b.count > 0).collect();
    view! {
        <div class="border-t border-gray-200 dark:border-gray-700 pt-4 mt-4">
            <div class="flex items-center gap-3">
                <span class="text-sm font-medium text-gray-700 dark:text-gray-300">{label}</span>
                <Badge variant=BadgeVariant::Success>{ok_count}" ok"</Badge>
                {problems.into_iter().map(|b| view! {
                    <Badge variant=b.variant>{b.count}" "{b.label}</Badge>
                }).collect_view()}
            </div>
        </div>
    }
}

/// Main detail view with grouped cards, info line, and tabbed content.
#[component]
fn ProviderDetailView(detail: ProviderDetail) -> impl IntoView {
    let summary = detail.summary;
    let distributions = detail.distributions;
    let tests = summary.top_failing_tests;
    let retrieval_errors = summary.retrieval_errors;
    let domain = summary.provider.clone();
    let sync_href = format!("/sync/{}", encode_path_segment(&domain));

    let (active_tab, set_active_tab) = signal("documents".to_string());

    view! {
        // Two-card grid
        <div class="grid grid-cols-1 gap-6 md:grid-cols-2 mb-6">
            // Overview card
            <div class="bg-white rounded-lg shadow-md dark:bg-gray-800 p-6">
                <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4">"Overview"</h2>
                <p class="text-4xl font-bold text-gray-800 dark:text-white">{summary.document_count}</p>
                <p class="text-sm text-gray-500 dark:text-gray-400 mb-4">"documents"</p>

                <div class="space-y-2 text-sm text-gray-600 dark:text-gray-400">
                    {summary.publisher_name.map(|name| view! {
                        <p><span class="text-gray-500 dark:text-gray-500">"Publisher: "</span><span class="text-gray-700 dark:text-gray-300">{name}</span></p>
                    })}
                    <p><span class="text-gray-500 dark:text-gray-500">"Last synced: "</span><span class="text-gray-700 dark:text-gray-300">{summary.validated_at}</span></p>
                </div>

                {summary.signatures.map(|sig| view! {
                    <HealthRow label="Signatures" ok_count=sig.valid badges=vec![
                        HealthBadge { count: sig.invalid, label: "invalid", variant: BadgeVariant::Danger },
                        HealthBadge { count: sig.missing, label: "missing", variant: BadgeVariant::Warning },
                    ] />
                })}

                <HealthRow label="Retrieval" ok_count={summary.document_count - retrieval_errors} badges=vec![
                    HealthBadge { count: retrieval_errors, label: "failed", variant: BadgeVariant::Danger },
                ] />

                <div class="border-t border-gray-200 dark:border-gray-700 pt-4 mt-4">
                    <a href={sync_href} class="text-sm text-blue-600 dark:text-blue-400 hover:underline">"Sync History \u{2192}"</a>
                </div>
            </div>

            // Validation card
            <div class="bg-white rounded-lg shadow-md dark:bg-gray-800 p-6">
                <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4">"Validation"</h2>
                {profile_row("Basic", summary.profiles.basic.clone())}
                {profile_row("Extended", summary.profiles.extended.clone())}
                {profile_row("Full", summary.profiles.full)}
            </div>
        </div>

        // Distribution health card (only when distributions are available)
        {(!distributions.is_empty()).then(|| view! {
            <DistributionsCard distributions=distributions />
        })}

        // Content tabs
        <ContentTabs>
            <ContentTab
                active=Signal::derive(move || active_tab.get() == "documents")
                on_click=Callback::new(move |_| set_active_tab.set("documents".to_string()))
            >"Documents"</ContentTab>
            <ContentTab
                active=Signal::derive(move || active_tab.get() == "tests")
                on_click=Callback::new(move |_| set_active_tab.set("tests".to_string()))
            >"Failing Tests"</ContentTab>
        </ContentTabs>

        // Tab content
        <div>
            {move || {
                let tab = active_tab.get();
                if tab == "documents" {
                    view! { <DocumentsTable domain=domain.clone() /> }.into_any()
                } else {
                    let tests = tests.clone();
                    view! { <FailingTestsView tests=tests /> }.into_any()
                }
            }}
        </div>
    }
}

/// Displays the top failing tests table, or a success message when empty.
#[component]
fn FailingTestsView(tests: Vec<crate::models::FailingTest>) -> impl IntoView {
    if tests.is_empty() {
        return view! {
            <p class="text-gray-500 dark:text-gray-400 text-center py-12">"No failing tests."</p>
        }
        .into_any();
    }

    view! {
        <Table>
            <Thead>
                <tr>
                    <Th>"Test ID"</Th>
                    <Th>"Count"</Th>
                    <Th>"Severity"</Th>
                </tr>
            </Thead>
            <Tbody>
                {tests.into_iter().map(|t| {
                    let test_id = t.test_id.clone();
                    let variant = match t.severity.as_str() {
                        "error" => BadgeVariant::Danger,
                        "warning" => BadgeVariant::Warning,
                        "info" => BadgeVariant::Info,
                        _ => BadgeVariant::Neutral,
                    };
                    let sev_label = t.severity.clone();
                    view! {
                        <tr>
                            <Td>{test_id}</Td>
                            <Td>{t.count}</Td>
                            <Td><Badge variant=variant>{sev_label}</Badge></Td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
    .into_any()
}

/// Renders a pass rate as a colored badge or "n/a".
fn rate_badge(rate: Option<f64>) -> impl IntoView {
    match rate {
        Some(r) => {
            let pct = r * 100.0;
            let variant = match color_for_pass_rate(r) {
                ProgressColor::Emerald => BadgeVariant::Success,
                ProgressColor::Amber => BadgeVariant::Warning,
                ProgressColor::Red => BadgeVariant::Danger,
            };
            view! { <Badge variant=variant>{format!("{pct:.1}%")}</Badge> }.into_any()
        }
        None => view! {
            <span class="text-gray-400 dark:text-gray-500">"n/a"</span>
        }
        .into_any(),
    }
}

/// Renders a table card showing per-distribution health metrics.
#[component]
fn DistributionsCard(distributions: Vec<DistributionHealth>) -> impl IntoView {
    view! {
        <div class="bg-white rounded-lg shadow-md dark:bg-gray-800 p-6 mb-6">
            <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4">"Distributions"</h2>
            <Table>
                <Thead>
                    <tr>
                        <Th>"Distribution"</Th>
                        <Th>"Documents"</Th>
                        <Th>"Basic"</Th>
                        <Th>"Extended"</Th>
                        <Th>"Full"</Th>
                        <Th>"Errors"</Th>
                    </tr>
                </Thead>
                <Tbody>
                    {distributions.into_iter().map(|d| {
                        let kind_label = match d.kind.as_str() {
                            "rolie" => "ROLIE",
                            _ => "Directory",
                        };
                        let tlp_badges = d.tlp_labels.iter().map(|tlp| {
                            let label = tlp.clone();
                            view! { <TlpBadge label=label/> }
                        }).collect::<Vec<_>>();
                        let error_cell = if d.skipped {
                            view! {
                                <Badge variant=BadgeVariant::Neutral>"Skipped"</Badge>
                            }.into_any()
                        } else if let Some(error) = d.distribution_error.clone() {
                            view! {
                                <Badge variant=BadgeVariant::Danger>{error}</Badge>
                            }.into_any()
                        } else {
                            let err_variant = if d.retrieval_errors > 0 {
                                BadgeVariant::Danger
                            } else {
                                BadgeVariant::Success
                            };
                            view! {
                                <Badge variant=err_variant>{d.retrieval_errors}</Badge>
                            }.into_any()
                        };
                        view! {
                            <tr>
                                <Td>
                                    <div class="flex items-center gap-2">
                                        <span class="font-medium text-gray-700 dark:text-gray-300">{d.label.clone()}</span>
                                        <Badge variant=BadgeVariant::Info>{kind_label}</Badge>
                                        {tlp_badges}
                                    </div>
                                </Td>
                                <Td>{d.document_count}</Td>
                                <Td>{rate_badge(d.basic_pass_rate)}</Td>
                                <Td>{rate_badge(d.extended_pass_rate)}</Td>
                                <Td>{rate_badge(d.full_pass_rate)}</Td>
                                <Td>{error_cell}</Td>
                            </tr>
                        }
                    }).collect::<Vec<_>>()}
                </Tbody>
            </Table>
        </div>
    }
}

/// Fetches paginated documents from the API.
async fn fetch_documents(
    domain: String,
    offset: u64,
    limit: u64,
    status: Option<String>,
) -> Result<PaginatedDocuments, String> {
    let mut url = format!(
        "/api/providers/{}/document?offset={offset}&limit={limit}",
        encode_path_segment(&domain)
    );
    if let Some(s) = &status {
        url.push_str(&format!("&status={s}"));
    }
    let resp = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(format!("Failed to load documents ({})", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Paginated documents table with status filter tabs.
#[component]
fn DocumentsTable(domain: String) -> impl IntoView {
    let nav = NavigateOptions {
        scroll: false,
        ..Default::default()
    };
    let (offset_param, set_offset_param) = query_signal_with_options::<u64>("offset", nav.clone());
    let offset = Signal::derive(move || offset_param.get().unwrap_or(0));

    let (status_param, set_status_param) = query_signal_with_options::<String>("status", nav);
    let status_filter = Signal::derive(move || status_param.get());

    let limit = 10u64;
    let domain = StoredValue::new(domain);

    let docs = LocalResource::new(move || {
        let d = domain.get_value();
        let o = offset.get();
        let s = status_filter.get();
        async move { fetch_documents(d, o, limit, s).await }
    });

    view! {
        <div class="mt-6 md:flex md:items-center md:justify-between">
            <Tabs>
                <Tab
                    active=Signal::derive(move || status_filter.get().is_none())
                    on_click=Callback::new(move |_| { set_status_param.set(None); set_offset_param.set(None); })
                >"All"</Tab>
                <Tab
                    active=Signal::derive(move || status_filter.get().as_deref() == Some("failing"))
                    on_click=Callback::new(move |_| { set_status_param.set(Some("failing".into())); set_offset_param.set(None); })
                >"Failing"</Tab>
                <Tab
                    active=Signal::derive(move || status_filter.get().as_deref() == Some("passing"))
                    on_click=Callback::new(move |_| { set_status_param.set(Some("passing".into())); set_offset_param.set(None); })
                >"Passing"</Tab>
                <Tab
                    active=Signal::derive(move || status_filter.get().as_deref() == Some("errors"))
                    on_click=Callback::new(move |_| { set_status_param.set(Some("errors".into())); set_offset_param.set(None); })
                >"Errors"</Tab>
            </Tabs>
        </div>

        <Transition fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading documents..."</p> }>
            {move || docs.get().map(|result| match result {
                Ok(page) => {
                    let total = page.total;
                    let count = page.items.len() as u64;
                    let d = domain.get_value();
                    view! {
                        <Table>
                            <Thead>
                                <tr>
                                    <Th>"Tracking ID"</Th>
                                    <Th>"Title"</Th>
                                    <Th>"Basic"</Th>
                                    <Th>"Extended"</Th>
                                    <Th>"Full"</Th>
                                    <Th>"Signature"</Th>
                                    <Th>"Versions"</Th>
                                </tr>
                            </Thead>
                            <Tbody>
                                {page.items.into_iter().map(|doc| {
                                    let href = format!("/providers/{}/documents/{}", encode_path_segment(&d), doc.tracking_id);
                                    let tid = doc.tracking_id.clone();
                                    let title = doc.title.clone();
                                    let retrieval_err = doc.retrieval_error.clone();
                                    let (sig_variant, sig_label) = if doc.signature_error.is_some() {
                                        (BadgeVariant::Danger, "Invalid")
                                    } else if doc.signature_present {
                                        (BadgeVariant::Success, "Valid")
                                    } else {
                                        (BadgeVariant::Warning, "Missing")
                                    };
                                    view! {
                                        <tr>
                                            <Td>
                                                <a href={href}>{tid}</a>
                                                {retrieval_err.map(|e| view! {
                                                    <span class="ml-2" title={e}>
                                                        <Badge variant=BadgeVariant::Danger>"Error"</Badge>
                                                    </span>
                                                })}
                                            </Td>
                                            <Td class="truncate max-w-xs">{title}</Td>
                                            <Td><DocProfileBadge detail=doc.profiles.basic /></Td>
                                            <Td><DocProfileBadge detail=doc.profiles.extended /></Td>
                                            <Td><DocProfileBadge detail=doc.profiles.full /></Td>
                                            <Td><Badge variant=sig_variant>{sig_label}</Badge></Td>
                                            <Td>{if doc.version_count > 1 {
                                                view! { <Badge variant=BadgeVariant::Neutral>{doc.version_count}</Badge> }.into_any()
                                            } else {
                                                view! { <span /> }.into_any()
                                            }}</Td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </Tbody>
                        </Table>

                        <Pagination
                            offset=offset
                            limit=limit
                            total=total
                            count=count
                            on_change=Callback::new(move |new_offset: u64| {
                                set_offset_param.set(if new_offset == 0 { None } else { Some(new_offset) });
                            })
                        />
                    }.into_any()
                }
                Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
            })}
        </Transition>
    }
}
