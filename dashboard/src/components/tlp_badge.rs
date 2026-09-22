use leptos::prelude::*;

/// Renders a TLP label badge with the specification color for that level.
#[component]
pub fn TlpBadge(label: String) -> impl IntoView {
    let classes = match label.as_str() {
        "WHITE" | "CLEAR" => {
            "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal \
             text-gray-500 bg-gray-100 dark:bg-gray-800 dark:text-gray-400"
        }
        "GREEN" => {
            "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal \
             text-emerald-500 bg-emerald-100/60 dark:bg-gray-800"
        }
        "AMBER" => {
            "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal \
             text-amber-500 bg-amber-100/60 dark:bg-gray-800"
        }
        "RED" => {
            "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal \
             text-red-500 bg-red-100/60 dark:bg-gray-800"
        }
        _ => {
            "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal \
             text-gray-500 bg-gray-100 dark:bg-gray-800 dark:text-gray-400"
        }
    };
    let text = format!("TLP:{label}");
    view! {
        <span class=classes>{text}</span>
    }
}
