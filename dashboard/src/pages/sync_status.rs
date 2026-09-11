use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::{JsCast, prelude::Closure};
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use crate::models::JobStatus;

/// Formats a duration in seconds as a human-readable string.
fn format_duration(seconds: Option<f64>) -> String {
    match seconds {
        None => "-".to_string(),
        Some(s) => {
            let total = s as u64;
            let hours = total / 3600;
            let minutes = (total % 3600) / 60;
            let secs = total % 60;
            if hours > 0 {
                format!("{hours}h {minutes}m {secs}s")
            } else if minutes > 0 {
                format!("{minutes}m {secs}s")
            } else {
                format!("{secs}s")
            }
        }
    }
}

/// Formats a relative time like "5m 23s ago" from an ISO 8601 timestamp.
fn format_relative_time(started_at: &str, now_ms: f64) -> String {
    let Ok(started) = chrono::DateTime::parse_from_rfc3339(started_at) else {
        return started_at.to_string();
    };

    let started_ms = started.timestamp_millis() as f64;
    let diff_seconds = ((now_ms - started_ms) / 1000.0).max(0.0) as u64;

    let days = diff_seconds / 86400;
    let hours = (diff_seconds % 86400) / 3600;
    let minutes = (diff_seconds % 3600) / 60;
    let secs = diff_seconds % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if secs > 0 || parts.is_empty() {
        parts.push(format!("{secs}s"));
    }
    format!("{} ago", parts.join(" "))
}

/// Builds the WebSocket URL from the current page origin.
fn ws_url() -> Option<String> {
    let origin = web_sys::window()?.location().origin().ok()?;
    let ws_origin = if origin.starts_with("https") {
        origin.replacen("https", "wss", 1)
    } else {
        origin.replacen("http", "ws", 1)
    };
    Some(format!("{ws_origin}/api/sync/ws"))
}

/// Opens a WebSocket connection and updates the jobs signal on incoming messages.
///
/// Automatically reconnects after a 2-second delay on close.
fn connect_ws(jobs: RwSignal<HashMap<String, JobStatus>>) {
    let Some(url) = ws_url() else {
        return;
    };

    let Ok(ws) = WebSocket::new(&url) else {
        spawn_reconnect(jobs);
        return;
    };

    let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
        if let Some(text) = e.data().as_string()
            && let Ok(data) = serde_json::from_str::<HashMap<String, JobStatus>>(&text)
        {
            jobs.set(data);
        }
    }) as Box<dyn FnMut(MessageEvent)>);
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    let onclose = Closure::wrap(Box::new(move |_: CloseEvent| {
        spawn_reconnect(jobs);
    }) as Box<dyn FnMut(CloseEvent)>);
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();
}

/// Schedules a reconnection attempt after a short delay.
fn spawn_reconnect(jobs: RwSignal<HashMap<String, JobStatus>>) {
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::sleep(std::time::Duration::from_secs(2)).await;
        connect_ws(jobs);
    });
}

#[component]
pub fn SyncStatusPage() -> impl IntoView {
    let jobs = RwSignal::new(HashMap::<String, JobStatus>::new());
    let now_ms = RwSignal::new(js_sys::Date::now());

    connect_ws(jobs);

    wasm_bindgen_futures::spawn_local(async move {
        loop {
            gloo_timers::future::sleep(std::time::Duration::from_secs(1)).await;
            now_ms.set(js_sys::Date::now());
        }
    });

    view! {
        <div>
            <h2>"Sync Status"</h2>
            {move || {
                let map = jobs.get();
                if map.is_empty() {
                    view! { <p class="text-muted text-center py-12">"Waiting for data\u{2026}"</p> }.into_any()
                } else {
                    let mut entries: Vec<_> = map.into_iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));

                    view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"Provider"</th>
                                    <th>"Status"</th>
                                    <th>"Phase"</th>
                                    <th>"Synced"</th>
                                    <th>"Validated"</th>
                                    <th>"Duration"</th>
                                    <th>"Started"</th>
                                    <th>"Error"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {entries.into_iter().map(|(domain, job)| {
                                    let status_class = match job.status.as_str() {
                                        "running" => "badge badge-warning",
                                        "completed" => "badge badge-success",
                                        "failed" => "badge badge-danger",
                                        _ => "badge",
                                    };
                                    let status = job.status.clone();
                                    let phase = job.phase.clone().unwrap_or_else(|| "-".to_string());
                                    let duration = format_duration(job.duration_seconds);
                                    let started_at = job.started_at.clone();
                                    let relative = format_relative_time(&started_at, now_ms.get());
                                    let error = job.error.clone().unwrap_or_default();
                                    view! {
                                        <tr>
                                            <td>{domain}</td>
                                            <td><span class={status_class}>{status}</span></td>
                                            <td>{phase}</td>
                                            <td>{job.documents_synced}</td>
                                            <td>{job.documents_validated}</td>
                                            <td>{duration}</td>
                                            <td title={started_at}>{relative}</td>
                                            <td>{error}</td>
                                        </tr>
                                    }
                                }).collect::<Vec<_>>()}
                            </tbody>
                        </table>
                    }.into_any()
                }
            }}
        </div>
    }
}
