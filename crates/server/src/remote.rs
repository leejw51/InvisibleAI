//! Remote prompts: what an iPhone or iPad user types for a running Invisible AI session.
//! Explicit commands are routed without a model. Free text goes to the configured model,
//! which may answer with one fixed action marker. Every argument is validated here;
//! model text is never executed and never reaches osascript.
use anyhow::{Result, ensure};

/// Longest prompt accepted from the web UI, in bytes.
pub const MAX_PROMPT: usize = 4_000;
/// Prompts waiting per session before the API refuses more.
pub const MAX_QUEUE: usize = 8;
/// Actions a model reply may request per prompt.
pub const MAX_ACTIONS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open a validated http(s) URL in Safari on the capture Mac.
    Open(String),
    /// Open a web search for this query in Safari on the capture Mac.
    Search(String),
    /// Evaluate arithmetic on the backend.
    Calculate(String),
}

/// How a remote prompt is handled before any model runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Direct(Action),
    Ask,
}

/// Instructions appended to the system prompt for remote prompts only.
pub const ACTION_GUIDE: &str = "The user sent this request from their phone to control their Mac. You may answer in plain text, or request an action by writing one of these markers on its own line at the start of a line (at most three):\n[OPEN_URL] https://example.com\n[SEARCH] words to search for\n[CALC] 12*(3+4)\nOPEN_URL opens Safari on the Mac at that http or https address. SEARCH opens a web search in Safari. CALC computes arithmetic. Use a marker only when the user asks to open, search, browse, or calculate; otherwise answer directly. Never invent addresses.";

/// The user asked for the expensive model by name, in a note or a phone request.
pub fn explicit_cloud(text: &str) -> bool {
    let tail: String = {
        let chars: Vec<char> = text.chars().collect();
        chars[chars.len().saturating_sub(400)..]
            .iter()
            .collect::<String>()
            .to_lowercase()
    };
    [
        "@cloud",
        "@codex",
        "@claude",
        "use cloud",
        "ask cloud",
        "cloud ai:",
        "클라우드로",
        "클라우드 ai",
        "클라우드에게",
        "클라우드한테",
    ]
    .iter()
    .any(|w| tail.contains(w))
}

/// Accept http(s) addresses only, with a host and no credentials; everything else fails.
pub fn validate_url(input: &str) -> Result<String> {
    let input = input.trim();
    ensure!(
        !input.is_empty() && input.len() <= 2_048,
        "Address must be 1–2048 characters"
    );
    ensure!(
        !input.chars().any(|c| c.is_control() || c.is_whitespace()),
        "Address must not contain spaces or control characters"
    );
    let url = reqwest::Url::parse(input).map_err(|_| anyhow::anyhow!("Not a valid address"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "Only http and https addresses can be opened"
    );
    ensure!(
        url.host_str().is_some_and(|h| !h.is_empty()),
        "Address has no host"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Addresses with credentials are not opened"
    );
    Ok(url.to_string())
}

/// A bare domain such as `youtube.com` or `docs.rs/serde`, promoted to https.
fn promote_domain(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches(['.', ',']);
    if text.is_empty() || text.contains(char::is_whitespace) || text.contains('@') {
        return None;
    }
    let host = text.split(['/', '?', '#']).next()?;
    let labels: Vec<&str> = host.split('.').collect();
    let valid = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    valid
        .then(|| validate_url(&format!("https://{text}")).ok())
        .flatten()
}

/// Any address the user could have meant: a full URL or a bare domain.
pub fn as_url(text: &str) -> Option<String> {
    validate_url(text).ok().or_else(|| promote_domain(text))
}

/// Safari search uses a fixed engine; the query is percent-encoded, never interpolated.
pub fn search_url(query: &str) -> Result<String> {
    let query = query.trim();
    ensure!(
        !query.is_empty() && query.len() <= 1_000,
        "Search words must be 1–1000 bytes"
    );
    let mut url = reqwest::Url::parse("https://duckduckgo.com/")?;
    url.query_pairs_mut().append_pair("q", query);
    Ok(url.to_string())
}

/// Drop trailing "=" and "?" from an arithmetic request, e.g. "12 * 34 = ?".
fn strip_tail(text: &str) -> &str {
    let mut text = text.trim();
    loop {
        let next = text.trim_end_matches(['=', '?']).trim();
        if next == text {
            return text;
        }
        text = next;
    }
}
fn strip_prefix_ci<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        return None;
    }
    for p in prefixes {
        if let Some(rest) = lower.strip_prefix(p)
            && (rest.is_empty()
                || rest.starts_with([' ', ':', '：', '\n', '\t'])
                || !p.ends_with(|c: char| c.is_ascii_alphabetic()))
        {
            let rest = &text[p.len()..];
            return Some(rest.trim_start_matches([' ', ':', '：', '\n', '\t']).trim());
        }
    }
    None
}

