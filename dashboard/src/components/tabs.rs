use leptos::prelude::*;

/// A container for line-style tab buttons following the Meraki UI pattern.
#[component]
pub fn Tabs(children: Children) -> impl IntoView {
    view! {
        <div class="flex overflow-x-auto border-b border-gray-200 whitespace-nowrap dark:border-gray-700 mb-4">
            {children()}
        </div>
    }
}

/// A single tab button within a `Tabs` container.
#[component]
pub fn Tab(
    active: Signal<bool>,
    #[prop(into)] on_click: Callback<()>,
    children: Children,
) -> impl IntoView {
    let class = move || {
        if active.get() {
            "inline-flex items-center h-10 px-4 -mb-px text-sm text-center text-blue-600 bg-transparent border-b-2 border-blue-500 dark:border-blue-400 dark:text-blue-300 whitespace-nowrap focus:outline-none cursor-pointer"
        } else {
            "inline-flex items-center h-10 px-4 -mb-px text-sm text-center text-gray-700 bg-transparent border-b-2 border-transparent dark:text-white whitespace-nowrap cursor-pointer focus:outline-none hover:border-gray-400"
        }
    };

    view! {
        <button class=class on:click=move |_| on_click.run(())>
            {children()}
        </button>
    }
}
