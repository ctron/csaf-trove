use leptos::prelude::*;

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

    /// Converts to the `data-theme` attribute value, if any.
    fn as_attr(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Light => Some("light"),
            Self::Dark => Some("dark"),
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
    match theme.as_attr() {
        Some(v) => {
            let _ = storage.set_item("theme", v);
        }
        None => {
            let _ = storage.remove_item("theme");
        }
    }
}

/// Applies the theme by setting or removing the `data-theme` attribute on `<html>`.
fn apply_theme(theme: Theme) {
    let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    else {
        return;
    };
    match theme.as_attr() {
        Some(v) => {
            let _ = root.set_attribute("data-theme", v);
        }
        None => {
            let _ = root.remove_attribute("data-theme");
        }
    }
}

/// A button that cycles through system / light / dark themes.
#[component]
pub fn ThemeToggle() -> impl IntoView {
    let (theme, set_theme) = signal(stored_theme());

    Effect::new(move |_| {
        let t = theme.get();
        apply_theme(t);
        store_theme(t);
    });

    let toggle = move |_| {
        set_theme.update(|t| *t = t.next());
    };

    view! {
        <button class="theme-toggle" on:click=toggle>
            {move || theme.get().label()}
        </button>
    }
}