/// Decide without a model when the user typed an explicit command. English and
/// Korean verbs are recognized; anything else is a question for the model.
pub fn route(prompt: &str) -> Route {
    let text = prompt.trim();
    if let Some(url) = validate_url(text).ok().filter(|_| !text.contains(' ')) {
        return Route::Direct(Action::Open(url));
    }
    const OPEN: &[&str] = &[
        "open url",
        "open safari",
        "safari",
        "open",
        "go to",
        "goto",
        "visit",
        "사파리",
        "열어줘",
        "열어",
        "열기",
    ];
    if let Some(mut rest) = strip_prefix_ci(text, OPEN) {
        // "Safari, open docs.rs" and "사파리 열어줘 docs.rs" carry two verbs.
        if let Some(again) = strip_prefix_ci(rest, OPEN) {
            rest = again;
        }
        if let Some(url) = as_url(rest) {
            return Route::Direct(Action::Open(url));
        }
        if !rest.is_empty() {
            return Route::Direct(Action::Search(rest.into()));
        }
    }
    if let Some(rest) = strip_prefix_ci(
        text,
        &[
            "web search",
            "search for",
            "search",
            "google",
            "look up",
            "lookup",
            "검색해줘",
            "검색",
            "찾아줘",
            "구글",
        ],
    ) && !rest.is_empty()
    {
        return Route::Direct(Action::Search(rest.into()));
    }
    if let Some(rest) = strip_prefix_ci(
        text,
        &[
            "calculate",
            "calculator",
            "calc",
            "compute",
            "계산해줘",
            "계산기",
            "계산",
            "=",
        ],
    ) && !rest.is_empty()
    {
        return Route::Direct(Action::Calculate(strip_tail(rest).into()));
    }
    if text.len() <= 200
        && text.chars().any(|c| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_whitespace() || "+-*/%^().,=?".contains(c))
        && text.chars().any(|c| "+-*/%^".contains(c))
        && crate::productivity::calculate(strip_tail(text)).is_ok()
    {
        return Route::Direct(Action::Calculate(strip_tail(text).into()));
    }
    if let Some(url) = promote_domain(text) {
        return Route::Direct(Action::Open(url));
    }
    Route::Ask
}

