use leptos::prelude::*;
use leptos_router::{
    NavigateOptions,
    hooks::{query_signal_with_options, use_params_map},
};

use crate::components::{
    badge::{Badge, BadgeVariant},
    breadcrumb::{Breadcrumb, BreadcrumbCurrent, BreadcrumbItem},
    pagination::Pagination,
    section_heading::SectionHeading,
    table::{Table, Tbody, Td, Th, Thead},
};
use crate::models::encode_path_segment;

/// Fetches paginated sync history from the API.
async fn fetch_sync_history(
    domain: String,
    offset: u64,
    limit: u64,
) -> Result<csaf_trove_common::Paginated<csaf_trove_common::CommitInfo>, String> {
    let url = format!(
        "/api/providers/{}/history?offset={offset}&limit={limit}",
        encode_path_segment(&domain)
    );
    let resp = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(format!("Failed to load sync history ({})", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Displays paginated sync run history for a single provider.
#[component]
pub fn SyncDetailPage() -> impl IntoView {
    let params = use_params_map();
    let domain = move || params.read().get("domain").unwrap_or_default();

    let (offset_param, set_offset_param) = query_signal_with_options::<u64>(
        "offset",
        NavigateOptions {
            scroll: false,
            ..Default::default()
        },
    );
    let offset = Signal::derive(move || offset_param.get().unwrap_or(0));
    let limit = 10u64;

    let history = LocalResource::new(move || {
        let d = domain();
        let o = offset.get();
        async move { fetch_sync_history(d, o, limit).await }
    });

    view! {
        <div>
            <Breadcrumb>
                <BreadcrumbItem href=Signal::derive(|| "/sync".to_string())>"Sync Status"</BreadcrumbItem>
                <BreadcrumbCurrent>{move || domain()}</BreadcrumbCurrent>
            </Breadcrumb>

            <SectionHeading>{move || format!("Sync History: {}", domain())}</SectionHeading>

            <Transition fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading..."</p> }>
                {move || history.get().map(|result| match result {
                    Ok(page) => {
                        let total = page.total;
                        let count = page.items.len() as u64;

                        if page.items.is_empty() && page.offset == 0 {
                            return view! {
                                <p class="text-gray-500 dark:text-gray-400 text-center py-12">"No sync history available."</p>
                            }.into_any();
                        }

                        view! {
                            <Table>
                                <Thead>
                                    <tr>
                                        <Th>"#"</Th>
                                        <Th>"Date"</Th>
                                        <Th>"Documents Changed"</Th>
                                    </tr>
                                </Thead>
                                <Tbody>
                                    {page.items.into_iter().map(|entry| {
                                        let ts = entry.timestamp;
                                        let date = format!(
                                            "{:04}-{:02}-{:02} {:02}:{:02}",
                                            ts.year(),
                                            u8::from(ts.month()),
                                            ts.day(),
                                            ts.hour(),
                                            ts.minute(),
                                        );
                                        let variant = if entry.files_changed > 0 {
                                            BadgeVariant::Info
                                        } else {
                                            BadgeVariant::Neutral
                                        };
                                        view! {
                                            <tr>
                                                <Td>{entry.id}</Td>
                                                <Td class="whitespace-nowrap">{date}</Td>
                                                <Td><Badge variant=variant>{entry.files_changed}</Badge></Td>
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
                    Err(e) => view! {
                        <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p>
                    }.into_any(),
                })}
            </Transition>
        </div>
    }
}
