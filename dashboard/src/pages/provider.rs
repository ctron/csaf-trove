use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::components::profile_badge::ProfileBadge;
use crate::models::{DocumentProfileDetail, PaginatedDocuments, ProviderDetail};

async fn fetch_provider(domain: String) -> Result<ProviderDetail, String> {
    let resp = gloo_net::http::Request::get(&format!("/api/providers/{domain}"))
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
            <Suspense fallback=|| view! { <p class="loading">"Loading..."</p> }>
                {move || detail.get().map(|result| match result {
                    Ok(d) => view! { <ProviderDetailView detail=d /> }.into_any(),
                    Err(e) => view! { <p class="error">{e}</p> }.into_any(),
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
        <div class="cards">
            <div class="card">
                <h3>"Documents"</h3>
                <div class="value">{summary.document_count}</div>
            </div>
            <div class="card">
                <h3>"Basic"</h3>
                <div class="value"><ProfileBadge profile=summary.profiles.basic /></div>
            </div>
            <div class="card">
                <h3>"Extended"</h3>
                <div class="value"><ProfileBadge profile=summary.profiles.extended /></div>
            </div>
            <div class="card">
                <h3>"Full"</h3>
                <div class="value"><ProfileBadge profile=summary.profiles.full /></div>
            </div>
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
                    let severity = t.severity.clone();
                    view! {
                        <tr>
                            <td>{test_id}</td>
                            <td>{t.count}</td>
                            <td>{severity}</td>
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
    let mut url = format!("/api/providers/{domain}/document?offset={offset}&limit={limit}");
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
        <div class="filter-bar">
            <button
                class=move || if status_filter.get().is_none() { "active" } else { "" }
                on:click=move |_| { set_status_filter.set(None); set_offset.set(0); }
            >"All"</button>
            <button
                class=move || if status_filter.get().as_deref() == Some("failing") { "active" } else { "" }
                on:click=move |_| { set_status_filter.set(Some("failing".into())); set_offset.set(0); }
            >"Failing"</button>
            <button
                class=move || if status_filter.get().as_deref() == Some("passing") { "active" } else { "" }
                on:click=move |_| { set_status_filter.set(Some("passing".into())); set_offset.set(0); }
            >"Passing"</button>
        </div>

        <Suspense fallback=|| view! { <p class="loading">"Loading documents..."</p> }>
            {move || docs.get().map(|result| match result {
                Ok(page) => {
                    let total = page.total;
                    let page_offset = page.offset;
                    let count = page.items.len() as u64;
                    let d = domain.get_value();
                    view! {
                        <p>{move || format!("Showing {}\u{2013}{} of {total}", page_offset + 1, page_offset + count)}</p>
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
                                    let href = format!("/providers/{}/documents/{}", d, doc.tracking_id);
                                    let tid = doc.tracking_id.clone();
                                    let title = doc.title.clone();
                                    let sig_class = if doc.signature_error.is_some() {
                                        "badge badge-red"
                                    } else if doc.signature_present {
                                        "badge badge-green"
                                    } else {
                                        "badge badge-yellow"
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
                                            <td class="truncate">{title}</td>
                                            <td><DocProfileBadge detail=doc.profiles.basic /></td>
                                            <td><DocProfileBadge detail=doc.profiles.extended /></td>
                                            <td><DocProfileBadge detail=doc.profiles.full /></td>
                                            <td><span class={sig_class}>{sig_label}</span></td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>

                        <div class="pagination">
                            <button
                                disabled=move || offset.get() == 0
                                on:click=move |_| set_offset.set(offset.get().saturating_sub(limit))
                            >"Previous"</button>
                            <button
                                disabled=move || offset.get() + limit >= total
                                on:click=move |_| set_offset.set(offset.get() + limit)
                            >"Next"</button>
                        </div>
                    }.into_any()
                }
                Err(e) => view! { <p class="error">{e}</p> }.into_any(),
            })}
        </Suspense>
    }
}

#[component]
fn DocProfileBadge(detail: Option<DocumentProfileDetail>) -> impl IntoView {
    match detail {
        Some(d) if d.passed => view! { <span class="badge badge-green">"Pass"</span> }.into_any(),
        Some(d) => {
            let label = format!("{} errors", d.error_count);
            view! { <span class="badge badge-red">{label}</span> }.into_any()
        }
        None => view! { <span class="badge">"-"</span> }.into_any(),
    }
}
