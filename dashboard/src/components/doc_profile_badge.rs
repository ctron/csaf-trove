use crate::components::badge::{Badge, BadgeVariant};
use crate::models::DocumentProfileDetail;
use leptos::prelude::*;

/// Displays a per-document profile validation badge showing test pass counts.
///
/// Red means errors, amber warnings only, green passed or informational notes only.
#[component]
pub fn DocProfileBadge(detail: Option<DocumentProfileDetail>) -> impl IntoView {
    match detail {
        Some(d) if d.passed => {
            let label = if d.total_tests > 0 {
                format!("{}/{}", d.total_tests, d.total_tests)
            } else {
                "Pass".to_string()
            };
            view! { <Badge variant=BadgeVariant::Success>{label}</Badge> }.into_any()
        }
        Some(d) => {
            let passed = d.total_tests.saturating_sub(d.failing_test_count);
            let label = if d.total_tests > 0 {
                format!("{}/{}", passed, d.total_tests)
            } else {
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
                if parts.is_empty() {
                    "Fail".to_string()
                } else {
                    parts.join(", ")
                }
            };
            // Color by severity so green always means "ok", whatever the pass rate.
            let variant = if d.error_count > 0 {
                BadgeVariant::Danger
            } else if d.warning_count > 0 {
                BadgeVariant::Warning
            } else {
                BadgeVariant::Success
            };
            view! { <Badge variant=variant>{label}</Badge> }.into_any()
        }
        None => view! { <Badge variant=BadgeVariant::Neutral>"-"</Badge> }.into_any(),
    }
}
