use leptos::prelude::*;

/// A segmented button group for filtering, following the Meraki UI pattern.
#[component]
pub fn Tabs(children: Children) -> impl IntoView {
    view! {
        <div class="inline-flex overflow-hidden bg-white border border-gray-200 divide-x divide-gray-200 rounded-lg dark:bg-gray-900 rtl:flex-row-reverse dark:border-gray-700 dark:divide-gray-700 mb-4">
            {children()}
        </div>
    }
}

/// A single filter button within a `Tabs` container.
#[component]
pub fn Tab(
    /// Whether this tab is currently active.
    active: Signal<bool>,
    /// Called when the tab is clicked.
    #[prop(into)]
    on_click: Callback<()>,
    children: Children,
) -> impl IntoView {
    let class = move || {
        if active.get() {
            "px-5 py-2 text-xs font-medium text-gray-600 transition-colors duration-200 bg-gray-100 sm:text-sm dark:bg-gray-800 dark:text-gray-300 cursor-pointer"
        } else {
            "px-5 py-2 text-xs font-medium text-gray-600 transition-colors duration-200 sm:text-sm dark:hover:bg-gray-800 dark:text-gray-300 hover:bg-gray-100 cursor-pointer"
        }
    };

    view! {
        <button class=class on:click=move |_| on_click.run(())>
            {children()}
        </button>
    }
}
