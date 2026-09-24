use leptos::prelude::*;

/// Fill color for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressColor {
    /// Green for healthy metrics (everything passed).
    Emerald,
    /// Yellow for cautionary metrics (pass rate ≥ 80%, but not everything passed).
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
///
/// Only a perfect rate is green, so green always means "no problems".
pub fn color_for_pass_rate(rate: f64) -> ProgressColor {
    if rate >= 1.0 {
        ProgressColor::Emerald
    } else if rate >= 0.80 {
        ProgressColor::Amber
    } else {
        ProgressColor::Red
    }
}

/// Formats a rate (0.0–1.0) as a percentage, capping imperfect rates below 100%
/// so they never display as 100%.
pub fn format_rate(rate: f64, decimals: usize) -> String {
    let mut pct = rate * 100.0;
    if rate < 1.0 {
        pct = pct.min(100.0 - 10f64.powi(-(decimals as i32)));
    }
    format!("{pct:.decimals$}%")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Imperfect rates must never read as 100% or be colored green.
    #[test]
    fn imperfect_rates_are_not_perfect() {
        assert_eq!(format_rate(0.99996, 1), "99.9%");
        assert_eq!(format_rate(0.954, 1), "95.4%");
        assert_eq!(format_rate(1.0, 1), "100.0%");
        assert_eq!(format_rate(0.996, 0), "99%");
        assert_eq!(color_for_pass_rate(0.954), ProgressColor::Amber);
        assert_eq!(color_for_pass_rate(0.99996), ProgressColor::Amber);
        assert_eq!(color_for_pass_rate(1.0), ProgressColor::Emerald);
        assert_eq!(color_for_pass_rate(0.5), ProgressColor::Red);
    }
}
