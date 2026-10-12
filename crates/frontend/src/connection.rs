//! Connection motion follows the transport's ready/close events, never a fake progress timer.
use crate::{
    api::ConnectionState,
    i18n::{Language, t},
    icons::icon,
};
use gloo_timers::callback::Timeout;
use yew::prelude::*;

pub fn state_name(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Reconnecting => "reconnecting",
    }
}
fn sculpture(connected: bool) -> Html {
    html! {<div class="connection-sculpture" aria-hidden="true">
        <span class="connection-aura"/>
        <span class="connection-orbit orbit-one"><i/></span>
        <span class="connection-orbit orbit-two"><i/></span>
        <img class="connection-orb" src="/assets/everyday-companion.png" width="192" height="192" alt="" decoding="async"/>
        if connected {<span class="connection-seal">{icon("check")}</span>}
    </div>}
}
pub fn splash(error: &str) -> Html {
    html! {<main class="connection-splash" data-testid="connection-splash">
        <section class="connection-stage" role="status" aria-live="polite" aria-atomic="true">
            <div class="connection-kicker">{"INVISIBLE AI"}</div>
            {sculpture(false)}
            <h1>{t("A little clarity, on its way.")}</h1>
            <p class="connection-stage-caption">{t("Connecting to your workspace…")}<span class="connection-ellipsis" aria-hidden="true"><i/><i/><i/></span></p>
            <div class="connection-journey" aria-hidden="true"><span class="journey-node">{icon("monitor")}</span><span class="journey-thread"><i/></span><span class="journey-node">{icon("spark")}</span></div>
            <div class="connection-journey-labels" aria-hidden="true"><span>{t("Your browser")}</span><span>{t("Your workspace")}</span></div>
            if !error.is_empty() {<p class="connection-retry-copy">{t("We’ll reconnect automatically.")}</p>}
        </section>
    </main>}
}
pub fn badge(state: ConnectionState, open: Callback<MouseEvent>) -> Html {
    html! {<button class={classes!("connection", "connection-badge", (state != ConnectionState::Connected).then_some("disconnected"))}
        data-state={state_name(state)} aria-label={t("Backend connection")} aria-describedby="connection-label"
        title={t("View connected devices")} onclick={open}>
        <span class="connection-signal" aria-hidden="true"><span/>{if state == ConnectionState::Connected {icon("check")} else {Html::default()}}</span>
        <span id="connection-label">{t(state.label())}</span>
    </button>}
}
#[derive(Properties, PartialEq)]
pub struct Props {
    pub state: ConnectionState,
    pub language: Language,
}
pub struct ConnectionActivity {
    visible: bool,
    leaving: bool,
    seen_connected: bool,
    restored: bool,
    timer: Option<Timeout>,
    generation: u64,
}
pub enum Msg {
    Leave(u64),
    Hide(u64),
}
impl ConnectionActivity {
    fn settle(&mut self, ctx: &Context<Self>) {
        let link = ctx.link().clone();
        let generation = self.generation;
        self.timer = Some(Timeout::new(3600, move || {
            link.send_message(Msg::Leave(generation))
        }));
    }
}
impl Component for ConnectionActivity {
    type Message = Msg;
    type Properties = Props;
    fn create(ctx: &Context<Self>) -> Self {
        let connected = ctx.props().state == ConnectionState::Connected;
        let mut this = Self {
            visible: true,
            leaving: false,
            seen_connected: connected,
            restored: false,
            timer: None,
            generation: 0,
        };
        if connected {
            this.settle(ctx);
        }
        this
    }
    fn changed(&mut self, ctx: &Context<Self>, old: &Props) -> bool {
        if old.state != ctx.props().state {
            self.generation += 1;
            self.timer = None;
            self.visible = true;
            self.leaving = false;
            if ctx.props().state == ConnectionState::Connected {
                self.restored = self.seen_connected;
                self.seen_connected = true;
                self.settle(ctx);
            }
        }
        true
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Leave(generation)
                if generation == self.generation
                    && ctx.props().state == ConnectionState::Connected =>
            {
                self.leaving = true;
                let link = ctx.link().clone();
                self.timer = Some(Timeout::new(400, move || {
                    link.send_message(Msg::Hide(generation))
                }));
            }
            Msg::Hide(generation)
                if generation == self.generation
                    && ctx.props().state == ConnectionState::Connected =>
            {
                self.visible = false;
                self.timer = None;
            }
            _ => return false,
        }
        true
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        if !self.visible {
            return Html::default();
        }
        let state = ctx.props().state;
        let connected = state == ConnectionState::Connected;
        let title = if connected {
            if self.restored {
                "Back in sync."
            } else {
                "Your workspace is ready."
            }
        } else if state == ConnectionState::Reconnecting {
            "Finding our way back…"
        } else {
            "Connecting your space…"
        };
        html! {<aside class={classes!("connection-activity", self.leaving.then_some("is-leaving"))} data-state={state_name(state)}
            data-testid="connection-activity" role="status" aria-live="polite" aria-atomic="true">
            {sculpture(connected)}
            <div class="connection-activity-copy"><div class="connection-kicker">{t(state.label())}</div><strong>{t(title)}</strong><p>{if connected {t("Everything is in place.")} else {t("We’ll reconnect automatically.")}}</p></div>
            <span class="connection-activity-line" aria-hidden="true"><i/></span>
        </aside>}
    }
}
