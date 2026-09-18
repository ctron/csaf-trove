use leptos::prelude::*;

/// Computes the page numbers to display with windowing.
///
/// For 7 or fewer pages, returns all numbers. For more, returns a window
/// around the current page with `0` as a sentinel for ellipsis gaps.
fn page_window(current: u64, total: u64) -> Vec<u64> {
    if total <= 7 {
        return (1..=total).collect();
    }

    let mut pages = Vec::with_capacity(9);
    pages.push(1);

    if current > 3 {
        pages.push(0);
    }

    let start = current.saturating_sub(1).max(2);
    let end = (current + 1).min(total - 1);
    for p in start..=end {
        pages.push(p);
    }

    if current < total - 2 {
        pages.push(0);
    }

    if *pages.last().unwrap_or(&0) != total {
        pages.push(total);
    }

    pages
}

/// Pagination controls following the Meraki UI pattern.
#[component]
pub fn Pagination(
    /// Current zero-based offset into the result set.
    #[prop(into)]
    offset: Signal<u64>,
    /// Number of items per page.
    limit: u64,
    /// Total number of items.
    total: u64,
    /// Number of items on the current page.
    count: u64,
    /// Called with the new offset when the user navigates.
    #[prop(into)]
    on_change: Callback<u64>,
) -> impl IntoView {
    let total_pages = if total == 0 { 1 } else { total.div_ceil(limit) };

    let current_page = move || offset.get() / limit + 1;
    let prev_disabled = move || offset.get() == 0;
    let next_disabled = move || offset.get() + limit >= total;

    let on_prev = move |_| {
        on_change.run(offset.get().saturating_sub(limit));
    };
    let on_next = move |_| {
        on_change.run(offset.get() + limit);
    };

    view! {
        <div class="flex flex-col items-center py-5 space-y-6 sm:flex-row sm:justify-between sm:space-y-0 mt-6">
            <div class="-mx-2 flex items-center">
                <button
                    class=move || if prev_disabled() {
                        "inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-500 rounded-lg cursor-not-allowed dark:text-gray-600"
                    } else {
                        "inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-700 transition-colors duration-300 transform rounded-lg hover:bg-gray-100 dark:text-white dark:hover:bg-gray-700 cursor-pointer"
                    }
                    disabled=prev_disabled
                    on:click=on_prev
                >
                    "previous"
                </button>

                {move || {
                    let cur = current_page();
                    page_window(cur, total_pages).into_iter().map(|p| {
                        if p == 0 {
                            view! { <span class="inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-500 dark:text-gray-400">"…"</span> }.into_any()
                        } else if p == cur {
                            view! {
                                <span class="inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-700 transition-colors duration-300 transform bg-gray-100 rounded-lg dark:text-white dark:bg-gray-700">
                                    {p}
                                </span>
                            }.into_any()
                        } else {
                            let new_offset = (p - 1) * limit;
                            view! {
                                <button
                                    class="inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-700 transition-colors duration-300 transform rounded-lg hover:bg-gray-100 dark:text-white dark:hover:bg-gray-700 cursor-pointer"
                                    on:click=move |_| on_change.run(new_offset)
                                >{p}</button>
                            }.into_any()
                        }
                    }).collect::<Vec<_>>()
                }}

                <button
                    class=move || if next_disabled() {
                        "inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-500 rounded-lg cursor-not-allowed dark:text-gray-600"
                    } else {
                        "inline-flex items-center justify-center px-4 py-1 mx-2 text-gray-700 transition-colors duration-300 transform rounded-lg hover:bg-gray-100 dark:text-white dark:hover:bg-gray-700 cursor-pointer"
                    }
                    disabled=next_disabled
                    on:click=on_next
                >
                    "next"
                </button>
            </div>

            <div class="text-gray-500 dark:text-gray-400">
                <span class="font-medium text-gray-700 dark:text-gray-100">
                    {move || {
                        let o = offset.get();
                        format!("{} - {}", o + 1, o + count)
                    }}
                </span>
                {format!(" of {total} records")}
            </div>
        </div>
    }
}
