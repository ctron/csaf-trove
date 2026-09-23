use leptos::prelude::*;
use crate::components::badge::{Badge, BadgeVariant};
use crate::models::SignatureSummary;

/// Displays an aggregate signature status badge for a provider.
#[component]
pub fn SignatureBadge(signatures: Option<SignatureSummary>) -> impl IntoView {
    match signatures {
        Some(s) => {
            let (variant, label) = if s.invalid > 0 {
                (BadgeVariant::Danger, format!("{} invalid", s.invalid))
            } else if s.valid + s.invalid + s.missing > 0 {
                let total = (s.valid + s.invalid + s.missing) as f64;
                let present_rate = (s.valid + s.invalid) as f64 / total;
                let variant = if present_rate >= 0.95 {
                    BadgeVariant::Success
                } else {
                    BadgeVariant::Warning
                };
                (variant, format!("{:.0}% signed", present_rate * 100.0))
            } else {
                (BadgeVariant::Neutral, "-".to_string())
            };
            view! { <Badge variant=variant>{label}</Badge> }.into_any()
        }
        None => view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> }.into_any(),
    }
}
