//! A small, contextual companion shared by every non-chat page.
use crate::{i18n::t, icons::icon};
use yew::prelude::*;

pub fn page(page: &str, busy: bool) -> Html {
    let (symbol, title, hint) = match page {
        "save" => (
            "folder",
            "Leave it with me.",
            "No folders to choose. Just add something you want to remember.",
        ),
        "wiki" => (
            "search",
            "Let’s find that idea.",
            "Search your notes, or pick one to explore.",
        ),
        "autonomous" => (
            "spark",
            "A little help while you do your thing.",
            "Choose where you want help. Start when you’re ready, stop whenever you like.",
        ),
        "coding" | "code" | "skills" => (
            "code",
            "What shall we make?",
            "Describe your idea in your own words. We’ll take it one step at a time.",
        ),
        "images" => (
            "image",
            "A little inspiration, in pictures.",
            "Pick a picture to take a closer look.",
        ),
        "videos" => (
            "play",
            "Let’s revisit a moment.",
            "Pick a video to watch or ask about.",
        ),
        "devices" => (
            "monitor",
            "Let’s get your Mac ready.",
            "Connect your Mac here, then come back and say hello.",
        ),
        "ai" => (
            "spark",
            "Pick how I can help.",
            "Choose your AI here. You can change it anytime.",
        ),
        "assistance" => (
            "check",
            "One small thing at a time.",
            "Take a look at what’s next and choose what matters to you.",
        ),
        "quiz" => (
            "spark",
            "Ready for a little challenge?",
            "Give it a try. I’m here if you need a hand.",
        ),
        "files" | "library" => (
            "folder",
            "Your things, all together.",
            "Find something you saved, or add something new.",
        ),
        "dashboard" | "admin" => (
            "shield",
            "Let’s check everything is ready.",
            "Your connection and settings are here when you need them.",
        ),
        _ => (
            "spark",
            "What’s on your mind?",
            "Choose something you’d like help with.",
        ),
    };
    html! {<aside class="page-companion" data-page={page.to_string()} aria-label={t("Your companion")}>
        <invisible-companion key={page.to_string()} data-page-companion="true" data-scene={page.to_string()} data-state={if busy {"thinking"} else {"idle"}}>
            <div class="companion-world" aria-hidden="true"/><div class="companion-halo" aria-hidden="true"/>
            <button type="button" class="companion-touch" aria-label={t("Say hello to your companion")}><img class="companion-figure" src="/assets/everyday-companion.png" alt="" width="1280" height="1280"/></button>
            <span class="companion-prop" aria-hidden="true">{icon(symbol)}</span><span class="companion-spark" aria-hidden="true">{"✦"}</span>
        </invisible-companion>
        <div class="page-companion-copy">if page == "autonomous" {<h1>{t(title)}</h1>} else {<strong>{t(title)}</strong>}<p>{t(hint)}</p><span class="page-companion-working" role="status">{t("Working…")}</span></div>
    </aside>}
}
