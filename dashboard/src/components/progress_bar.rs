use leptos::prelude::*;

/// Fill color for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressColor {
    /// Green for healthy metrics (pass rate ≥ 95%).
    Emerald,
    /// Yellow for cautionary metrics (pass rate ≥ 80%).
    Amber,
    /// Red for problematic metrics (pass rate < 80%).
    Red,
}

impl ProgressColor {
    /// Returns the Tailwind background class for this color.
    fn bar_class(self) -> &'static str {
        match self {
            Self::Emerald => "bg-emerald-500 h-2.5 rounded-full",
            Self::Amber => "bg-amber-500 h-2.5 rounded-full",
            Self::Red => "bg-red-500 h-2.5 rounded-full",
        }
    }
}

/// Returns the appropriate progress color for a pass rate (0.0–1.0).
pub fn color_for_pass_rate(rate: f64) -> ProgressColor {
    if rate >= 0.95 {
        ProgressColor::Emerald
    } else if rate >= 0.80 {
        ProgressColor::Amber
    } else {
        ProgressColor::Red
    }
}

/// A horizontal progress bar with colored fill.
#[component]
pub fn ProgressBar(
    /// Fill percentage (0.0–100.0).
    percentage: f64,
    /// Fill color.
    color: ProgressColor,
) -> impl IntoView {
    let width = format!("width: {percentage:.1}%");

    view! {
        <div class="w-full bg-gray-200 rounded-full h-2.5 dark:bg-gray-700">
            <div class={color.bar_class()} style={width}></div>
        </div>
    }
}
