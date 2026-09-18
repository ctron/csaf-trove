use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use crate::{
    components::theme_toggle::ThemeToggle,
    pages::{
        document::DocumentPage, home::HomePage, provider::ProviderPage,
        sync_detail::SyncDetailPage, sync_status::SyncStatusPage,
    },
};

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <nav class="bg-white shadow dark:bg-gray-800">
                <div class="container flex items-center justify-between px-6 py-4 mx-auto">
                    <a href="/" class="text-xl font-bold text-gray-800 dark:text-white no-underline hover:no-underline">"csaf-trove"</a>
                    <div class="flex items-center gap-x-4">
                        <a href="/" class="text-gray-600 border-b-2 border-transparent transition-colors duration-300 hover:text-gray-800 hover:border-blue-500 dark:text-gray-300 dark:hover:text-gray-200 no-underline hover:no-underline">"Providers"</a>
                        <a href="/sync" class="text-gray-600 border-b-2 border-transparent transition-colors duration-300 hover:text-gray-800 hover:border-blue-500 dark:text-gray-300 dark:hover:text-gray-200 no-underline hover:no-underline">"Sync Status"</a>
                        <ThemeToggle />
                    </div>
                </div>
            </nav>
            <main class="py-8">
                <div class="container mx-auto px-6">
                    <Routes fallback=|| view! { <p class="text-gray-500 dark:text-gray-400">"Page not found."</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/providers/:domain") view=ProviderPage />
                        <Route path=path!("/providers/:domain/documents/:tracking_id") view=DocumentPage />
                        <Route path=path!("/sync") view=SyncStatusPage />
                        <Route path=path!("/sync/:domain") view=SyncDetailPage />
                    </Routes>
                </div>
            </main>
            <footer class="border-t border-gray-200 dark:border-gray-700 py-4 mt-auto">
                <div class="container mx-auto px-6 text-xs text-gray-500 dark:text-gray-400">
                    "csaf-trove v" {env!("CARGO_PKG_VERSION")}
                </div>
            </footer>
        </Router>
    }
}
