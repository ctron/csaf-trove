use leptos::prelude::*;

use crate::models::ProfileSummary;

#[component]
pub fn ProfileBadge(profile: Option<ProfileSummary>) -> impl IntoView {
    match profile {
        Some(p) => {
            let class = if p.pass_rate >= 0.95 {
                "badge badge-success"
            } else if p.pass_rate >= 0.80 {
                "badge badge-warning"
            } else {
                "badge badge-danger"
            };
            let label = format!("{:.1}%", p.pass_rate * 100.0);
            view! { <span class={class}>{label}</span> }.into_any()
        }
        None => view! { <span class="badge">"-"</span> }.into_any(),
    }
}
