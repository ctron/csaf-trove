use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use crate::{
    components::theme_toggle::ThemeToggle,
    pages::{
        document::DocumentPage, home::HomePage, provider::ProviderPage, sync_status::SyncStatusPage,
    },
};

#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <header class="border-b border-border py-4">
                <div class="max-w-[1200px] mx-auto px-6">
                    <h1 class="text-xl font-semibold">"csaf-trove"</h1>
                    <nav class="flex gap-4 mt-2 items-center">
                        <a href="/" class="text-muted text-sm px-2 py-1 rounded-md no-underline hover:text-foreground hover:bg-surface hover:no-underline">"Providers"</a>
                        <a href="/sync" class="text-muted text-sm px-2 py-1 rounded-md no-underline hover:text-foreground hover:bg-surface hover:no-underline">"Sync Status"</a>
                        <ThemeToggle />
                    </nav>
                </div>
            </header>
            <main class="py-6">
                <div class="max-w-[1200px] mx-auto px-6">
                    <Routes fallback=|| view! { <p>"Page not found."</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/providers/:domain") view=ProviderPage />
                        <Route path=path!("/providers/:domain/documents/:tracking_id") view=DocumentPage />
                        <Route path=path!("/sync") view=SyncStatusPage />
                    </Routes>
                </div>
            </main>
            <footer class="border-t border-border py-4 mt-auto">
                <div class="max-w-[1200px] mx-auto px-6 text-xs text-muted">
                    "csaf-trove v" {env!("CARGO_PKG_VERSION")}
                </div>
            </footer>
        </Router>
    }
}
