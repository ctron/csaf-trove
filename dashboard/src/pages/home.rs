use leptos::prelude::*;
use time::format_description::{self, well_known::Rfc3339};
use time::OffsetDateTime;
use crate::components::{
    profile_badge::ProfileBadge,
    section_heading::SectionHeading,
    signature_badge::SignatureBadge,
    table::{Table, Tbody, Td, Th, Thead},
};
use crate::models::{ProviderSummary, encode_path_segment};

fn format_validated_at(s: &str) -> String {
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .and_then(|dt| {
            let fmt = format_description::parse_borrowed::<2>("[year]-[month]-[day] [hour]:[minute] UTC").ok()?;
            dt.format(&fmt).ok()
        })
        .unwrap_or_else(|| s.to_string())
}

async fn fetch_providers() -> Result<Vec<ProviderSummary>, String> {
    let resp = gloo_net::http::Request::get("/api/providers")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[component]
pub fn HomePage() -> impl IntoView {
    let providers = LocalResource::new(fetch_providers);

    view! {
        <div>
            <SectionHeading>"CSAF Providers"</SectionHeading>
            <Suspense fallback=|| view! { <p class="text-gray-500 dark:text-gray-400 text-center py-12">"Loading providers..."</p> }>
                {move || providers.get().map(|result| match result {
                    Ok(list) => view! { <ProviderTable providers=list /> }.into_any(),
                    Err(e) => view! { <p class="text-red-500 dark:text-red-400 text-center py-12">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn ProviderTable(providers: Vec<ProviderSummary>) -> impl IntoView {
    view! {
        <Table>
            <Thead>
                <tr>
                    <Th>"Provider"</Th>
                    <Th>"Documents"</Th>
                    <Th>"Basic"</Th>
                    <Th>"Extended"</Th>
                    <Th>"Full"</Th>
                    <Th>"Signatures"</Th>
                    <Th>"Errors"</Th>
                    <Th>"Last Validated"</Th>
                </tr>
            </Thead>
            <Tbody>
                {providers.into_iter().map(|p| {
                    let domain = p.provider.clone();
                    let href = format!("/providers/{}", encode_path_segment(&domain));
                    let display_domain = domain.clone();
                    let validated_at = p.validated_at.clone();
                    view! {
                        <tr>
                            <Td><a href={href}>{display_domain}</a></Td>
                            <Td>{p.document_count}</Td>
                            <Td><ProfileBadge profile=p.profiles.basic /></Td>
                            <Td><ProfileBadge profile=p.profiles.extended /></Td>
                            <Td><ProfileBadge profile=p.profiles.full /></Td>
                            <Td><SignatureBadge signatures=p.signatures /></Td>
                            <Td>{if p.retrieval_errors > 0 { p.retrieval_errors.to_string() } else { "-".to_string() }}</Td>
                            <Td>{format_validated_at(&validated_at)}</Td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
}
