//! Code: the reusable parts of everything kept, function by function. A kept
//! message with fenced code stays one Wiki node; here each function in it is
//! its own card with an Input section (parameters), an Output section (what it
//! returns), the code to copy, and a note the user can edit. Nothing is run or
//! sent anywhere: the split is a plain reading of signatures, on this computer.
use crate::{Api, ApiError, PrivateState, db, wiki};
use anyhow::Result;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Kept messages read per listing; the newest code first.
const SOURCES: usize = 400;
/// Functions per listing page.
const PAGE: usize = 60;

pub fn init(conn: &Connection) -> Result<()> {
    crate::schema::ensure(
        conn,
        "CREATE TABLE IF NOT EXISTS code_notes(id TEXT NOT NULL,owner TEXT NOT NULL,description TEXT NOT NULL,updated_at TEXT NOT NULL,PRIMARY KEY(owner,id));",
    )
}

/// One reusable piece: a function, or a whole block when no function is found.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub language: String,
    /// The signature as written, one line.
    pub signature: String,
    /// Input: (name, type), in order; `self`/`this` receivers are left out.
    pub params: Vec<(String, String)>,
    /// Output: the declared return type, or empty when none is written.
    pub returns: String,
    pub code: String,
}

/// Fenced blocks as (language, code); the fence's word is the language.
pub fn fenced_blocks(text: &str) -> Vec<(String, String)> {
    let mut blocks = vec![];
    let mut language = String::new();
    let mut body: Vec<&str> = vec![];
    let mut open = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if open {
                blocks.push((language.clone(), body.join("\n")));
                body.clear();
                open = false;
            } else {
                language = trimmed
                    .trim_start_matches('`')
                    .trim()
                    .split(|c: char| !c.is_alphanumeric() && c != '+' && c != '#')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                open = true;
            }
            continue;
        }
        if open {
            body.push(line);
        }
    }
    if open && !body.is_empty() {
        blocks.push((language, body.join("\n")));
    }
    blocks
}

/// The language a fence word means, so `rs`, `rust` and `Rust` meet.
fn canonical(language: &str) -> &'static str {
    match language {
        "rs" | "rust" => "rust",
        "py" | "python" | "python3" => "python",
        "js" | "javascript" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "typescript" | "tsx" => "typescript",
        "go" | "golang" => "go",
        "swift" => "swift",
        "kt" | "kotlin" => "kotlin",
        "sh" | "bash" | "zsh" | "shell" => "shell",
        "c" => "c",
        "cpp" | "c++" | "cc" | "cxx" => "cpp",
        "java" => "java",
        "cs" | "csharp" | "c#" => "csharp",
        "rb" | "ruby" => "ruby",
        "php" => "php",
        "sql" => "sql",
        "html" => "html",
        "css" => "css",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "" => "text",
        _ => "other",
    }
}

