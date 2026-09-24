use crate::{
    components::{
        badge::{Badge, BadgeVariant},
        progress_bar::{ProgressColor, color_for_pass_rate, format_rate},
    },
    models::ProfileSummary,
};
use leptos::prelude::*;

/// Displays a provider-level profile pass-rate badge.
#[component]
pub fn ProfileBadge(profile: Option<ProfileSummary>) -> impl IntoView {
    match profile {
        Some(p) => {
            let variant = match color_for_pass_rate(p.pass_rate) {
                ProgressColor::Emerald => BadgeVariant::Success,
                ProgressColor::Amber => BadgeVariant::Warning,
                ProgressColor::Red => BadgeVariant::Danger,
            };
            let label = format_rate(p.pass_rate, 1);
            view! { <Badge variant=variant>{label}</Badge> }.into_any()
        }
        None => view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> }.into_any(),
    }
}
