use leptos::prelude::*;
use wasm_bindgen::{JsCast, prelude::Closure};

/// Theme preference: system default, forced light, or forced dark.
#[derive(Clone, Copy, PartialEq)]
enum Theme {
    /// Follow the OS / browser preference.
    System,
    /// Force light mode.
    Light,
    /// Force dark mode.
    Dark,
}

impl Theme {
    /// Returns the next theme in the cycle.
    fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    /// Display label shown on the toggle button.
    fn label(self) -> &'static str {
        match self {
            Self::System => "Auto",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Parses a stored string back into a theme.
    fn from_str(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// Returns the localStorage value for this theme.
    fn storage_value(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Light => Some("light"),
            Self::Dark => Some("dark"),
        }
    }
}

/// Returns whether the OS prefers dark mode.
fn os_prefers_dark() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok()?)
        .map(|mq| mq.matches())
        .unwrap_or(false)
}

/// Resolves a theme to the concrete `data-theme` attribute value.
fn resolve_data_theme(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::System => {
            if os_prefers_dark() {
                "dark"
            } else {
                "light"
            }
        }
    }
}

/// Reads the stored theme preference from localStorage.
fn stored_theme() -> Theme {
    web_sys::window()
        .and_then(|w| w.local_storage().ok()?)
        .and_then(|s| s.get_item("theme").ok()?)
        .map(|v| Theme::from_str(&v))
        .unwrap_or(Theme::System)
}

/// Persists the theme preference to localStorage.
fn store_theme(theme: Theme) {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok()?) else {
        return;
    };
    match theme.storage_value() {
        Some(v) => {
            let _ = storage.set_item("theme", v);
        }
        None => {
            let _ = storage.remove_item("theme");
        }
    }
}

/// Sets the `data-theme` attribute on `<html>`.
fn set_data_theme(value: &str) {
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-theme", value);
    }
}

/// A button that cycles through system / light / dark themes.
#[component]
pub fn ThemeToggle() -> impl IntoView {
    let (theme, set_theme) = signal(stored_theme());

    Effect::new(move |_| {
        let t = theme.get();
        set_data_theme(resolve_data_theme(t));
        store_theme(t);
    });

    // Listen for OS preference changes so System mode updates live.
    if let Some(mq) = web_sys::window()
        .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok()?)
    {
        let cb = Closure::wrap(Box::new(move || {
            if theme.get_untracked() == Theme::System {
                set_data_theme(resolve_data_theme(Theme::System));
            }
        }) as Box<dyn Fn()>);
        let _ = mq.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    let toggle = move |_| {
        set_theme.update(|t| *t = t.next());
    };

    view! {
        <button
            class="px-3 py-1.5 text-xs font-medium text-gray-700 bg-white border border-gray-200 rounded-lg hover:bg-gray-100 dark:bg-gray-800 dark:text-gray-200 dark:border-gray-700 dark:hover:bg-gray-700 transition-colors duration-200 cursor-pointer ml-auto"
            on:click=toggle
        >
            {move || theme.get().label()}
        </button>
    }
}
