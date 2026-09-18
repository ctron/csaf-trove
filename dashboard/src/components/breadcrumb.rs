use leptos::prelude::*;

/// A breadcrumb navigation container following the Meraki UI pattern.
#[component]
pub fn Breadcrumb(children: Children) -> impl IntoView {
    view! {
        <div class="flex items-center py-4 overflow-x-auto whitespace-nowrap">
            {children()}
        </div>
    }
}

/// A single item in a breadcrumb trail.
#[component]
pub fn BreadcrumbItem(#[prop(into)] href: Signal<String>, children: Children) -> impl IntoView {
    view! {
        <a href=move || href.get() class="text-gray-600 dark:text-gray-200 hover:underline">
            {children()}
        </a>
        <span class="mx-3 text-gray-500 dark:text-gray-300">"/"</span>
    }
}

/// The final (current) item in a breadcrumb trail, rendered as plain text.
#[component]
pub fn BreadcrumbCurrent(children: Children) -> impl IntoView {
    view! {
        <span class="text-blue-600 dark:text-blue-400">
            {children()}
        </span>
    }
}
