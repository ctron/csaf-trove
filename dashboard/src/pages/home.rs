use crate::components::{
    badge::{Badge, BadgeVariant},
    note_indicator::NoteIndicator,
    profile_badge::ProfileBadge,
    section_heading::SectionHeading,
    table::{Table, Tbody, Td, Th, Thead},
};
use crate::models::{ProviderSummary, encode_path_segment};
use csaf_trove_common::document_checks::DocumentCheckSummary;
use leptos::prelude::*;
use std::cmp::Ordering;
use time::{
    OffsetDateTime,
    format_description::{self, well_known::Rfc3339},
};

fn format_validated_at(s: &str) -> String {
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .and_then(|dt| {
            let fmt =
                format_description::parse_borrowed::<2>("[year]-[month]-[day] [hour]:[minute] UTC")
                    .ok()?;
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

/// Provider summary columns that support sorting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderSort {
    /// Provider domain name.
    Name,
    /// Number of documents.
    Documents,
    /// Basic profile pass rate.
    Basic,
    /// Extended profile pass rate.
    Extended,
    /// Full profile pass rate.
    Full,
}

impl ProviderSort {
    /// Compares providers, keeping missing profile results last in either direction.
    fn compare(
        self,
        left: &ProviderSummary,
        right: &ProviderSummary,
        descending: bool,
    ) -> Ordering {
        let order = match self {
            Self::Name => left.provider.cmp(&right.provider),
            Self::Documents => left.document_count.cmp(&right.document_count),
            profile => {
                let rate = |provider: &ProviderSummary| {
                    match profile {
                        Self::Basic => &provider.profiles.basic,
                        Self::Extended => &provider.profiles.extended,
                        _ => &provider.profiles.full,
                    }
                    .as_ref()
                    .map(|summary| summary.pass_rate)
                };
                match (rate(left), rate(right)) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    (Some(_), None) => return Ordering::Less,
                    (None, Some(_)) => return Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
            }
        };
        let order = if descending { order.reverse() } else { order };
        order.then_with(|| left.provider.cmp(&right.provider))
    }
}

/// A keyboard-accessible sort heading with a visible direction indicator.
#[component]
fn ProviderSortHeading(
    /// Column controlled by this heading.
    column: ProviderSort,
    /// Visible column label.
    label: &'static str,
    /// Active sort column and whether it is descending.
    sort: RwSignal<(ProviderSort, bool)>,
) -> impl IntoView {
    view! {
        <th
            scope="col"
            class="py-3.5 px-4 text-sm font-normal text-left text-gray-500 dark:text-gray-400"
            aria-sort=move || match sort.get() {
                (active, _) if active != column => "none",
                (_, true) => "descending",
                _ => "ascending",
            }
        >
            <button
                type="button"
                class="inline-flex items-center gap-1 cursor-pointer hover:text-gray-800 dark:hover:text-gray-200"
                on:click=move |_| sort.update(|(active, descending)| {
                    *descending = if *active == column { !*descending } else { column != ProviderSort::Name };
                    *active = column;
                })
            >
                {label}
                <span aria-hidden="true">{move || match sort.get() {
                    (active, _) if active != column => "↕",
                    (_, true) => "↓",
                    _ => "↑",
                }}</span>
            </button>
        </th>
    }
}

/// Displays the combined essential checks, highlighting any known problem in red.
#[component]
fn ProviderChecks(
    /// Aggregate results, absent for providers without recorded checks.
    checks: Option<DocumentCheckSummary>,
) -> impl IntoView {
    let (label, variant) = checks
        .map(|checks| {
            let stages = [
                checks.retrieval,
                checks.parsing,
                checks.signature,
                checks.digest,
            ];
            if stages
                .iter()
                .any(|c| c.failed > 0 || c.warning > 0 || c.missing > 0)
            {
                ("Issues", BadgeVariant::Danger)
            } else if stages.iter().any(|c| c.passed > 0) {
                ("Passed", BadgeVariant::Success)
            } else {
                ("-", BadgeVariant::Neutral)
            }
        })
        .unwrap_or(("-", BadgeVariant::Neutral));
    view! { <Badge variant=variant>{label}</Badge> }
}

/// Renders provider summaries with sortable name, document count, and profile columns.
#[component]
fn ProviderTable(providers: Vec<ProviderSummary>) -> impl IntoView {
    let providers = StoredValue::new(providers);
    let sort = RwSignal::new((ProviderSort::Name, false));
    let sorted_providers = move || {
        let mut rows = providers.get_value();
        let (column, descending) = sort.get();
        rows.sort_by(|left, right| column.compare(left, right, descending));
        rows
    };
    view! {
        <Table>
            <Thead>
                <tr>
                    <ProviderSortHeading column=ProviderSort::Name label="Provider" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Documents label="Documents" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Basic label="Basic" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Extended label="Extended" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Full label="Full" sort=sort />
                    <Th>"Document Issues"</Th>
                    <Th>"Last Validated"</Th>
                </tr>
            </Thead>
            <Tbody>
                {move || sorted_providers().into_iter().map(|p| {
                    let domain = p.provider.clone();
                    let href = format!("/providers/{}", encode_path_segment(&domain));
                    let checks_href = href.clone();
                    let display_domain = domain.clone();
                    let validated_at = p.validated_at.clone();
                    let note = p.note.clone();
                    view! {
                        <tr>
                            <Td>
                                <span class="inline-flex items-center gap-1.5">
                                    <a href={href.clone()}>{display_domain}</a>
                                    {note.map(|text| view! { <NoteIndicator text=text /> })}
                                </span>
                            </Td>
                            <Td>{p.document_count}</Td>
                            <Td><ProfileBadge profile=p.profiles.basic /></Td>
                            <Td><ProfileBadge profile=p.profiles.extended /></Td>
                            <Td><ProfileBadge profile=p.profiles.full /></Td>
                            <Td><a href=checks_href><ProviderChecks checks=p.checks /></a></Td>
                            <Td>{format_validated_at(&validated_at)}</Td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </Tbody>
        </Table>
    }
}
