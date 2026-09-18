use leptos::prelude::*;

use crate::components::badge::{Badge, BadgeVariant};
use crate::models::ProfileSummary;

/// Displays a provider-level profile pass-rate badge.
#[component]
pub fn ProfileBadge(profile: Option<ProfileSummary>) -> impl IntoView {
    match profile {
        Some(p) => {
            let variant = if p.pass_rate >= 0.95 {
                BadgeVariant::Success
            } else if p.pass_rate >= 0.80 {
                BadgeVariant::Warning
            } else {
                BadgeVariant::Danger
            };
            let label = format!("{:.1}%", p.pass_rate * 100.0);
            view! { <Badge variant=variant>{label}</Badge> }.into_any()
        }
        None => view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> }.into_any(),
    }
}
