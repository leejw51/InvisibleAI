//! Tweened progress: the bar never jumps to a polled value, it eases there.
//! The easing is exponential in and out, so a bar that has to move far
//! starts slowly, sweeps, and settles softly.
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use yew::prelude::*;

/// Standard easeInOutExpo: f(0)=0, f(1)=1, symmetric around (0.5, 0.5).
pub fn ease_in_out_expo(t: f64) -> f64 {
    if t <= 0.0 {
        0.0
    } else if t >= 1.0 {
        1.0
    } else if t < 0.5 {
        2f64.powf(20.0 * t - 10.0) / 2.0
    } else {
        (2.0 - 2f64.powf(-20.0 * t + 10.0)) / 2.0
    }
}
/// Where a value eased from `from` to `to` is after `elapsed_ms` of a
/// `duration_ms` tween.
pub fn tween_value(from: f64, to: f64, elapsed_ms: f64, duration_ms: f64) -> f64 {
    if duration_ms <= 0.0 || elapsed_ms >= duration_ms {
        return to;
    }
    let t = (elapsed_ms / duration_ms).clamp(0.0, 1.0);
    from + (to - from) * ease_in_out_expo(t)
}
pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}
/// Seconds left at the current speed, as a short text; empty until a speed is known.
pub fn eta_text(remaining_bytes: u64, bytes_per_second: u64) -> String {
    if bytes_per_second == 0 || remaining_bytes == 0 {
        return String::new();
    }
    let secs = remaining_bytes / bytes_per_second;
    if secs >= 3600 {
        format!("{}h {}m left", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s left", secs / 60, secs % 60)
    } else {
        format!("{secs}s left")
    }
}

const DURATION_MS: f64 = 700.0;

#[derive(Properties, PartialEq)]
pub struct ProgressProps {
    /// 0.0 to 1.0, the latest value reported by the backend.
    pub fraction: f64,
    pub label: AttrValue,
    #[prop_or_default]
    pub detail: AttrValue,
}
pub enum Msg {
    Frame(f64),
}
/// A progress bar whose fill tweens to each new fraction with requestAnimationFrame.
/// Readers who prefer reduced motion get the value at once.
pub struct ProgressBar {
    shown: f64,
    from: f64,
    to: f64,
    start: Option<f64>,
    frame: Option<Closure<dyn FnMut(f64)>>,
    handle: Option<i32>,
}
impl ProgressBar {
    fn reduced_motion() -> bool {
        web_sys::window()
            .and_then(|w| {
                w.match_media("(prefers-reduced-motion: reduce)")
                    .ok()
                    .flatten()
            })
            .is_some_and(|m| m.matches())
    }
    fn request(&mut self, ctx: &Context<Self>) {
        let link = ctx.link().clone();
        let closure = Closure::wrap(
            Box::new(move |ts: f64| link.send_message(Msg::Frame(ts))) as Box<dyn FnMut(f64)>
        );
        self.handle = web_sys::window().and_then(|w| {
            w.request_animation_frame(closure.as_ref().unchecked_ref())
                .ok()
        });
        self.frame = Some(closure);
    }
    /// Cancel a pending frame before its closure is freed.
    fn cancel(&mut self) {
        if let (Some(handle), Some(w)) = (self.handle.take(), web_sys::window()) {
            let _ = w.cancel_animation_frame(handle);
        }
        self.frame = None;
    }
    fn retarget(&mut self, ctx: &Context<Self>, to: f64) {
        let to = to.clamp(0.0, 1.0);
        if Self::reduced_motion() {
            self.shown = to;
            self.from = to;
            self.to = to;
            return;
        }
        self.from = self.shown;
        self.to = to;
        self.start = None;
        self.cancel();
        self.request(ctx);
    }
}
impl Component for ProgressBar {
    type Message = Msg;
    type Properties = ProgressProps;
    fn create(ctx: &Context<Self>) -> Self {
        let mut bar = Self {
            shown: 0.0,
            from: 0.0,
            to: 0.0,
            start: None,
            frame: None,
            handle: None,
        };
        bar.retarget(ctx, ctx.props().fraction);
        bar
    }
    fn changed(&mut self, ctx: &Context<Self>, old: &Self::Properties) -> bool {
        if (ctx.props().fraction - old.fraction).abs() > f64::EPSILON {
            self.retarget(ctx, ctx.props().fraction);
        }
        true
    }
    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        let Msg::Frame(ts) = msg;
        let start = *self.start.get_or_insert(ts);
        self.shown = tween_value(self.from, self.to, ts - start, DURATION_MS);
        self.handle = None;
        if ts - start < DURATION_MS {
            self.request(ctx);
        } else {
            self.frame = None;
        }
        true
    }
    fn destroy(&mut self, _ctx: &Context<Self>) {
        self.cancel();
    }
    fn view(&self, ctx: &Context<Self>) -> Html {
        let percent = (self.shown * 100.0).clamp(0.0, 100.0);
        let reported = (ctx.props().fraction * 100.0).round().clamp(0.0, 100.0) as u32;
        html! {<div class="model-progress">
            <div class="model-progress-track" role="progressbar" aria-label={ctx.props().label.clone()} aria-valuemin="0" aria-valuemax="100" aria-valuenow={reported.to_string()} aria-valuetext={format!("{reported}%")} data-target={format!("{:.3}",ctx.props().fraction.clamp(0.0,1.0)*100.0)}>
                <i style={format!("width:{percent:.2}%")}/>
            </div>
            <div class="model-progress-text"><strong data-testid="model-progress-percent">{format!("{reported}%")}</strong><span>{ctx.props().detail.clone()}</span></div>
        </div>}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_easing_starts_slow_sweeps_and_settles() {
        assert_eq!(ease_in_out_expo(0.0), 0.0);
        assert_eq!(ease_in_out_expo(1.0), 1.0);
        assert!((ease_in_out_expo(0.5) - 0.5).abs() < 1e-12);
        // Symmetric: easing in mirrors easing out.
        for i in 0..=100 {
            let t = i as f64 / 100.0;
            assert!(
                (ease_in_out_expo(t) + ease_in_out_expo(1.0 - t) - 1.0).abs() < 1e-9,
                "{t}"
            );
        }
        // Monotonic, slow at the ends, fastest in the middle.
        let mut last = 0.0;
        for i in 1..=100 {
            let v = ease_in_out_expo(i as f64 / 100.0);
            assert!(v >= last, "not monotonic at {i}");
            last = v;
        }
        assert!(ease_in_out_expo(0.1) < 0.01);
        assert!(ease_in_out_expo(0.9) > 0.99);
        assert!(ease_in_out_expo(0.55) - ease_in_out_expo(0.45) > 0.4);
        assert_eq!(ease_in_out_expo(-1.0), 0.0);
        assert_eq!(ease_in_out_expo(2.0), 1.0);
    }
    #[test]
    fn tween_moves_from_the_shown_value_to_the_target() {
        assert_eq!(tween_value(0.2, 0.6, 0.0, 700.0), 0.2);
        assert_eq!(tween_value(0.2, 0.6, 700.0, 700.0), 0.6);
        assert_eq!(tween_value(0.2, 0.6, 900.0, 700.0), 0.6);
        assert!((tween_value(0.2, 0.6, 350.0, 700.0) - 0.4).abs() < 1e-9);
        assert_eq!(tween_value(0.2, 0.6, 10.0, 0.0), 0.6);
        // Going backwards (a restarted download) eases too.
        assert!(tween_value(0.9, 0.1, 350.0, 700.0) < 0.9);
    }
    #[test]
    fn bytes_and_eta_read_naturally() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2_500_000), "2 MB");
        assert_eq!(format_bytes(2_934_000_000), "2.93 GB");
        assert_eq!(eta_text(0, 1000), "");
        assert_eq!(eta_text(1000, 0), "");
        assert_eq!(eta_text(45_000, 1000), "45s left");
        assert_eq!(eta_text(125_000, 1000), "2m 5s left");
        assert_eq!(eta_text(3_660_000, 1000), "1h 1m left");
    }
}
