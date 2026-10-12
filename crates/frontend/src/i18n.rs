//! Browser-wide UI language. Only explicitly marked UI text is translated;
//! wallet names, user content, AI output and protocol values are never rewritten.
use std::{cell::Cell, collections::HashMap, sync::LazyLock};
use web_sys::HtmlSelectElement;
use yew::prelude::*;

pub const STORAGE_KEY: &str = "causewaybay.language.v1";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    Korean,
    Cantonese,
    Chinese,
    Japanese,
    Spanish,
    Czech,
}
impl Language {
    pub const ALL: [Self; 7] = [
        Self::English,
        Self::Korean,
        Self::Cantonese,
        Self::Chinese,
        Self::Japanese,
        Self::Spanish,
        Self::Czech,
    ];
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Korean => "ko",
            Self::Cantonese => "yue-HK",
            Self::Chinese => "zh-CN",
            Self::Japanese => "ja",
            Self::Spanish => "es",
            Self::Czech => "cs",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Korean => "한국어",
            Self::Cantonese => "廣東話",
            Self::Chinese => "简体中文",
            Self::Japanese => "日本語",
            Self::Spanish => "Español",
            Self::Czech => "Čeština",
        }
    }
    pub fn parse(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == value)
            .unwrap_or_default()
    }
    fn index(self) -> Option<usize> {
        match self {
            Self::English => None,
            Self::Korean => Some(0),
            Self::Cantonese => Some(1),
            Self::Chinese => Some(2),
            Self::Japanese => Some(3),
            Self::Spanish => Some(4),
            Self::Czech => Some(5),
        }
    }
}
// A single Yew application owns this value. Its language prop invalidates child
// views without remounting them, so drafts, selections and SSE streams survive.
thread_local! { static CURRENT: Cell<Language> = const { Cell::new(Language::English) }; }
static CATALOG: LazyLock<HashMap<String, [String; 6]>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("locales.json")).expect("validated UI translation catalog")
});
pub fn t(text: &str) -> String {
    CURRENT
        .with(|language| language.get().index())
        .and_then(|index| CATALOG.get(text).map(|values| values[index].clone()))
        .unwrap_or_else(|| text.to_owned())
}
/// Named placeholders permit different word order without translating values.
pub fn tf(text: &str, values: &[(&str, String)]) -> String {
    let template = t(text);
    let mut output = String::new();
    let mut rest = template.as_str();
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let key = &rest[start + 1..start + end];
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
            output.push_str(value);
        } else {
            output.push_str(&rest[start..=start + end]);
        }
        rest = &rest[start + end + 1..];
    }
    output.push_str(rest);
    output
}
pub fn load() -> Language {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(STORAGE_KEY).ok().flatten())
        .map(|value| Language::parse(&value))
        .unwrap_or_default()
}
pub fn activate(language: Language) {
    CURRENT.with(|current| current.set(language));
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(root) = document.document_element() {
            let _ = root.set_attribute("lang", language.code());
        }
        document.set_title(&t("Invisible AI · Your thinking space"));
    }
}
pub fn save(language: Language) -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .is_some_and(|storage| storage.set_item(STORAGE_KEY, language.code()).is_ok())
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
}
pub fn selector(language: Language, changed: Callback<Language>) -> Html {
    html! {<label class="language-picker"><span class="language-symbol" aria-hidden="true">{"文/A"}</span><select data-testid="language-select" aria-label={t("Language")} title={t("Language")} value={language.code()} onchange={Callback::from(move |event: Event| changed.emit(Language::parse(&event.target_unchecked_into::<HtmlSelectElement>().value())))}>
        {for Language::ALL.into_iter().map(|option| html! {<option value={option.code()} selected={option == language} lang={option.code()}>{option.name()}</option>})}
    </select></label>}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_round_trip_and_unknown_preferences_fall_back() {
        for language in Language::ALL {
            assert_eq!(Language::parse(language.code()), language);
        }
        for value in ["", "null", "invalid", "{\"language\":\"ko\"}"] {
            assert_eq!(Language::parse(value), Language::English);
        }
    }

    #[test]
    fn translated_templates_preserve_parameter_text_without_recursive_expansion() {
        CURRENT.with(|current| current.set(Language::Korean));
        assert_eq!(t("Capture"), "캡처");
        let name = "<b>{duration}</b> 日本語.txt";
        assert_eq!(
            tf(
                "Download {name}",
                &[("name", name.into()), ("duration", "BAD".into())]
            ),
            format!("{name} 다운로드")
        );
        assert_eq!(
            t("Unrecognized server diagnostic"),
            "Unrecognized server diagnostic"
        );
        CURRENT.with(|current| current.set(Language::English));
        assert_eq!(t("Capture"), "Capture");
    }

    #[test]
    fn incomplete_templates_are_kept_readable() {
        CURRENT.with(|current| current.set(Language::English));
        assert_eq!(
            tf("Open {name} / {unknown}", &[("name", "file".into())]),
            "Open file / {unknown}"
        );
        assert_eq!(tf("Trailing {", &[]), "Trailing {");
    }
}
