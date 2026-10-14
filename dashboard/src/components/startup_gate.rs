use crate::components::progress_bar::{ProgressBar, ProgressColor};
use csaf_trove_common::startup::StartupStatus;
use leptos::prelude::*;
use std::time::Duration;

/// Interval between startup status polls.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Fetches the server's startup status.
async fn fetch_startup() -> Result<StartupStatus, String> {
    let resp = gloo_net::http::Request::get("/api/startup")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Shows the server's startup progress until it is ready, then renders its children.
#[component]
pub fn StartupGate(
    /// Content shown once the server has finished starting up.
    children: ChildrenFn,
) -> impl IntoView {
    // `None` until the first poll answered, to avoid flashing the startup screen.
    let status = RwSignal::new(None::<Result<StartupStatus, String>>);
    let ready =
        Memo::new(move |_| matches!(status.get(), Some(Ok(StartupStatus { ready: true, .. }))));

    wasm_bindgen_futures::spawn_local(async move {
        loop {
            let result = fetch_startup().await;
            let done = matches!(result, Ok(StartupStatus { ready: true, .. }));
            status.set(Some(result));
            if done {
                break;
            }
            gloo_timers::future::sleep(POLL_INTERVAL).await;
        }
    });

    view! {
        <Show when=move || ready.get() fallback=move || view! { <StartupProgress status /> }>
            {children()}
        </Show>
    }
}

/// Card showing the current startup task and the tasks already completed.
#[component]
fn StartupProgress(
    /// Latest startup status poll result, `None` before the first answer.
    status: RwSignal<Option<Result<StartupStatus, String>>>,
) -> impl IntoView {
    move || {
        let body = match status.get()? {
            Err(_) => view! {
                <p class="text-gray-500 dark:text-gray-400">"Waiting for server\u{2026}"</p>
            }
            .into_any(),
            Ok(s) => {
                let progress = (s.total > 0).then(|| {
                    let percentage = s.current as f64 / s.total as f64 * 100.0;
                    view! {
                        <div class="mt-3 flex items-center gap-x-3">
                            <ProgressBar percentage color=ProgressColor::Emerald />
                            <span class="text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap">
                                {format!("{} / {}", s.current, s.total)}
                            </span>
                        </div>
                    }
                });
                view! {
                    <ul class="space-y-1 text-sm">
                        {s.completed
                            .into_iter()
                            .map(|task| view! {
                                <li class="text-gray-500 dark:text-gray-400">"\u{2713} " {task}</li>
                            })
                            .collect_view()}
                        {s.task.map(|task| view! {
                            <li class="font-medium text-gray-800 dark:text-gray-200">"\u{2026} " {task}</li>
                        })}
                    </ul>
                    {progress}
                }
                .into_any()
            }
        };

        Some(view! {
            <div class="max-w-md mx-auto mt-16 p-6 bg-white rounded-lg shadow dark:bg-gray-800">
                <h2 class="text-lg font-semibold text-gray-800 dark:text-white mb-4">
                    "csaf-trove is starting up"
                </h2>
                {body}
            </div>
        })
    }
}
