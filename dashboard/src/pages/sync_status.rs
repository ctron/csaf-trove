use std::collections::HashMap;

use leptos::prelude::*;

use crate::models::JobStatus;

async fn fetch_sync_status() -> Result<HashMap<String, JobStatus>, String> {
    let resp = gloo_net::http::Request::get("/api/sync/status")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

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

#[component]
pub fn SyncStatusPage() -> impl IntoView {
    let trigger = RwSignal::new(0u32);

    let jobs = LocalResource::new(move || {
        trigger.get();
        fetch_sync_status()
    });

    wasm_bindgen_futures::spawn_local(async move {
        loop {
            gloo_timers::future::sleep(std::time::Duration::from_secs(5)).await;
            trigger.update(|n| *n += 1);
        }
    });

    view! {
        <div>
            <h2>"Sync Status"</h2>
            <Suspense fallback=|| view! { <p class="loading">"Loading..."</p> }>
                {move || jobs.get().map(|result| match result {
                    Ok(map) => view! { <JobTable jobs=map /> }.into_any(),
                    Err(e) => view! { <p class="error">{e}</p> }.into_any(),
                })}
            </Suspense>
        </div>
    }
}

#[component]
fn JobTable(jobs: HashMap<String, JobStatus>) -> impl IntoView {
    let mut entries: Vec<_> = jobs.into_iter().collect();
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
                        "running" => "badge badge-yellow",
                        "completed" => "badge badge-green",
                        "failed" => "badge badge-red",
                        _ => "badge",
                    };
                    let status = job.status.clone();
                    let phase = job.phase.clone().unwrap_or_else(|| "-".to_string());
                    let duration = format_duration(job.duration_seconds);
                    let started = job.started_at.clone();
                    let error = job.error.clone().unwrap_or_default();
                    view! {
                        <tr>
                            <td>{domain}</td>
                            <td><span class={status_class}>{status}</span></td>
                            <td>{phase}</td>
                            <td>{job.documents_synced}</td>
                            <td>{job.documents_validated}</td>
                            <td>{duration}</td>
                            <td>{started}</td>
                            <td>{error}</td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}
