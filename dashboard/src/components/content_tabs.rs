use leptos::prelude::*;

/// A line-style tab bar for switching between content sections.
#[component]
pub fn ContentTabs(children: Children) -> impl IntoView {
    view! {
        <div class="flex overflow-x-auto overflow-y-hidden border-b border-gray-200 whitespace-nowrap dark:border-gray-700 mb-6">
            {children()}
        </div>
    }
}

/// A single tab in a line-style tab bar.
#[component]
pub fn ContentTab(
    /// Whether this tab is currently active.
    active: Signal<bool>,
    /// Called when the tab is clicked.
    #[prop(into)]
    on_click: Callback<()>,
    children: Children,
) -> impl IntoView {
    let class = move || {
        if active.get() {
            "inline-flex items-center h-10 px-4 -mb-px text-sm text-center text-blue-600 bg-transparent border-b-2 border-blue-500 sm:text-base dark:border-blue-400 dark:text-blue-300 whitespace-nowrap focus:outline-none cursor-pointer"
        } else {
            "inline-flex items-center h-10 px-4 -mb-px text-sm text-center text-gray-700 bg-transparent border-b-2 border-transparent sm:text-base dark:text-white whitespace-nowrap focus:outline-none hover:border-gray-400 cursor-pointer"
        }
    };

    view! {
        <button class=class on:click=move |_| on_click.run(())>
            {children()}
        </button>
    }
}
