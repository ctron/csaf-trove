use leptos::prelude::*;

/// Visual style variant for an alert.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum AlertVariant {
    /// Green alert for success messages.
    Success,
    /// Blue alert for informational messages.
    Info,
    /// Yellow alert for warning messages.
    Warning,
    /// Red alert for error messages.
    Error,
}

impl AlertVariant {
    /// Returns the accent bar background color class.
    fn bar_class(self) -> &'static str {
        match self {
            Self::Success => "bg-emerald-500",
            Self::Info => "bg-blue-500",
            Self::Warning => "bg-yellow-400",
            Self::Error => "bg-red-500",
        }
    }

    /// Returns the title text color classes.
    fn title_class(self) -> &'static str {
        match self {
            Self::Success => "text-emerald-500 dark:text-emerald-400",
            Self::Info => "text-blue-500 dark:text-blue-400",
            Self::Warning => "text-yellow-500 dark:text-yellow-300",
            Self::Error => "text-red-500 dark:text-red-400",
        }
    }

    /// Returns the SVG icon path for this variant.
    fn icon_path(self) -> &'static str {
        match self {
            Self::Success => "M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z",
            Self::Info => "M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z",
            Self::Warning => {
                "M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            }
            Self::Error => "M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z",
        }
    }
}

/// A pop-style alert following the Meraki UI pattern.
#[component]
pub fn Alert(variant: AlertVariant, children: Children) -> impl IntoView {
    view! {
        <div class="flex w-full overflow-hidden bg-white rounded-lg shadow-md dark:bg-gray-800 mb-4">
            <div class={format!("flex items-center justify-center w-12 {}", variant.bar_class())}>
                <svg class="w-6 h-6 text-white fill-current" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d={variant.icon_path()} stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none" />
                </svg>
            </div>
            <div class="px-4 py-2 -mx-3">
                <div class="mx-3">
                    <p class={format!("text-sm {}", variant.title_class())}>
                        {children()}
                    </p>
                </div>
            </div>
        </div>
    }
}
