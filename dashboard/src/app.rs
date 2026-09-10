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
            <header>
                <div class="container">
                    <h1>"csaf-trove"</h1>
                    <nav>
                        <a href="/">"Providers"</a>
                        <a href="/sync">"Sync Status"</a>
                        <ThemeToggle />
                    </nav>
                </div>
            </header>
            <main>
                <div class="container">
                    <Routes fallback=|| view! { <p>"Page not found."</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/providers/:domain") view=ProviderPage />
                        <Route path=path!("/providers/:domain/documents/:tracking_id") view=DocumentPage />
                        <Route path=path!("/sync") view=SyncStatusPage />
                    </Routes>
                </div>
            </main>
        </Router>
    }
}
