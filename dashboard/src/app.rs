use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::pages::home::HomePage;
use crate::pages::provider::ProviderPage;
use crate::pages::sync_status::SyncStatusPage;

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
                    </nav>
                </div>
            </header>
            <main>
                <div class="container">
                    <Routes fallback=|| view! { <p>"Page not found."</p> }>
                        <Route path=path!("/") view=HomePage />
                        <Route path=path!("/providers/:domain") view=ProviderPage />
                        <Route path=path!("/sync") view=SyncStatusPage />
                    </Routes>
                </div>
            </main>
        </Router>
    }
}