/// Extract fixed action markers from a model reply. Returns the actions in order and
/// the reply text with marker lines removed. Invalid markers become visible errors.
pub fn parse_markers(reply: &str) -> (Vec<Result<Action>>, String) {
    let mut actions = vec![];
    let mut text = String::new();
    for line in reply.lines() {
        let trimmed = line.trim();
        let parsed = if let Some(rest) = trimmed.strip_prefix("[OPEN_URL]") {
            Some(
                as_url(rest)
                    .ok_or_else(|| {
                        anyhow::anyhow!("The model suggested an address that cannot be opened")
                    })
                    .map(Action::Open),
            )
        } else if let Some(rest) = trimmed.strip_prefix("[SEARCH]") {
            Some(search_url(rest).map(|_| Action::Search(rest.trim().into())))
        } else {
            trimmed.strip_prefix("[CALC]").map(|rest| {
                crate::productivity::calculate(rest.trim())
                    .map(|_| Action::Calculate(rest.trim().into()))
            })
        };
        match parsed {
            Some(action) if actions.len() < MAX_ACTIONS => actions.push(action),
            Some(_) => {}
            None => {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    (actions, text.trim().to_string())
}

/// Fixed summary of one action for the session card and the note. URLs and
/// results appear verbatim; nothing else from the model does.
pub fn describe(action: &Action, outcome: &str) -> String {
    match action {
        Action::Open(url) => format!("Opened in Safari: {url}"),
        Action::Search(query) => format!("Searched in Safari: {query}"),
        Action::Calculate(expr) => format!("{expr} = {outcome}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_commands_route_without_a_model() {
        assert_eq!(
            route("open https://example.com/a?b=1"),
            Route::Direct(Action::Open("https://example.com/a?b=1".into()))
        );
        assert_eq!(
            route("https://example.com"),
            Route::Direct(Action::Open("https://example.com/".into()))
        );
        assert_eq!(
            route("youtube.com"),
            Route::Direct(Action::Open("https://youtube.com/".into()))
        );
        assert_eq!(
            route("Open youtube"),
            Route::Direct(Action::Search("youtube".into()))
        );
        assert_eq!(
            route("사파리 열어줘 docs.rs"),
            Route::Direct(Action::Open("https://docs.rs/".into()))
        );
        assert_eq!(
            route("열어 docs.rs"),
            Route::Direct(Action::Open("https://docs.rs/".into()))
        );
        assert_eq!(
            route("search: bitcoin price"),
            Route::Direct(Action::Search("bitcoin price".into()))
        );
        assert_eq!(
            route("검색 비트코인 시세"),
            Route::Direct(Action::Search("비트코인 시세".into()))
        );
        assert_eq!(
            route("calc 12*34"),
            Route::Direct(Action::Calculate("12*34".into()))
        );
        assert_eq!(
            route("계산 (2+3)*4 ="),
            Route::Direct(Action::Calculate("(2+3)*4".into()))
        );
        assert_eq!(
            route("12 * 34 = ?"),
            Route::Direct(Action::Calculate("12 * 34".into()))
        );
        assert_eq!(route("Explain mining"), Route::Ask);
        assert_eq!(route("search"), Route::Ask);
        assert_eq!(route("opening hours of the museum"), Route::Ask);
        assert_eq!(route("2024"), Route::Ask);
        assert_eq!(
            route("open javascript:alert(1)"),
            Route::Direct(Action::Search("javascript:alert(1)".into()))
        );
    }

    #[test]
    fn explicit_cloud_requests_are_recognized() {
        assert!(explicit_cloud("Summarize this @cloud"));
        assert!(explicit_cloud("@Codex 이 코드 고쳐줘"));
        assert!(explicit_cloud("이건 클라우드로 물어봐"));
        assert!(!explicit_cloud("Cloud computing is a model of"));
        assert!(!explicit_cloud("What is a Merkle tree?"));
    }

    #[test]
    fn urls_are_validated_strictly() {
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user:pw@example.com").is_err());
        assert!(validate_url("https://").is_err());
        assert!(validate_url("https://example.com/a b").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert_eq!(
            validate_url(" http://example.com:8080/x ").unwrap(),
            "http://example.com:8080/x"
        );
        assert_eq!(promote_domain("-bad.com"), None);
        assert_eq!(promote_domain("localhost"), None);
        assert_eq!(promote_domain("a@b.com"), None);
        assert_eq!(
            search_url("bitcoin & 비트코인").unwrap(),
            "https://duckduckgo.com/?q=bitcoin+%26+%EB%B9%84%ED%8A%B8%EC%BD%94%EC%9D%B8"
        );
    }

    #[test]
    fn model_markers_are_parsed_and_bounded() {
        let (actions, text) = parse_markers(
            "Sure.\n[OPEN_URL] https://example.com\n[CALC] 2+2\n[SEARCH] rust async\n[OPEN_URL] https://ignored.example\nDone.",
        );
        let actions: Vec<Action> = actions.into_iter().map(|a| a.unwrap()).collect();
        assert_eq!(
            actions,
            [
                Action::Open("https://example.com/".into()),
                Action::Calculate("2+2".into()),
                Action::Search("rust async".into())
            ]
        );
        assert_eq!(text, "Sure.\nDone.");
        let (actions, text) =
            parse_markers("[OPEN_URL] javascript:alert(1)\n[CALC] rm -rf\nAnswer");
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().all(|a| a.is_err()));
        assert_eq!(text, "Answer");
        let (actions, text) = parse_markers("Plain answer with [OPEN_URL] inside a sentence.");
        assert!(actions.is_empty());
        assert_eq!(text, "Plain answer with [OPEN_URL] inside a sentence.");
    }
}
