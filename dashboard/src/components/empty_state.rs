use leptos::prelude::*;

/// A centered empty-state panel following the Meraki UI pattern.
#[component]
pub fn EmptyState(
    title: String,
    message: String,
    action_href: String,
    action_label: String,
) -> impl IntoView {
    view! {
        <div class="flex items-center mt-6 text-center border rounded-lg h-96 dark:border-gray-700">
            <div class="flex flex-col w-full max-w-sm px-4 mx-auto">
                <div class="p-3 mx-auto text-blue-500 bg-blue-100 rounded-full dark:bg-gray-800">
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-6 h-6">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-5.197-5.197m0 0A7.5 7.5 0 105.196 5.196a7.5 7.5 0 0010.607 10.607z" />
                    </svg>
                </div>
                <h1 class="mt-3 text-lg text-gray-800 dark:text-white">{title}</h1>
                <p class="mt-2 text-gray-500 dark:text-gray-400">{message}</p>
                <div class="flex items-center justify-center mt-4">
                    <a href=action_href class="flex items-center justify-center px-5 py-2 text-sm text-gray-700 transition-colors duration-200 bg-white border rounded-lg gap-x-2 dark:hover:bg-gray-800 dark:bg-gray-900 hover:bg-gray-100 dark:text-gray-200 dark:border-gray-700">
                        {action_label}
                    </a>
                </div>
            </div>
        </div>
    }
}
