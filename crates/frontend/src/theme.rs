use crate::{i18n::t, icons::icon};
use yew::prelude::*;
pub const STORAGE_KEY: &str = "causewaybay.theme.v1";
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}
impl Theme {
    pub fn parse(value: &str) -> Self {
        if value == "dark" {
            Self::Dark
        } else {
            Self::Light
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    pub fn toggle(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}
pub fn load() -> Theme {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
        .map(|v| Theme::parse(&v))
        .unwrap_or_default()
}
pub fn save(theme: Theme) -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .is_some_and(|s| s.set_item(STORAGE_KEY, theme.name()).is_ok())
}
pub fn activate(theme: Theme) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(root) = document.document_element() {
            let _ = root.set_attribute("data-theme", theme.name());
        }
        if let Ok(Some(meta)) = document.query_selector("meta[name='theme-color']") {
            let _ = meta.set_attribute(
                "content",
                if theme == Theme::Dark {
                    "#18161e"
                } else {
                    "#f8f7f4"
                },
            );
        }
    }
}
pub fn button(theme: Theme, toggle: Callback<()>) -> Html {
    let label = t(if theme == Theme::Light {
        "Switch to dark mode"
    } else {
        "Switch to light mode"
    });
    html! {<button type="button" class="theme-toggle" data-testid="theme-toggle" aria-label={label.clone()} title={label} onclick={toggle.reform(|_|())}>
        <span class="theme-toggle-icon" aria-hidden="true">{icon(if theme==Theme::Light {"sun"}else{"moon"})}</span>
        <span>{t(if theme==Theme::Light {"Light"}else{"Dark"})}</span>
    </button>}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggles_round_trip_and_invalid_storage_defaults_to_light() {
        assert_eq!(Theme::parse("dark").toggle(), Theme::Light);
        assert_eq!(Theme::parse("light").toggle(), Theme::Dark);
        assert_eq!(Theme::parse("invalid"), Theme::Light);
        for theme in [Theme::Light, Theme::Dark] {
            assert_eq!(Theme::parse(theme.name()), theme);
        }
    }
}
