use yew::prelude::*;
pub fn icon(name: &str) -> Html {
    let path = match name {
        "refresh" => "M20 7a9 9 0 0 0-15-2L2 8m0-6v6h6M4 17a9 9 0 0 0 15 2l3-3m0 6v-6h-6",
        "moon" => "M20.9 13.3A9 9 0 0 1 10.7 3.1 9 9 0 1 0 20.9 13.3Z",
        "sun" => {
            "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5"
        }
        "dashboard" => "M3 3h7v7H3ZM14 3h7v7h-7ZM3 14h7v7H3ZM14 14h7v7h-7Z",
        "monitor" => "M3 4h18v12H3ZM12 16v4M8 21h8",
        "cloud" => "M6 19a4 4 0 0 1-.8-7.9A7 7 0 0 1 19 9a5 5 0 0 1 0 10H6Z",
        "globe" => {
            "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM3 12h18M12 3c5 5 5 13 0 18M12 3c-5 5-5 13 0 18"
        }
        "home" => "M3 10 12 3l9 7v10a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1Z",
        "spark" => "m12 3 2.7 6.3L21 12l-6.3 2.7L12 21l-2.7-6.3L3 12l6.3-2.7L12 3Z",
        "search" => "M21 21l-5-5M18 10.5a7.5 7.5 0 1 1-15 0 7.5 7.5 0 0 1 15 0Z",
        "book" => "M4 3h14a2 2 0 0 1 2 2v16H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3M3 17h17M8 7h7M8 11h5",
        "pin" => "m16 3 5 5-4 2-3 6-3-3-6 6m3-11 6-3 2-4M8 8l8 8",
        "folder" => {
            "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"
        }
        "code" => "m8 5-6 7 6 7m8-14 6 7-6 7m-3-16-2 18",
        "screen" => {
            "M8 3H4a1 1 0 0 0-1 1v4m13-5h4a1 1 0 0 1 1 1v4M3 16v4a1 1 0 0 0 1 1h4m8 0h4a1 1 0 0 0 1-1v-4M7 7h10v10H7Z"
        }
        "message" => "M21 11a8 8 0 0 1-8 8H7l-5 3 2-6a8 8 0 1 1 17-5ZM8 10h8M8 14h5",
        "arrow" => "M5 12h14m-6-6 6 6-6 6",
        "up" => "M12 20V4m-6 6 6-6 6 6",
        "down" => "m6 9 6 6 6-6",
        "plus" => "M12 4v16M4 12h16",
        "close" => "m6 6 12 12M6 18 18 6",
        "check" => "m5 12 4 4L19 6",
        "shield" => "m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Zm-4 9 3 3 5-6",
        "settings" => {
            "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM10 2h4l1 3 3 1 3 3-2 3 2 3-3 3-3 1-1 3h-4l-1-3-3-1-3-3 2-3-2-3 3-3 3-1 1-3Z"
        }
        "upload" => "M12 16V3m-5 5 5-5 5 5M3 15v5a1 1 0 0 0 1 1h16a1 1 0 0 0 1-1v-5",
        "download" => "M12 3v13m-5-5 5 5 5-5M3 16v4a1 1 0 0 0 1 1h16a1 1 0 0 0 1-1v-4",
        "file" => {
            "M14 2H5a1 1 0 0 0-1 1v18a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1V8l-6-6Zm0 0v6h6M8 13h8M8 17h5"
        }
        "audio" => {
            "M9 18V5l11-2v13M9 7l11-2M9 18a3 3 0 1 1-3-3c1.7 0 3 1.3 3 3ZM20 16a3 3 0 1 1-3-3c1.7 0 3 1.3 3 3Z"
        }
        "copy" => "M9 8h11v13H9ZM15 8V3H4v13h5",
        "wallet" => "M3 7h18v13H3ZM3 7l13-4v4M16 13.5h1.5",
        "trash" => "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7",
        "clock" => "M12 7v6l4 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z",
        "stop" => "M6 6h12v12H6Z",
        "leaf" => "M20 3C8 2 2 8 5 15s15 5 15-12ZM4 21l11-12",
        "bolt" => "m13 2-9 12h7l-1 8 10-13h-7l1-7Z",
        "phone" => {
            "M7 2h10a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1ZM11 18h2"
        }
        "send" => "M22 2 11 13M22 2 15 22l-4-9-9-4 20-7Z",
        "menu" => "M4 6h16M4 12h16M4 18h16",
        "hash" => "m9 3-3 18M18 3l-3 18M3 9h18M2 15h18",
        "film" => "M4 4h16v16H4zM4 9h16M4 15h16M8 4v16M16 4v16",
        "image" => "M4 5h16v14H4zM4 15l5-5 4 4 3-3 4 4M15 9h.01",
        "quiz" => {
            "M9 9a3 3 0 1 1 4.5 2.6c-.9.5-1.5 1.2-1.5 2.4M12 17h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z"
        }
        "user" => "M20 21a8 8 0 0 0-16 0M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z",
        "play" => "M8 5v14l11-7z",
        "zoom-in" => "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-5-5M11 8v6M8 11h6",
        "zoom-out" => "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-5-5M8 11h6",
        "expand" => "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5",
        // AI processing modes: a chip for on-device, a chip inside a cloud for
        // Hybrid, a cloud with a spark for Cloud AI, and terminals for the two
        // command-line tools, Claude Code with an asterisk and Codex with a cursor line.
        "ondevice" => {
            "M7 5h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2ZM9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3M9.5 12l2 2 4-4"
        }
        "hybrid" => {
            "M6 19a4 4 0 0 1-.8-7.9A7 7 0 0 1 18.5 10.5 4.5 4.5 0 0 1 18 19H6ZM10 11.5h4v4h-4ZM12 9.5v2M12 15.5v2M8 13.5h2M14 13.5h2"
        }
        "cloudai" => {
            "M6 19a4 4 0 0 1-.8-7.9A7 7 0 0 1 18.5 10.5 4.5 4.5 0 0 1 18 19H6Zm6-8.5.9 2.1 2.1.9-2.1.9-.9 2.1-.9-2.1-2.1-.9 2.1-.9.9-2.1Z"
        }
        "claude_code" => {
            "M3 5a1 1 0 0 1 1-1h16a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5ZM7 9l3 3-3 3M15.5 9v6M12.9 10.5l5.2 3M12.9 13.5l5.2-3"
        }
        "codex" => {
            "M3 5a1 1 0 0 1 1-1h16a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5ZM7 9l3 3-3 3M12 15h5"
        }
        _ => "M12 8v4m0 4h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z",
    };
    html! {<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={path}/></svg>}
}
/// The icon for an AI processing mode (`ondevice`, `hybrid`, `cloudai`,
/// `codex`, `claude_code`); unknown modes get the cloud.
pub fn mode_icon(mode: &str) -> &'static str {
    match mode {
        "ondevice" => "ondevice",
        "hybrid" => "hybrid",
        "codex" => "codex",
        "claude_code" => "claude_code",
        _ => "cloudai",
    }
}
pub fn brand() -> Html {
    html! {<svg width="26" height="26" viewBox="0 0 32 32" fill="none" aria-hidden="true"><path d="M7 9v16m9-22v22m9-13v13M4 29h24" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"/></svg>}
}
