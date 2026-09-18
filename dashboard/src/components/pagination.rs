use leptos::prelude::*;

/// Pagination controls with "Showing X-Y of Z" text and Previous/Next buttons.
#[component]
pub fn Pagination(
    offset: ReadSignal<u64>,
    limit: u64,
    total: u64,
    count: u64,
    #[prop(into)] on_prev: Callback<()>,
    #[prop(into)] on_next: Callback<()>,
) -> impl IntoView {
    let page_offset = offset.get_untracked();

    let prev_disabled = move || offset.get() == 0;
    let next_disabled = move || offset.get() + limit >= total;

    let btn_base = "flex items-center px-5 py-2 text-sm capitalize transition-colors duration-200 border rounded-md gap-x-2 focus:outline-none";
    let btn_enabled = "text-gray-700 bg-white hover:bg-gray-100 dark:bg-gray-900 dark:text-gray-200 dark:border-gray-700 dark:hover:bg-gray-800 border-gray-200";
    let btn_disabled = "text-gray-400 bg-gray-100 cursor-not-allowed dark:bg-gray-800 dark:text-gray-600 dark:border-gray-700 border-gray-200";

    view! {
        <div class="flex items-center justify-between mt-6">
            <p class="text-sm text-gray-500 dark:text-gray-400">
                {move || format!("Showing {}\u{2013}{} of {total}", page_offset + 1, page_offset + count)}
            </p>
            <div class="flex gap-x-4">
                <button
                    class=move || format!("{btn_base} {}", if prev_disabled() { btn_disabled } else { btn_enabled })
                    disabled=prev_disabled
                    on:click=move |_| on_prev.run(())
                >
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-5 h-5 rtl:-scale-x-100">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M6.75 15.75L3 12m0 0l3.75-3.75M3 12h18" />
                    </svg>
                    "Previous"
                </button>
                <button
                    class=move || format!("{btn_base} {}", if next_disabled() { btn_disabled } else { btn_enabled })
                    disabled=next_disabled
                    on:click=move |_| on_next.run(())
                >
                    "Next"
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-5 h-5 rtl:-scale-x-100">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M17.25 8.25L21 12m0 0l-3.75 3.75M21 12H3" />
                    </svg>
                </button>
            </div>
        </div>
    }
}
