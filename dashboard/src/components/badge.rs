use leptos::prelude::*;

/// Visual style variant for a badge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BadgeVariant {
    /// Green badge for positive/passing states.
    Success,
    /// Yellow badge for caution states.
    Warning,
    /// Red badge for error/failing states.
    Danger,
    /// Blue badge for informational states.
    Info,
    /// Gray badge for neutral/unknown states.
    Neutral,
}

impl BadgeVariant {
    /// Returns the Tailwind classes for this variant.
    fn classes(self) -> &'static str {
        match self {
            Self::Success => {
                "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal text-emerald-500 bg-emerald-100/60 dark:bg-gray-800"
            }
            Self::Warning => {
                "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal text-amber-500 bg-amber-100/60 dark:bg-gray-800"
            }
            Self::Danger => {
                "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal text-red-500 bg-red-100/60 dark:bg-gray-800"
            }
            Self::Info => {
                "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal text-blue-500 bg-blue-100/60 dark:bg-gray-800"
            }
            Self::Neutral => {
                "inline-flex items-center px-3 py-1 rounded-full text-sm font-normal text-gray-500 bg-gray-100 dark:bg-gray-800 dark:text-gray-400"
            }
        }
    }
}

/// A colored pill badge following the Meraki UI pattern.
#[component]
pub fn Badge(variant: BadgeVariant, children: Children) -> impl IntoView {
    view! {
        <span class={variant.classes()}>
            {children()}
        </span>
    }
}
