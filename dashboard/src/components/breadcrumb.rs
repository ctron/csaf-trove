use leptos::prelude::*;

/// A full-width breadcrumb navigation bar following the Meraki UI pattern.
#[component]
pub fn Breadcrumb(children: Children) -> impl IntoView {
    view! {
        <div class="bg-gray-200 dark:bg-gray-800 -mx-6 px-6 mb-6">
            <div class="flex items-center py-4 overflow-x-auto whitespace-nowrap">
                {children()}
            </div>
        </div>
    }
}

/// A single item in a breadcrumb trail with a trailing chevron separator.
#[component]
pub fn BreadcrumbItem(#[prop(into)] href: Signal<String>, children: Children) -> impl IntoView {
    view! {
        <a href=move || href.get() class="text-gray-600 dark:text-gray-200 hover:underline">
            {children()}
        </a>
        <span class="mx-5 text-gray-500 dark:text-gray-300 rtl:-scale-x-100">
            <svg xmlns="http://www.w3.org/2000/svg" class="w-5 h-5" viewBox="0 0 20 20" fill="currentColor">
                <path fill-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z" clip-rule="evenodd" />
            </svg>
        </span>
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
