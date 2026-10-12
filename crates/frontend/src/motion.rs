use gloo_events::EventListener;
use gloo_timers::callback::Timeout;
use web_sys::{AnimationEvent, HtmlElement, MediaQueryList};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct FadeProps {
    pub identity: AttrValue,
    pub children: Children,
    #[prop_or_default]
    pub on_swap: Callback<()>,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Leaving,
    Entering,
}

/// Retains the outgoing view until its fade completes. Incoming updates always
/// use the latest props; background updates with the same identity don't fade.
pub struct FadeSwap {
    shown: Html,
    identity: AttrValue,
    phase: Phase,
    from: f64,
    generation: u64,
    node: NodeRef,
    notify: bool,
    timer: Option<Timeout>,
    media: Option<MediaQueryList>,
    _media_listener: Option<EventListener>,
}

pub enum Msg {
    Finished(u64),
    MotionChanged,
}

impl FadeSwap {
    fn reduced(&self) -> bool {
        self.media.as_ref().is_none_or(MediaQueryList::matches)
    }

    fn opacity(&self) -> f64 {
        self.node
            .cast::<HtmlElement>()
            .and_then(|el| web_sys::window()?.get_computed_style(&el).ok().flatten())
            .and_then(|style| {
                style
                    .get_property_value("opacity")
                    .ok()?
                    .parse::<f64>()
                    .ok()
            })
            .filter(|v| v.is_finite())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0)
    }

    fn phase(&mut self, ctx: &Context<Self>, phase: Phase, from: f64) {
        self.timer = None;
        self.generation += 1;
        self.phase = phase;
        self.from = from;
        if phase != Phase::Idle {
            let generation = self.generation;
            let link = ctx.link().clone();
            // Fallback if an animation is cancelled or its event isn't delivered.
            self.timer = Some(Timeout::new(320, move || {
                link.send_message(Msg::Finished(generation));
            }));
        }
    }

    fn swap(&mut self, ctx: &Context<Self>) {
        self.identity = ctx.props().identity.clone();
        self.shown = ctx.props().children.iter().collect();
        self.notify = true;
        self.phase(
            ctx,
            if self.reduced() {
                Phase::Idle
            } else {
                Phase::Entering
            },
            0.0,
        );
    }
}

impl Component for FadeSwap {
    type Message = Msg;
    type Properties = FadeProps;

    fn create(ctx: &Context<Self>) -> Self {
        let media = web_sys::window().and_then(|w| {
            w.match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        });
        let link = ctx.link().clone();
        let listener = media.as_ref().map(|media| {
            EventListener::new(media, "change", move |_| {
                link.send_message(Msg::MotionChanged)
            })
        });
        Self {
            shown: ctx.props().children.iter().collect(),
            identity: ctx.props().identity.clone(),
            phase: Phase::Idle,
            from: 1.0,
            generation: 0,
            node: NodeRef::default(),
            notify: false,
            timer: None,
            media,
            _media_listener: listener,
        }
    }

    fn changed(&mut self, ctx: &Context<Self>, _old: &Self::Properties) -> bool {
        if self.identity == ctx.props().identity {
            self.shown = ctx.props().children.iter().collect();
            if self.reduced() && self.phase != Phase::Idle {
                self.phase(ctx, Phase::Idle, 1.0);
            } else if self.phase == Phase::Leaving {
                self.phase(ctx, Phase::Entering, self.opacity());
            }
        } else if self.reduced() {
            self.swap(ctx);
        } else if self.phase != Phase::Leaving {
            self.phase(ctx, Phase::Leaving, self.opacity());
        }
        true
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Msg) -> bool {
        match msg {
            Msg::Finished(generation) if generation == self.generation => match self.phase {
                Phase::Leaving => self.swap(ctx),
                Phase::Entering => self.phase(ctx, Phase::Idle, 1.0),
                Phase::Idle => return false,
            },
            Msg::MotionChanged if self.reduced() => {
                if self.identity == ctx.props().identity {
                    self.phase(ctx, Phase::Idle, 1.0);
                } else {
                    self.swap(ctx);
                }
            }
            _ => return false,
        }
        true
    }

    fn rendered(&mut self, ctx: &Context<Self>, _first: bool) {
        if self.notify {
            self.notify = false;
            ctx.props().on_swap.emit(());
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let generation = self.generation;
        let phase = self.phase;
        let name = match phase {
            Phase::Idle => "idle",
            Phase::Leaving => "leaving",
            Phase::Entering => "entering",
        };
        let onanimationend = ctx.link().batch_callback(move |event: AnimationEvent| {
            let expected = if phase == Phase::Leaving {
                "ui-fade-out"
            } else {
                "ui-fade-in"
            };
            (event.target() == event.current_target() && event.animation_name() == expected)
                .then_some(Msg::Finished(generation))
        });
        html! {<div class="fade-swap" ref={self.node.clone()} data-phase={name}
            style={format!("--fade-from:{}",self.from)}
            inert={(phase==Phase::Leaving).then_some("")}
            aria-hidden={(phase==Phase::Leaving).then_some("true")}
            {onanimationend}>
            <div key={self.identity.to_string()}>{self.shown.clone()}</div>
        </div>}
    }
}
