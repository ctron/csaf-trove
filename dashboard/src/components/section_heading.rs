use leptos::prelude::*;

/// A section heading rendered as `<h2>` following the Meraki UI pattern.
#[component]
pub fn SectionHeading(children: Children) -> impl IntoView {
    view! {
        <h2 class="text-lg font-medium text-gray-800 dark:text-white mb-4">
            {children()}
        </h2>
    }
}

/// A sub-section heading rendered as `<h3>` following the Meraki UI pattern.
#[component]
pub fn SubHeading(children: Children) -> impl IntoView {
    view! {
        <h3 class="text-base font-medium text-gray-800 dark:text-white mt-6 mb-3">
            {children()}
        </h3>
    }
}
