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

/// Pagination controls following the Meraki UI table pattern.
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
        <div class="flex flex-col items-center mt-6 space-y-4 sm:flex-row sm:justify-between sm:space-y-0">
            <div class="flex items-center gap-x-2">
                <button
                    class=move || if prev_disabled() {
                        "flex items-center px-5 py-2 text-sm font-normal text-gray-700 capitalize transition-colors duration-200 bg-white border border-gray-200 rounded-md gap-x-2 dark:bg-gray-900 dark:text-gray-200 dark:border-gray-700 opacity-50 cursor-not-allowed"
                    } else {
                        "flex items-center px-5 py-2 text-sm font-normal text-gray-700 capitalize transition-colors duration-200 bg-white border border-gray-200 rounded-md gap-x-2 hover:bg-gray-100 dark:bg-gray-900 dark:text-gray-200 dark:border-gray-700 dark:hover:bg-gray-800 cursor-pointer"
                    }
                    disabled=prev_disabled
                    on:click=on_prev
                >
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-5 h-5 rtl:-scale-x-100">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M6.75 15.75L3 12m0 0l3.75-3.75M3 12h18" />
                    </svg>
                    <span>"previous"</span>
                </button>

                <div class="items-center hidden md:flex gap-x-3">
                    {move || {
                        let cur = current_page();
                        page_window(cur, total_pages).into_iter().map(|p| {
                            if p == 0 {
                                view! { <span class="px-2 py-1 text-sm text-gray-500 dark:text-gray-400">"…"</span> }.into_any()
                            } else if p == cur {
                                view! {
                                    <span class="px-2 py-1 text-sm text-blue-500 rounded-md dark:bg-gray-800 bg-blue-100/60">
                                        {p}
                                    </span>
                                }.into_any()
                            } else {
                                let new_offset = (p - 1) * limit;
                                view! {
                                    <button
                                        class="px-2 py-1 text-sm text-gray-500 rounded-md dark:hover:bg-gray-800 dark:text-gray-300 hover:bg-gray-100 cursor-pointer"
                                        on:click=move |_| on_change.run(new_offset)
                                    >{p}</button>
                                }.into_any()
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>

                <button
                    class=move || if next_disabled() {
                        "flex items-center px-5 py-2 text-sm font-normal text-gray-700 capitalize transition-colors duration-200 bg-white border border-gray-200 rounded-md gap-x-2 dark:bg-gray-900 dark:text-gray-200 dark:border-gray-700 opacity-50 cursor-not-allowed"
                    } else {
                        "flex items-center px-5 py-2 text-sm font-normal text-gray-700 capitalize transition-colors duration-200 bg-white border border-gray-200 rounded-md gap-x-2 hover:bg-gray-100 dark:bg-gray-900 dark:text-gray-200 dark:border-gray-700 dark:hover:bg-gray-800 cursor-pointer"
                    }
                    disabled=next_disabled
                    on:click=on_next
                >
                    <span>"Next"</span>
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-5 h-5 rtl:-scale-x-100">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M17.25 8.25L21 12m0 0l-3.75 3.75M21 12H3" />
                    </svg>
                </button>
            </div>

            <div class="text-sm text-gray-500 dark:text-gray-400">
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
