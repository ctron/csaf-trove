use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::components::profile_badge::ProfileBadge;
use crate::models::{
    DocumentProfileDetail, PaginatedDocuments, ProviderDetail, encode_path_segment,
};

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
            <h2>{move || format!("Provider: {}", domain())}</h2>
            <Suspense fallback=|| view! { <p class="text-muted text-center py-12">"Loading..."</p> }>
                {move || detail.get().map(|result| match result {
                    Ok(d) => view! { <ProviderDetailView detail=d /> }.into_any(),
                    Err(e) => view! { <p class="text-danger text-center py-12">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ProviderDetailView(detail: ProviderDetail) -> impl IntoView {
    let summary = detail.summary;
    let tests = summary.top_failing_tests;
    let domain = summary.provider.clone();

    view! {
        <div class="flex items-center gap-4 text-sm text-muted mb-6">
            <span>{summary.document_count}" documents"</span>
            <span>"\u{00b7}"</span>
            <span>"Basic "<ProfileBadge profile=summary.profiles.basic /></span>
            <span>"\u{00b7}"</span>
            <span>"Extended "<ProfileBadge profile=summary.profiles.extended /></span>
            <span>"\u{00b7}"</span>
            <span>"Full "<ProfileBadge profile=summary.profiles.full /></span>
        </div>

        <h3>"Top Failing Tests"</h3>
        <table>
            <thead>
                <tr>
                    <th>"Test ID"</th>
                    <th>"Count"</th>
                    <th>"Severity"</th>
                </tr>
            </thead>
            <tbody>
                {tests.into_iter().map(|t| {
                    let test_id = t.test_id.clone();
                    let sev_class = match t.severity.as_str() {
                        "error" => "badge badge-danger",
                        "warning" => "badge badge-warning",
                        "info" => "badge badge-info",
                        _ => "badge",
                    };
                    let sev_label = t.severity.clone();
                    view! {
                        <tr>
                            <td>{test_id}</td>
                            <td>{t.count}</td>
                            <td><span class={sev_class}>{sev_label}</span></td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </tbody>
        </table>

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
        <h3>"Documents"</h3>
        <div class="flex items-center justify-between mb-4">
            <div class="flex gap-2">
                <button
                    class=move || if status_filter.get().is_none() { "btn btn-active" } else { "btn" }
                    on:click=move |_| { set_status_filter.set(None); set_offset.set(0); }
                >"All"</button>
                <button
                    class=move || if status_filter.get().as_deref() == Some("failing") { "btn btn-active" } else { "btn" }
                    on:click=move |_| { set_status_filter.set(Some("failing".into())); set_offset.set(0); }
                >"Failing"</button>
                <button
                    class=move || if status_filter.get().as_deref() == Some("passing") { "btn btn-active" } else { "btn" }
                    on:click=move |_| { set_status_filter.set(Some("passing".into())); set_offset.set(0); }
                >"Passing"</button>
            </div>
        </div>

        <Suspense fallback=|| view! { <p class="text-muted text-center py-12">"Loading documents..."</p> }>
            {move || docs.get().map(|result| match result {
                Ok(page) => {
                    let total = page.total;
                    let page_offset = page.offset;
                    let count = page.items.len() as u64;
                    let d = domain.get_value();
                    view! {
                        <div class="flex items-center justify-between mb-2">
                            <p class="text-sm text-muted">{move || format!("Showing {}\u{2013}{} of {total}", page_offset + 1, page_offset + count)}</p>
                            <div class="flex gap-2">
                                <button
                                    class="btn"
                                    disabled={move || offset.get() == 0}
                                    on:click=move |_| set_offset.set(offset.get().saturating_sub(limit))
                                >"Previous"</button>
                                <button
                                    class="btn"
                                    disabled={move || offset.get() + limit >= total}
                                    on:click=move |_| set_offset.set(offset.get() + limit)
                                >"Next"</button>
                            </div>
                        </div>
                        <table>
                            <thead>
                                <tr>
                                    <th>"Tracking ID"</th>
                                    <th>"Title"</th>
                                    <th>"Basic"</th>
                                    <th>"Extended"</th>
                                    <th>"Full"</th>
                                    <th>"Signature"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {page.items.into_iter().map(|doc| {
                                    let href = format!("/providers/{}/documents/{}", encode_path_segment(&d), doc.tracking_id);
                                    let tid = doc.tracking_id.clone();
                                    let title = doc.title.clone();
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
                                    view! {
                                        <tr>
                                            <td><a href={href}>{tid}</a></td>
                                            <td class="truncate max-w-xs">{title}</td>
                                            <td><DocProfileBadge detail=doc.profiles.basic /></td>
                                            <td><DocProfileBadge detail=doc.profiles.extended /></td>
                                            <td><DocProfileBadge detail=doc.profiles.full /></td>
                                            <td><span class={sig_class}>{sig_label}</span></td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>

                    }.into_any()
                }
                Err(e) => view! { <p class="text-danger text-center py-12">{e}</p> }.into_any(),
            })}
        </Suspense>
    }
}

#[component]
fn DocProfileBadge(detail: Option<DocumentProfileDetail>) -> impl IntoView {
    match detail {
        Some(d) if d.passed => view! { <span class="badge badge-success">"Pass"</span> }.into_any(),
        Some(d) => {
            let mut parts = Vec::new();
            if d.error_count > 0 {
                parts.push(format!("{} errors", d.error_count));
            }
            if d.warning_count > 0 {
                parts.push(format!("{} warnings", d.warning_count));
            }
            if d.info_count > 0 {
                parts.push(format!("{} info", d.info_count));
            }
            let label = if parts.is_empty() {
                "Fail".to_string()
            } else {
                parts.join(", ")
            };
            let class = if d.error_count > 0 {
                "badge badge-danger"
            } else if d.warning_count > 0 {
                "badge badge-warning"
            } else {
                "badge badge-info"
            };
            view! { <span class={class}>{label}</span> }.into_any()
        }
        None => view! { <span class="badge">"-"</span> }.into_any(),
    }
}
