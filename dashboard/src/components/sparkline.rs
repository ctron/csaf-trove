use csaf_trove_common::SyncPoint;
use leptos::prelude::*;

const SLOTS: usize = 30;

/// Inline SVG bar chart showing recent sync points.
///
/// `global_max` scales all sparklines in the table to the same baseline
/// so providers with more changes show taller bars. Always renders 30 slots,
/// left-padding with zeros when fewer data points exist.
#[component]
pub fn Sparkline(points: Vec<SyncPoint>, global_max: u64) -> impl IntoView {
    let max = global_max.max(1) as f64;

    let pad_count = SLOTS.saturating_sub(points.len());
    let mut padded: Vec<Option<&SyncPoint>> = vec![None; pad_count];
    padded.extend(points.iter().map(Some));
    padded.truncate(SLOTS);

    let width: f64 = 120.0;
    let height: f64 = 24.0;
    let gap: f64 = 1.0;
    let bar_width = (width - gap * (SLOTS as f64 - 1.0)) / SLOTS as f64;

    let bars: Vec<_> = padded
        .iter()
        .enumerate()
        .map(|(i, point)| {
            let (v, title) = match point {
                Some(p) => {
                    let date = p.timestamp.date().to_string();
                    (p.count, format!("{date}: {}", p.count))
                }
                None => (0, "0".to_string()),
            };

            let (bar_height, fill) = if v > 0 {
                (((v as f64 / max) * height).max(2.0), "#6366f1")
            } else {
                (1.0, "#d1d5db")
            };
            let x = i as f64 * (bar_width + gap);
            let y = height - bar_height;
            view! {
                <rect
                    x=x.to_string()
                    y=y.to_string()
                    width=bar_width.to_string()
                    height=bar_height.to_string()
                    fill=fill
                    rx="0.5"
                >
                    <title>{title}</title>
                </rect>
            }
        })
        .collect();

    view! {
        <svg
            width=width.to_string()
            height=height.to_string()
            viewBox=format!("0 0 {width} {height}")
            class="inline-block"
        >
            {bars}
        </svg>
    }
}
