use crate::{
    components::{
        badge::{Badge, BadgeVariant},
        note_indicator::NoteIndicator,
        profile_badge::ProfileBadge,
        section_heading::SectionHeading,
        table::{Table, Tbody, Td, Th, Thead},
    },
    format::format_number,
    models::{ProviderSummary, encode_path_segment},
};
use csaf_trove_common::document_checks::DocumentCheckSummary;
use leptos::{ev, html::Input, prelude::*};
use std::cmp::Ordering;
use time::{
    OffsetDateTime,
    format_description::{self, well_known::Rfc3339},
};
use wasm_bindgen::JsCast;
use web_sys::Element;

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

/// Displays the total issue count across all essential checks: 0 = green, >0 = red.
#[component]
fn ProviderChecks(checks: Option<DocumentCheckSummary>) -> impl IntoView {
    let Some(checks) = checks else {
        return view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> };
    };
    let issues = checks.total_issues();
    let variant = if issues > 0 {
        BadgeVariant::Danger
    } else {
        BadgeVariant::Success
    };
    view! { <Badge variant=variant>{format_number(issues)}</Badge> }
}

/// Matches a provider name using a case-insensitive substring, ignoring surrounding spaces.
fn provider_matches(provider: &str, query: &str) -> bool {
    provider
        .to_lowercase()
        .contains(&query.trim().to_lowercase())
}

/// Renders provider summaries with substring filtering and sortable columns.
#[component]
fn ProviderTable(providers: Vec<ProviderSummary>) -> impl IntoView {
    let providers = StoredValue::new(providers);
    let sort = RwSignal::new((ProviderSort::Name, false));
    let query = RwSignal::new(String::new());
    let search = NodeRef::<Input>::new();
    let listener = window_event_listener(ev::keydown, move |event| {
        if event.key() != "/"
            || event.ctrl_key()
            || event.meta_key()
            || event.alt_key()
            || event.is_composing()
            || event.default_prevented()
        {
            return;
        }
        let editing = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .is_some_and(|target| {
                target
                    .closest("input, textarea, select, [contenteditable]:not([contenteditable='false']), [role='textbox']")
                    .ok()
                    .flatten()
                    .is_some()
            });
        if !editing && let Some(input) = search.get_untracked() {
            event.prevent_default();
            let _ = input.focus();
        }
    });
    on_cleanup(move || listener.remove());
    let sorted_providers = move || {
        let mut rows = providers.get_value();
        let query = query.get();
        rows.retain(|provider| provider_matches(&provider.provider, &query));
        let (column, descending) = sort.get();
        rows.sort_by(|left, right| column.compare(left, right, descending));
        rows
    };
    view! {
        <div class="mb-4">
            <label for="provider-search" class="sr-only">"Filter providers"</label>
            <input
                id="provider-search"
                node_ref=search
                type="search"
                placeholder="Filter providers… (/)"
                aria-keyshortcuts="/"
                class="w-full sm:max-w-sm rounded-lg border border-gray-300 bg-white px-3 py-2 text-sm text-gray-800 focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-white"
                prop:value=move || query.get()
                on:input=move |event| query.set(event_target_value(&event))
                on:keydown=move |event| {
                    if event.key() == "Escape" {
                        event.prevent_default();
                        query.set(String::new());
                    }
                }
            />
        </div>
        <Table>
            <Thead>
                <tr>
                    <ProviderSortHeading column=ProviderSort::Name label="Provider" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Documents label="Documents" sort=sort />
                    <Th>"CSAF versions"</Th>
                    <ProviderSortHeading column=ProviderSort::Basic label="Basic" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Extended label="Extended" sort=sort />
                    <ProviderSortHeading column=ProviderSort::Full label="Full" sort=sort />
                    <Th>"Document Issues"</Th>
                    <Th>"Last Validated"</Th>
                </tr>
            </Thead>
            <Tbody>
                {move || {
                    let rows = sorted_providers();
                    if rows.is_empty() {
                        return view! {
                            <tr><td colspan="8" class="px-4 py-8 text-center text-sm text-gray-500 dark:text-gray-400" role="status">"No matching providers."</td></tr>
                        }.into_any();
                    }
                    rows.into_iter().map(|p| {
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
                            <Td>{format_number(p.document_count)}</Td>
                            <Td>
                                <div class="flex flex-wrap gap-2">
                                    {match p.csaf_versions {
                                        Some(mut counts) => {
                                            counts.retain(|version, count| version != "Unknown" && !version.is_empty() && *count > 0);
                                            let total: u64 = counts.values().sum();
                                            let mixed = counts.len() > 1;
                                            if counts.is_empty() {
                                                view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> }.into_any()
                                            } else {
                                            counts.into_iter().map(|(version, count)| {
                                                let percentage = if total == 0 { 0.0 } else { count as f64 / total as f64 * 100.0 };
                                                let variant = BadgeVariant::Info;
                                                view! {
                                                    <span title=format!("{} documents", format_number(count))>
                                                        <Badge variant>
                                                            <span class="whitespace-nowrap">{version}{mixed.then(|| format!(" · {percentage:.0}%"))}</span>
                                                        </Badge>
                                                    </span>
                                                }
                                            }).collect_view().into_any()
                                            }
                                        }
                                        None => view! { <Badge variant=BadgeVariant::Neutral>"n/a"</Badge> }.into_any(),
                                    }}
                                </div>
                            </Td>
                            <Td><ProfileBadge profile=p.profiles.basic /></Td>
                            <Td><ProfileBadge profile=p.profiles.extended /></Td>
                            <Td><ProfileBadge profile=p.profiles.full /></Td>
                            <Td><a href=checks_href><ProviderChecks checks=p.checks /></a></Td>
                            <Td>{format_validated_at(&validated_at)}</Td>
                        </tr>
                    }
                    }).collect_view().into_any()
                }}
            </Tbody>
        </Table>
    }
}

#[cfg(test)]
mod tests {
    use super::provider_matches;

    /// Filters literal substrings, ignores case and surrounding spaces, and clears to all rows.
    #[test]
    fn matches_provider_substrings() {
        assert!(provider_matches("security.Example.COM", " EXAMPLE "));
        assert!(provider_matches("example.com", "ample.co"));
        assert!(provider_matches("example.com", ""));
        assert!(provider_matches("example.com", "   "));
        assert!(!provider_matches("example.com", "other"));
        assert!(!provider_matches("example.com", "example.*"));
    }
}
