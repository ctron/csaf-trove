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

#[component]
pub fn SyncStatusPage() -> impl IntoView {
    let jobs = LocalResource::new(fetch_sync_status);

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
                    let started = job.started_at.clone();
                    let error = job.error.clone().unwrap_or_default();
                    view! {
                        <tr>
                            <td>{domain}</td>
                            <td><span class={status_class}>{status}</span></td>
                            <td>{phase}</td>
                            <td>{job.documents_synced}</td>
                            <td>{job.documents_validated}</td>
                            <td>{started}</td>
                            <td>{error}</td>
                        </tr>
                    }
                }).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}
