use leptos::prelude::*;

use crate::models::SignatureSummary;

/// Displays an aggregate signature status badge for a provider.
#[component]
pub fn SignatureBadge(signatures: Option<SignatureSummary>) -> impl IntoView {
    match signatures {
        Some(s) => {
            let (class, label) = if s.invalid > 0 {
                ("badge badge-danger", format!("{} invalid", s.invalid))
            } else if s.valid + s.invalid + s.missing > 0 {
                let total = (s.valid + s.invalid + s.missing) as f64;
                let present_rate = (s.valid + s.invalid) as f64 / total;
                let class = if present_rate >= 0.95 {
                    "badge badge-success"
                } else {
                    "badge badge-warning"
                };
                (class, format!("{:.0}% signed", present_rate * 100.0))
            } else {
                ("badge", "-".to_string())
            };
            view! { <span class={class}>{label}</span> }.into_any()
        }
        None => view! { <span class="badge">"-"</span> }.into_any(),
    }
}
