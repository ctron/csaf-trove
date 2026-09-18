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
    pagination::Pagination,
    progress_bar::{ProgressBar, color_for_pass_rate},
    table::{Table, Tbody, Td, Th, Thead},
    tabs::{Tab, Tabs},
};
use crate::models::{
    PaginatedDocuments, ProfileSummary, ProviderDetail, SignatureSummary, encode_path_segment,
};

/// Fetches provider detail from the API.
async fn fetch_provider(domain: String) -> Result<ProviderDetail, String> {
    let resp =
        gloo_net::http::Request::get(&format!("/api/providers/{}", encode_path_segment(&domain)))
            .send()
            .await
            .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
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
                    Ok(d) => view! { <ProviderDetailView detail=d /> }.into_any(),
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

/// Renders the optional signature summary section inside the overview card.
fn signature_section(sig: SignatureSummary) -> impl IntoView {
    view! {
        <div class="border-t border-gray-200 dark:border-gray-700 pt-4 mt-4">
            <p class="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">"Signatures"</p>
            <div class="flex items-center gap-3">
                <Badge variant=BadgeVariant::Success>{sig.valid}" valid"</Badge>
                <Badge variant=BadgeVariant::Danger>{sig.invalid}" invalid"</Badge>
                <Badge variant=BadgeVariant::Warning>{sig.missing}" missing"</Badge>
            </div>
        </div>
    }
}

/// Main detail view with grouped cards, info line, and tabbed content.
#[component]
fn ProviderDetailView(detail: ProviderDetail) -> impl IntoView {
    let summary = detail.summary;
    let tests = summary.top_failing_tests;
    let signatures = summary.signatures.clone();
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
                    <p>
                        <a href={sync_href} class="text-blue-600 dark:text-blue-400 hover:underline">"Sync History \u{2192}"</a>
                    </p>
                </div>

                {signatures.map(signature_section)}
            </div>

            // Validation card
            <div class="bg-white rounded-lg shadow-md dark:bg-gray-800 p-6">
                <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4">"Validation"</h2>
                {profile_row("Basic", summary.profiles.basic.clone())}
                {profile_row("Extended", summary.profiles.extended.clone())}
                {profile_row("Full", summary.profiles.full)}
            </div>
        </div>

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
                                    let (sig_variant, sig_label) = if doc.signature_error.is_some() {
                                        (BadgeVariant::Danger, "Invalid")
                                    } else if doc.signature_present {
                                        (BadgeVariant::Success, "Valid")
                                    } else {
                                        (BadgeVariant::Warning, "Missing")
                                    };
                                    view! {
                                        <tr>
                                            <Td><a href={href}>{tid}</a></Td>
                                            <Td class="truncate max-w-xs">{title}</Td>
                                            <Td><DocProfileBadge detail=doc.profiles.basic /></Td>
                                            <Td><DocProfileBadge detail=doc.profiles.extended /></Td>
                                            <Td><DocProfileBadge detail=doc.profiles.full /></Td>
                                            <Td><Badge variant=sig_variant>{sig_label}</Badge></Td>
                                            <Td>{match doc.version_count {
                                                Some(n) if n > 1 => view! { <Badge variant=BadgeVariant::Neutral>{n}</Badge> }.into_any(),
                                                _ => view! { <span /> }.into_any(),
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
