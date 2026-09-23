use leptos::prelude::*;
use crate::components::badge::{Badge, BadgeVariant};
use crate::models::DocumentProfileDetail;

/// Displays a per-document profile validation badge.
#[component]
pub fn DocProfileBadge(detail: Option<DocumentProfileDetail>) -> impl IntoView {
    match detail {
        Some(d) if d.passed => {
            view! { <Badge variant=BadgeVariant::Success>"Pass"</Badge> }.into_any()
        }
        Some(d) => {
            let mut parts = Vec::new();
            if d.error_count > 0 {
                parts.push(format!("{} errors", d.error_count));
            }
            if d.warning_count > 0 {
                parts.push(format!("{} warnings", d.warning_count));
            }
            if d.info_count > 0 {
                parts.push(format!("{} info", d.info_count));
            }
            let label = if parts.is_empty() {
                "Fail".to_string()
            } else {
                parts.join(", ")
            };
            let variant = if d.error_count > 0 {
                BadgeVariant::Danger
            } else if d.warning_count > 0 {
                BadgeVariant::Warning
            } else {
                BadgeVariant::Info
            };
            view! { <Badge variant=variant>{label}</Badge> }.into_any()
        }
        None => view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> }.into_any(),
    }
}
