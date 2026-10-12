//! A global indicator of actual requests, independent of the current page or selected mode.
use crate::{
    i18n::{Language, t},
    icons::icon,
};
use gloo_events::EventListener;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use yew::prelude::*;
#[derive(Clone, Deserialize, PartialEq)]
struct Call {
    provider: String,
    model: String,
    location: String,
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub language: Language,
}
#[function_component(Indicator)]
pub fn indicator(_props: &Props) -> Html {
    let calls = use_state(Vec::<Call>::new);
    let connected = use_state(|| false);
    {
        let calls = calls.clone();
        let connected = connected.clone();
        use_effect_with((), move |_| {
            let stream = web_sys::EventSource::new("/api/activity/events")
                .ok()
                .map(|source| {
                    let c = calls.clone();
                    let ready = connected.clone();
                    let message = EventListener::new(&source, "message", move |event| {
                        if let Some(message) = event.dyn_ref::<web_sys::MessageEvent>()
                            && let Some(data) = message.data().as_string()
                            && let Ok(value) = serde_json::from_str::<Vec<Call>>(&data)
                        {
                            c.set(value);
                            ready.set(true);
                        }
                    });
                    let error = EventListener::new(&source, "error", move |_| {
                        calls.set(Vec::new());
                        connected.set(false);
                    });
                    (source, message, error)
                });
            move || {
                if let Some((source, _, _)) = stream {
                    source.close();
                }
            }
        });
    }
    if !*connected || calls.is_empty() {
        return Html::default();
    }
    html! {<aside class="ai-location" role="status" aria-live="polite" aria-label={t("AI activity")}>
        {for ["local", "cloud", "service"].into_iter().filter(|location|calls.iter().any(|c|c.location==*location)).map(|location| {
            let (symbol,title,detail) = match location {
                "local" => ("shield","On-device AI is working","This AI request is processed on your Mac."),
                "cloud" => ("cloud","Cloud AI is working","This request is sent to your AI provider."),
                _ => ("server","AI service is working","This request uses your configured AI server."),
            };
            let models = calls.iter().filter(|c|c.location==location).map(|c|format!("{} · {}",c.provider,c.model)).collect::<Vec<_>>().join(", ");
            html! {<div class={classes!("ai-location-pill",location)} title={models} data-location={location}>{icon(symbol)}<span><strong>{t(title)}</strong><small>{t(detail)}</small></span><span class="ai-working-dot" aria-hidden="true"/></div>}
        })}
    </aside>}
}
