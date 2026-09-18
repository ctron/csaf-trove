use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::components::{
    badge::{Badge, BadgeVariant},
    doc_profile_badge::DocProfileBadge,
    pagination::Pagination,
    profile_badge::ProfileBadge,
    section_heading::{SectionHeading, SubHeading},
    signature_badge::SignatureBadge,
    table::{Table, Tbody, Td, Th, Thead},
    tabs::{Tab, Tabs},
};
use crate::models::{PaginatedDocuments, ProviderDetail, encode_path_segment};

async fn fetch_provider(domain: String) -> Result<ProviderDetail, String> {
    let resp =
        gloo_net::http::Request::get(&format!("/api/providers/{}", encode_path_segment(&domain)))
            .send()
            .await
            .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

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
            <SectionHeading>{move || format!("Provider: {}", domain())}</SectionHeading>
            <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading..."</p> }>
                {move || detail.get().map(|result| match result {
                    Ok(d) => view! { <ProviderDetailView detail=d /> }.into_any(),
                    Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ProviderDetailView(detail: ProviderDetail) -> impl IntoView {
    let history = detail.history;
    let summary = detail.summary;
    let tests = summary.top_failing_tests;
    let signatures = summary.signatures.clone();
    let domain = summary.provider.clone();

    view! {
        <div class="flex items-center gap-4 text-sm text-gray-500 dark:text-gray-400 mb-6">
            <span>{summary.document_count}" documents"</span>
            <span>"\u{00b7}"</span>
            <span>"Basic "<ProfileBadge profile=summary.profiles.basic /></span>
            <span>"\u{00b7}"</span>
            <span>"Extended "<ProfileBadge profile=summary.profiles.extended /></span>
            <span>"\u{00b7}"</span>
            <span>"Full "<ProfileBadge profile=summary.profiles.full /></span>
            <span>"\u{00b7}"</span>
            <span>"Signatures "<SignatureBadge signatures=signatures.clone() /></span>
        </div>

        {signatures.map(|sig| view! {
            <div class="flex items-center gap-4 text-sm mb-4">
                <Badge variant=BadgeVariant::Success>{sig.valid}" valid"</Badge>
                <Badge variant=BadgeVariant::Danger>{sig.invalid}" invalid"</Badge>
                <Badge variant=BadgeVariant::Warning>{sig.missing}" missing"</Badge>
            </div>
        })}

        <SubHeading>"Top Failing Tests"</SubHeading>
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

        {if !history.is_empty() {
            Some(view! {
                <SubHeading>"Sync History"</SubHeading>
                <Table>
                    <Thead>
                        <tr>
                            <Th>"Date"</Th>
                            <Th>"Documents Changed"</Th>
                            <Th>"Message"</Th>
                        </tr>
                    </Thead>
                    <Tbody>
                        {history.into_iter().map(|commit| {
                            let ts = commit.timestamp;
                            let date = format!(
                                "{:04}-{:02}-{:02} {:02}:{:02}",
                                ts.year(),
                                u8::from(ts.month()),
                                ts.day(),
                                ts.hour(),
                                ts.minute(),
                            );
                            view! {
                                <tr>
                                    <Td>{date}</Td>
                                    <Td>{commit.files_changed}</Td>
                                    <Td class="truncate max-w-xs">{commit.message}</Td>
                                </tr>
                            }
                        }).collect::<Vec<_>>()}
                    </Tbody>
                </Table>
            })
        } else {
            None
        }}

        <DocumentsTable domain=domain />
    }
}

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

#[component]
fn DocumentsTable(domain: String) -> impl IntoView {
    let (offset, set_offset) = signal(0u64);
    let (status_filter, set_status_filter) = signal(Option::<String>::None);
    let limit = 50u64;
    let domain = StoredValue::new(domain);

    let docs = LocalResource::new(move || {
        let d = domain.get_value();
        let o = offset.get();
        let s = status_filter.get();
        async move { fetch_documents(d, o, limit, s).await }
    });

    view! {
        <SubHeading>"Documents"</SubHeading>

        <Tabs>
            <Tab
                active=Signal::derive(move || status_filter.get().is_none())
                on_click=Callback::new(move |_| { set_status_filter.set(None); set_offset.set(0); })
            >"All"</Tab>
            <Tab
                active=Signal::derive(move || status_filter.get().as_deref() == Some("failing"))
                on_click=Callback::new(move |_| { set_status_filter.set(Some("failing".into())); set_offset.set(0); })
            >"Failing"</Tab>
            <Tab
                active=Signal::derive(move || status_filter.get().as_deref() == Some("passing"))
                on_click=Callback::new(move |_| { set_status_filter.set(Some("passing".into())); set_offset.set(0); })
            >"Passing"</Tab>
        </Tabs>

        <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading documents..."</p> }>
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
                            on_prev=Callback::new(move |_| set_offset.set(offset.get().saturating_sub(limit)))
                            on_next=Callback::new(move |_| set_offset.set(offset.get() + limit))
                        />
                    }.into_any()
                }
                Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
            })}
        </Suspense>
    }
}
