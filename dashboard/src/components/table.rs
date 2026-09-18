use leptos::prelude::*;

/// A styled table wrapper following the Meraki UI pattern.
#[component]
pub fn Table(children: Children) -> impl IntoView {
    view! {
        <div class="overflow-hidden border border-gray-200 dark:border-gray-700 rounded-lg">
            <table class="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                {children()}
            </table>
        </div>
    }
}

/// A styled table header following the Meraki UI pattern.
#[component]
pub fn Thead(children: Children) -> impl IntoView {
    view! {
        <thead class="bg-gray-50 dark:bg-gray-800">
            {children()}
        </thead>
    }
}

/// A styled table body following the Meraki UI pattern.
#[component]
pub fn Tbody(children: Children) -> impl IntoView {
    view! {
        <tbody class="bg-white divide-y divide-gray-200 dark:divide-gray-700 dark:bg-gray-900">
            {children()}
        </tbody>
    }
}

/// A styled table header cell following the Meraki UI pattern.
#[component]
pub fn Th(children: Children) -> impl IntoView {
    view! {
        <th class="py-3.5 px-4 text-sm font-normal text-left text-gray-500 dark:text-gray-400">
            {children()}
        </th>
    }
}

/// A styled table data cell following the Meraki UI pattern.
#[component]
pub fn Td(
    #[prop(optional, default = "")] class: &'static str,
    children: Children,
) -> impl IntoView {
    let classes = if class.is_empty() {
        "px-4 py-4 text-sm text-gray-700 dark:text-gray-300".to_string()
    } else {
        format!("px-4 py-4 text-sm text-gray-700 dark:text-gray-300 {class}")
    };

    view! {
        <td class={classes}>
            {children()}
        </td>
    }
}