/// Where a function starts on this line, if one does: (name, keyword end).
fn signature_start(language: &str, line: &str) -> Option<String> {
    // `pub(crate) fn` and `pub(super) fn` are plain `pub fn` for this purpose.
    let plain = line
        .trim_start()
        .replacen("pub(crate) ", "pub ", 1)
        .replacen("pub(super) ", "pub ", 1);
    let t = plain.as_str();
    let words: Vec<&str> = t
        .split(|c: char| c.is_whitespace() || c == '(' || c == '<' || c == ':')
        .filter(|w| !w.is_empty())
        .collect();
    let after = |keyword: &str| -> Option<String> {
        let at = words.iter().position(|w| *w == keyword)?;
        // Only modifiers may come before the keyword; `let f = fn()` is not a declaration.
        if words[..at].iter().any(|w| {
            !matches!(
                *w,
                "pub"
                    | "pub(crate)"
                    | "pub(super)"
                    | "async"
                    | "unsafe"
                    | "const"
                    | "extern"
                    | "export"
                    | "default"
                    | "public"
                    | "private"
                    | "internal"
                    | "open"
                    | "static"
                    | "override"
                    | "final"
                    | "protected"
                    | "fileprivate"
                    | "suspend"
                    | "inline"
                    | "@objc"
                    | "mutating"
                    | "def"
            ) && !w.starts_with("pub(")
                && !w.starts_with("extern")
        }) {
            return None;
        }
        let name = words.get(at + 1)?;
        let name = name.trim_start_matches('*');
        (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .then(|| name.to_string())
    };
    match language {
        "rust" => after("fn"),
        "python" => after("def"),
        "javascript" | "typescript" => after("function").or_else(|| {
            // `const name = (a, b) => {` and `export const name = async (a) => {`.
            let at = words
                .iter()
                .position(|w| matches!(*w, "const" | "let" | "var"))?;
            let name = words.get(at + 1)?;
            (t.contains("=>") && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .then(|| name.to_string())
        }),
        "go" => {
            // `func (r *Receiver) Name(` names the method, not the receiver.
            let rest = t.strip_prefix("func ")?;
            let rest = if let Some(after) = rest.strip_prefix('(') {
                after.split_once(')')?.1.trim_start()
            } else {
                rest
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        }
        "swift" => after("func"),
        "kotlin" => after("fun"),
        _ => None,
    }
}

/// Split `a: T, b: Vec<(u8, u8)>` on the commas outside brackets.
fn split_top(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut depth = 0i32;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth <= 0 => {
                out.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.into_iter().filter(|p| !p.is_empty()).collect()
}

/// The text between the signature's first `(` and its matching `)`, and what follows.
fn params_and_rest(signature: &str) -> (String, String) {
    let Some(open) = signature.find('(') else {
        return (String::new(), signature.to_string());
    };
    let mut depth = 0i32;
    for (i, c) in signature[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let inner = &signature[open + 1..open + i];
                    let rest = &signature[open + i + 1..];
                    return (inner.to_string(), rest.to_string());
                }
            }
            _ => {}
        }
    }
    (signature[open + 1..].to_string(), String::new())
}

/// Input and Output read from one signature line.
fn inputs_outputs(language: &str, signature: &str) -> (Vec<(String, String)>, String) {
    // `pub(crate)` has a parenthesis of its own that is not the parameter list.
    let plain = signature
        .trim()
        .replacen("pub(crate) ", "pub ", 1)
        .replacen("pub(super) ", "pub ", 1);
    let signature = plain.as_str();
    // A Go method's receiver is not an input: `func (s *Server) Handle(...)`.
    let signature = match signature.strip_prefix("func (") {
        Some(after) if language == "go" => after.split_once(')').map(|p| p.1).unwrap_or(after),
        _ => signature,
    };
    let (inner, rest) = params_and_rest(signature);
    let params = split_top(&inner)
        .into_iter()
        .filter_map(|p| {
            let p = p.trim_start_matches("mut ").trim();
            if matches!(
                p,
                "self" | "&self" | "&mut self" | "this" | "cls" | "*" | "/"
            ) || p.starts_with("self:")
            {
                return None;
            }
            let p = p.split('=').next().unwrap_or(p).trim();
            if language == "go" {
                // `a, b int` shares a type; `name type`.
                let mut parts = p.rsplitn(2, ' ');
                let ty = parts.next().unwrap_or("").trim();
                let name = parts.next().unwrap_or("").trim();
                return Some(if name.is_empty() {
                    (ty.to_string(), String::new())
                } else {
                    (name.to_string(), ty.to_string())
                });
            }
            let (name, ty) = p.split_once(':').unwrap_or((p, ""));
            Some((
                name.trim().trim_start_matches("...").to_string(),
                ty.trim().to_string(),
            ))
        })
        .collect();
    let rest = rest.trim();
    let returns = if let Some(after) = rest.split_once("->") {
        after.1
    } else if language == "go" {
        rest
    } else if matches!(language, "typescript" | "kotlin") && rest.starts_with(':') {
        &rest[1..]
    } else {
        ""
    };
    let returns = returns
        .split(['{', '='])
        .next()
        .unwrap_or("")
        .split(" where ")
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches(':')
        .trim()
        .to_string();
    (params, returns)
}

/// Lines from `start` to the end of the function: brace languages close when
/// the braces balance, Python when the indentation returns.
fn function_end(language: &str, lines: &[&str], start: usize) -> usize {
    if language == "python" {
        let indent = lines[start].len() - lines[start].trim_start().len();
        let mut end = start;
        for (i, line) in lines.iter().enumerate().skip(start + 1) {
            if line.trim().is_empty() {
                continue;
            }
            let here = line.len() - line.trim_start().len();
            if here <= indent {
                break;
            }
            end = i;
        }
        return end;
    }
    let mut depth = 0i32;
    let mut seen = false;
    for (i, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => {
                    depth += 1;
                    seen = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if seen && depth <= 0 {
            return i;
        }
        // A one-line arrow function (`const f = (n) => n * 2;`) or a declaration
        // without a body ends at its semicolon; a signature wrapped over many
        // lines keeps going until its brace.
        if !seen && line.trim_end().ends_with(';') {
            return i;
        }
        if !seen && i > start + 24 {
            return start;
        }
    }
    lines.len() - 1
}

/// Every function in one fenced block. Doc comments, attributes and decorators
/// directly above a function travel with it.
pub fn functions(language: &str, block: &str) -> Vec<Function> {
    let language = canonical(language);
    let lines: Vec<&str> = block.lines().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let Some(name) = signature_start(language, lines[i]) else {
            i += 1;
            continue;
        };
        // The signature may span lines until its `{`, `:` or `=>`.
        let mut signature_end = i;
        let mut signature = lines[i].trim().to_string();
        while !signature.contains('{')
            && !signature.contains("=>")
            && !(language == "python" && signature.ends_with(':'))
            && signature_end + 1 < lines.len()
            && signature_end - i < 12
        {
            signature_end += 1;
            signature.push(' ');
            signature.push_str(lines[signature_end].trim());
        }
        let end = function_end(language, &lines, i).max(signature_end);
        let mut first = i;
        while first > 0 {
            let above = lines[first - 1].trim();
            if above.starts_with("///")
                || above.starts_with("//!")
                || above.starts_with("#[")
                || above.starts_with('@')
                || above.starts_with("/**")
                || above.starts_with('*')
                || above.starts_with("\"\"\"")
            {
                first -= 1;
            } else {
                break;
            }
        }
        let one_line = signature
            .split(['{', '='])
            .next()
            .unwrap_or("")
            .split("=>")
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(':')
            .trim()
            .to_string();
        let (params, returns) = inputs_outputs(language, &signature);
        out.push(Function {
            name,
            language: language.to_string(),
            signature: one_line,
            params,
            returns,
            code: lines[first..=end].join("\n"),
        });
        i = end + 1;
    }
    out
}

/// Everything reusable in one kept message: its functions, or each whole block
/// (named by the message) when a block declares no function.
pub fn reusable(title: &str, text: &str) -> Vec<Function> {
    let mut out = vec![];
    for (language, block) in fenced_blocks(text) {
        let found = functions(&language, &block);
        if !found.is_empty() {
            out.extend(found);
        } else if block.lines().filter(|l| !l.trim().is_empty()).count() >= 2 {
            out.push(Function {
                name: title.trim().chars().take(80).collect(),
                language: canonical(&language).to_string(),
                signature: String::new(),
                params: vec![],
                returns: String::new(),
                code: block,
            });
        }
    }
    out
}

/// Stable across rebuilds: the same function in the same message keeps its id,
/// so the user's note stays with it.
fn id_of(memo: &str, f: &Function) -> String {
    let digest = Sha256::digest(format!(
        "{memo}\n{}\n{}\n{}",
        f.language, f.name, f.signature
    ));
    format!("{digest:x}")[..24].to_string()
}

#[derive(Deserialize, Default)]
pub struct Search {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub offset: usize,
}

/// Every function in the kept code, newest message first, searchable by name,
/// signature, note, code and the message's title or tags.
pub async fn list(PrivateState(s): PrivateState, Query(q): Query<Search>) -> Api<Json<Value>> {
    let conn = s.db.0.lock().unwrap();
    let owner = s.db.owner();
    let sources: Vec<(String, String, String, String, String)> = conn
        .prepare("SELECT m.id,m.title,m.input,m.answer,m.created_at FROM wiki_sources w JOIN memos m ON m.id=w.memo_id WHERE w.owner=?1 AND m.owner_wallet=?1 AND m.status='complete' AND (instr(m.input,'```')>0 OR instr(m.answer,'```')>0) ORDER BY m.created_at DESC,m.id DESC LIMIT ?2")?
        .query_map(params![owner, SOURCES as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let notes: std::collections::HashMap<String, String> = conn
        .prepare("SELECT id,description FROM code_notes WHERE owner=?1")?
        .query_map([owner], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let needle = q.q.trim().to_lowercase();
    let wanted = canonical(&q.lang.trim().to_lowercase());
    let mut items = vec![];
    let mut languages: std::collections::BTreeMap<String, u64> = Default::default();
    for (memo, title, input, answer, created_at) in &sources {
        let tags = wiki::tags_of(&conn, owner, memo)?;
        let text = format!("{input}\n\n{answer}");
        // A tidied answer often repeats the code it was asked about: one card.
        let mut seen = std::collections::HashSet::new();
        for f in reusable(title, &text) {
            let id = id_of(memo, &f);
            if !seen.insert(id.clone()) {
                continue;
            }
            let description = notes.get(&id).cloned().unwrap_or_default();
            *languages.entry(f.language.clone()).or_default() += 1;
            if !q.lang.trim().is_empty() && f.language != wanted {
                continue;
            }
            if !needle.is_empty() {
                let haystack = format!(
                    "{} {} {} {} {} {}",
                    f.name,
                    f.signature,
                    description,
                    f.code,
                    title,
                    tags.iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .to_lowercase();
                if !needle.split_whitespace().all(|w| haystack.contains(w)) {
                    continue;
                }
            }
            items.push(json!({
                "id": id,
                "memo_id": memo,
                "memo_title": title,
                "memo_url": format!("/wiki/memo/{memo}"),
                "created_at": created_at,
                "language": f.language,
                "name": f.name,
                "signature": f.signature,
                "params": f.params.iter().map(|(n,t)| json!({"name":n,"type":t})).collect::<Vec<_>>(),
                "returns": f.returns,
                "code": f.code,
                "description": description,
                "tags": tags,
            }));
        }
    }
    let total = items.len();
    let page: Vec<Value> = items.into_iter().skip(q.offset).take(PAGE).collect();
    Ok(Json(json!({
        "items": page,
        "total": total,
        "offset": q.offset,
        "page_size": PAGE,
        "languages": languages.iter().map(|(l,c)| json!({"language":l,"count":c})).collect::<Vec<_>>(),
        "messages": sources.len(),
    })))
}

/// The user's own note on a function: what it is for, how to call it. Kept
/// apart from the code, which stays as the message wrote it.
pub async fn update(
    PrivateState(s): PrivateState,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> Api<Json<Value>> {
    if id.len() != 24 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError(StatusCode::NOT_FOUND, "Function not found".into()));
    }
    let description: String = v["description"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .take(2000)
        .collect();
    let conn = s.db.0.lock().unwrap();
    s.db.writable(&conn)?;
    let owner = s.db.owner();
    if description.is_empty() {
        conn.execute(
            "DELETE FROM code_notes WHERE owner=?1 AND id=?2",
            params![owner, id],
        )?;
    } else {
        conn.execute(
            "INSERT INTO code_notes(id,owner,description,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(owner,id) DO UPDATE SET description=excluded.description,updated_at=excluded.updated_at",
            params![id, owner, description, db::now()],
        )?;
    }
    let saved: Option<String> = conn
        .query_row(
            "SELECT description FROM code_notes WHERE owner=?1 AND id=?2",
            params![owner, id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Json(
        json!({"id":id,"description":saved.unwrap_or_default()}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_functions_carry_input_output_and_docs() {
        let text = "Here is the helper.\n\n```rust\nuse std::fmt;\n\n/// Adds two numbers.\n#[inline]\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nimpl Point {\n    pub async fn scale(&mut self, by: f32, out: &mut Vec<(f32, f32)>) -> Result<(), String> {\n        if by > 0.0 { Ok(()) } else { Err(\"no\".into()) }\n    }\n}\n```\n";
        let found = reusable("Helpers", text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "add");
        assert_eq!(found[0].language, "rust");
        assert_eq!(found[0].signature, "pub fn add(a: i32, b: i32) -> i32");
        assert_eq!(
            found[0].params,
            vec![("a".into(), "i32".into()), ("b".into(), "i32".into())]
        );
        assert_eq!(found[0].returns, "i32");
        assert!(
            found[0]
                .code
                .starts_with("/// Adds two numbers.\n#[inline]\npub fn add")
        );
        assert!(found[0].code.trim_end().ends_with('}'));
        assert_eq!(found[1].name, "scale");
        assert_eq!(
            found[1].params,
            vec![
                ("by".into(), "f32".into()),
                ("out".into(), "&mut Vec<(f32, f32)>".into())
            ]
        );
        assert_eq!(found[1].returns, "Result<(), String>");
        assert_eq!(found[1].code.lines().count(), 3);
    }

    #[test]
    fn python_javascript_and_go_split_by_signature() {
        let text = "```python\nimport os\n\n@cache\ndef greet(name: str, times: int = 1) -> str:\n    for _ in range(times):\n        print(name)\n    return name\n\nclass A:\n    def method(self, x):\n        return x\n```\n```js\nexport async function fetchAll(urls, {retries = 2} = {}) {\n  return Promise.all(urls.map(fetch));\n}\nconst double = (n) => n * 2;\n```\n```go\nfunc (s *Server) Handle(w http.ResponseWriter, r *http.Request) error {\n\treturn nil\n}\n```";
        let found = reusable("Snippets", text);
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["greet", "method", "fetchAll", "double", "Handle"]);
        assert_eq!(
            found[0].params,
            vec![
                ("name".into(), "str".into()),
                ("times".into(), "int".into())
            ]
        );
        assert_eq!(found[0].returns, "str");
        assert!(found[0].code.starts_with("@cache\ndef greet"));
        assert_eq!(found[0].code.lines().count(), 5);
        assert_eq!(found[1].params, vec![("x".into(), String::new())]);
        assert_eq!(found[2].language, "javascript");
        assert_eq!(found[2].params.len(), 2);
        assert_eq!(found[3].signature, "const double");
        assert_eq!(found[3].code, "const double = (n) => n * 2;");
        assert_eq!(found[4].language, "go");
        assert_eq!(
            found[4].params,
            vec![
                ("w".into(), "http.ResponseWriter".into()),
                ("r".into(), "*http.Request".into())
            ]
        );
        assert_eq!(found[4].returns, "error");
    }

    #[test]
    fn a_block_without_functions_is_kept_whole_and_one_liners_are_not() {
        let text = "```sh\nbrew install ffmpeg\nffmpeg -i in.mov out.mp4\n```\n```\nls\n```";
        let found = reusable("Convert a clip", text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Convert a clip");
        assert_eq!(found[0].language, "shell");
        assert!(found[0].params.is_empty());
        assert_eq!(
            found[0].code,
            "brew install ffmpeg\nffmpeg -i in.mov out.mp4"
        );
    }

    #[test]
    fn visibility_and_wrapped_signatures_keep_their_bodies() {
        let text = "```rust\npub(crate) fn tags_of(\n    conn: &Connection,\n    owner: &str,\n    memo: &str,\n    limit: usize,\n    deep: bool,\n) -> Result<Vec<String>> {\n    let rows = vec![];\n    Ok(rows)\n}\n\npub(super) async fn tick() {\n    sleep();\n}\n```";
        let found = reusable("db", text);
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["tags_of", "tick"]);
        assert_eq!(found[0].params.len(), 5);
        assert_eq!(found[0].returns, "Result<Vec<String>>");
        assert_eq!(found[0].code.lines().count(), 10);
        assert!(found[0].code.ends_with("Ok(rows)\n}"));
        assert_eq!(
            found[1].code,
            "pub(super) async fn tick() {\n    sleep();\n}"
        );
    }

    #[test]
    fn ids_follow_the_function_not_the_body() {
        let a = reusable("t", "```rust\nfn f(x: u8) -> u8 { x }\n```").remove(0);
        let b = reusable("t", "```rust\nfn f(x: u8) -> u8 { x + 1 }\n```").remove(0);
        assert_eq!(id_of("m", &a), id_of("m", &b));
        assert_ne!(id_of("m", &a), id_of("n", &a));
        assert_eq!(id_of("m", &a).len(), 24);
    }
}
