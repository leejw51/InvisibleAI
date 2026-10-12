//! Fixed TextEdit, Notes and Safari automation only. Screen/model text is never
//! executable script: every request is a JSON file read by a constant JXA program,
//! and Safari receives one validated address through /usr/bin/open.
use anyhow::{Result, ensure};
use causewaybay_shared::NoteError;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Apple Notes: the single selected note is the document. `body` is HTML that Notes
/// normalizes on write, so staleness is checked on `plaintext`, and the append is
/// verified by looking for the answer's words in the resulting plaintext.
const NOTES_SCRIPT: &str = r#"
ObjC.import('Foundation');
function run(argv) {
  const failure = code => JSON.stringify({error_code:code});
  try {
  const raw = $.NSString.stringWithContentsOfFileEncodingError(argv[0], $.NSUTF8StringEncoding, null);
  const request = JSON.parse(ObjC.unwrap(raw));
  const app = Application('Notes');
  if (!app.running()) return failure('no_document');
  function selected() {
    const notes = app.selection();
    if (!notes || notes.length !== 1) return null;
    const note = notes[0];
    return {note, identity:'notes:' + String(note.id())};
  }
  const current = selected();
  if (!current) return failure('no_document');
  const {note, identity} = current;
  const text = String(note.plaintext());
  if (request.document && request.document !== identity) return failure('document_changed');
  if (request.operation === 'append') {
    if (Date.now() > request.expires_at) return failure('expired');
    if (text !== request.expected) return failure('note_changed');
    const again = selected();
    if (!again || again.identity !== identity) return failure('document_changed');
    if (String(note.plaintext()) !== text) return failure('note_changed');
    note.body = String(note.body()) + request.suffix_html;
    const written = String(note.plaintext());
    const squash = s => s.replace(/\s+/g, ' ').trim();
    if (!squash(written).endsWith(squash(request.suffix))) return failure('operation_failed');
    return JSON.stringify({document:identity,text:written});
  }
  return JSON.stringify({document:identity,text:text});
  } catch (error) {
    return failure(error.errorNumber === -1743 ? 'permission_denied' :
      error.errorNumber === -1712 ? 'timed_out' :
      error.errorNumber === -1728 ? 'document_changed' : 'operation_failed');
  }
}
"#;

/// Terminal.app or iTerm2: the front window's selected tab or session is the
/// document (identity = its TTY or session id), `text` is the visible contents.
/// The script only reads and verifies; an answer is written by the CLI straight
/// to the tab's TTY device as display output. No command is ever sent to the
/// shell and nothing is typed into the prompt.
const TERMINAL_SCRIPT: &str = r#"
ObjC.import('Foundation');
function run(argv) {
  const failure = code => JSON.stringify({error_code:code});
  try {
  const raw = $.NSString.stringWithContentsOfFileEncodingError(argv[0], $.NSUTF8StringEncoding, null);
  const request = JSON.parse(ObjC.unwrap(raw));
  const terminal = Application('Terminal');
  const iterm = Application('iTerm2');
  const preferIterm = request.prefer === 'iterm' ? iterm.running() : !terminal.running();
  function selected() {
    if (preferIterm && iterm.running()) {
      const window = iterm.currentWindow();
      if (!window) return null;
      const session = window.currentSession();
      if (!session) return null;
      return {kind:'iterm', target:session, identity:'iterm:' + String(session.id()), tty:String(session.tty()), text:String(session.contents())};
    }
    if (terminal.running()) {
      const windows = terminal.windows();
      if (!windows.length) return null;
      const tab = windows[0].selectedTab();
      if (!tab) return null;
      return {kind:'terminal', target:tab, identity:'terminal:' + String(tab.tty()), tty:String(tab.tty()), text:String(tab.contents())};
    }
    return null;
  }
  const current = selected();
  if (!current) return failure('no_document');
  const {identity, tty, text} = current;
  if (request.document && request.document !== identity) return failure('document_changed');
  if (request.operation === 'append') {
    // Verify only; the CLI writes to the TTY after this script returns.
    if (Date.now() > request.expires_at) return failure('expired');
    if (text !== request.expected) return failure('note_changed');
    const again = selected();
    if (!again || again.identity !== identity) return failure('document_changed');
    if (again.text !== text) return failure('note_changed');
  }
  return JSON.stringify({document:identity,tty:tty,text:text});
  } catch (error) {
    return failure(error.errorNumber === -1743 ? 'permission_denied' :
      error.errorNumber === -1712 ? 'timed_out' :
      error.errorNumber === -1728 ? 'document_changed' : 'operation_failed');
  }
}
"#;

/// Plain answer text as Notes HTML: one `<div>` per line, entities escaped.
pub fn notes_html(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                "<div><br></div>".to_string()
            } else {
                let escaped: String = line
                    .chars()
                    .map(|c| match c {
                        '&' => "&amp;".to_string(),
                        '<' => "&lt;".to_string(),
                        '>' => "&gt;".to_string(),
                        '"' => "&quot;".to_string(),
                        c => c.to_string(),
                    })
                    .collect();
                format!("<div>{escaped}</div>")
            }
        })
        .collect()
}

/// Accept only http(s) addresses with a host; the backend validated the same way.
pub fn safe_url(url: &str) -> Result<String> {
    ensure!(url.len() <= 2_048, "Address is too long");
    ensure!(
        !url.chars().any(|c| c.is_control() || c.is_whitespace()),
        "Address must not contain spaces"
    );
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        matches!(parsed.scheme(), "http" | "https"),
        "Only http and https addresses can be opened"
    );
    ensure!(
        parsed.host_str().is_some_and(|h| !h.is_empty()),
        "Address has no host"
    );
    ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "Addresses with credentials are not opened"
    );
    Ok(parsed.to_string())
}

/// Open one validated address in Safari. No script, no keystrokes, no page content.
pub async fn open_url(v: &Value) -> Result<Value> {
    ensure!(cfg!(target_os = "macos"), "Safari requires macOS");
    let url = safe_url(v["url"].as_str().unwrap_or(""))?;
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new("/usr/bin/open")
            .args(["-a", "Safari", "--", &url])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| NoteError::TimedOut)?
    .map_err(|_| NoteError::OperationFailed)?;
    ensure!(output.status.success(), NoteError::OperationFailed);
    Ok(json!({"opened":url}))
}

const SCRIPT: &str = r#"
ObjC.import('Foundation');
function run(argv) {
  const failure = code => JSON.stringify({error_code:code});
  try {
  const raw = $.NSString.stringWithContentsOfFileEncodingError(argv[0], $.NSUTF8StringEncoding, null);
  const request = JSON.parse(ObjC.unwrap(raw));
  const app = Application('TextEdit');
  if (!app.running()) return failure('no_document');
  // Use the document belonging to the front visible window. TextEdit can also
  // have hidden utility windows; its document and window arrays need not align.
  function selected() {
    for (const window of app.windows()) {
      if (!window.visible()) continue;
      try {
        const document = window.document();
        return {document, identity:String(window.id()) + ':' + document.name()};
      } catch (error) {
        if (error.errorNumber !== -1728) throw error;
      }
    }
    return null;
  }
  const note = selected();
  if (!note) return failure('no_document');
  const {document, identity} = note;
  const text = document.text();
  if (request.document && request.document !== identity) return failure('document_changed');
  if (request.operation === 'append') {
    if (Date.now() > request.expires_at) return failure('expired');
    if (text !== request.expected) return failure('note_changed');
    // Compare again immediately before the Apple event. No activation or keystrokes.
    const current = selected();
    if (!current || current.identity !== identity) return failure('document_changed');
    if (document.text() !== text) return failure('note_changed');
    document.text = text + request.suffix;
    const written = document.text();
    if (written !== text + request.suffix) return failure('operation_failed');
    return JSON.stringify({document:identity,text:written});
  }
  return JSON.stringify({document:identity,text:text});
  } catch (error) {
    // macOS requests Automation consent on the first Apple event. A denial
    // requires a change in Settings; never echo raw osascript errors or text.
    return failure(error.errorNumber === -1743 ? 'permission_denied' :
      error.errorNumber === -1712 ? 'timed_out' :
      error.errorNumber === -1728 ? 'document_changed' : 'operation_failed');
  }
}
"#;

/// Bundle identifier of the app in front on this Mac, through AppKit; no Apple
/// event is sent and no Automation consent is involved.
async fn frontmost_bundle() -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("/usr/bin/osascript")
            .args([
                "-l",
                "JavaScript",
                "-e",
                "ObjC.import('AppKit'); function run(){ return ObjC.unwrap($.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier) || ''; }",
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !id.is_empty()).then_some(id)
}

/// `app: "auto"` follows the front app when it is TextEdit or Notes; otherwise it
/// uses whichever of the two currently has a note open, TextEdit first.
pub async fn execute(v: &Value) -> Result<Value> {
    ensure!(
        cfg!(target_os = "macos"),
        "TextEdit automation requires macOS"
    );
    let app = v["app"].as_str().unwrap_or("auto");
    ensure!(
        matches!(app, "auto" | "textedit" | "notes" | "terminal"),
        "Unknown note application"
    );
    if app != "auto" {
        return execute_in(v, app).await;
    }
    let front = frontmost_bundle().await;
    let order: [&str; 3] = match front.as_deref() {
        Some("com.apple.Notes") => ["notes", "textedit", "terminal"],
        Some("com.apple.Terminal" | "com.googlecode.iterm2") => ["terminal", "textedit", "notes"],
        _ => ["textedit", "notes", "terminal"],
    };
    let mut v = v.clone();
    if front.as_deref() == Some("com.googlecode.iterm2") {
        v["prefer"] = json!("iterm");
    }
    let mut result = execute_in(&v, order[0]).await;
    for app in &order[1..] {
        match &result {
            Err(error) if error.downcast_ref::<NoteError>() == Some(&NoteError::NoDocument) => {
                result = execute_in(&v, app).await;
            }
            _ => break,
        }
    }
    result
}

/// Show text in a terminal by writing to its TTY device as output. Only a
/// `/dev/ttys*` path reported by Terminal or iTerm2 is accepted; this is display
/// output, not input, so the shell never sees a command.
pub async fn show_in_terminal(tty: &str, suffix: &str) -> Result<()> {
    ensure!(
        tty.starts_with("/dev/ttys") && !tty.contains("..") && !tty.contains(['\'', ' ']),
        "Unexpected terminal device"
    );
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(tty)
        .await
        .map_err(|_| NoteError::PermissionDenied)?;
    // Typed like a person, one character at a time, but never slower than about
    // twelve seconds in total so the backend's operation timeout is respected.
    let chars: Vec<char> = suffix.chars().collect();
    let per_char =
        Duration::from_millis(12).min(Duration::from_millis(12_000 / chars.len().max(1) as u64));
    let mut buffer = [0u8; 4];
    for c in chars {
        let bytes: &[u8] = if c == '\n' {
            b"\r\n"
        } else {
            c.encode_utf8(&mut buffer).as_bytes()
        };
        file.write_all(bytes)
            .await
            .map_err(|_| NoteError::OperationFailed)?;
        file.flush().await.map_err(|_| NoteError::OperationFailed)?;
        if !per_char.is_zero() {
            tokio::time::sleep(per_char).await;
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod terminal_tests {
    use super::*;
    #[tokio::test]
    async fn only_pty_devices_are_written() {
        assert!(show_in_terminal("/etc/passwd", "x").await.is_err());
        assert!(show_in_terminal("/dev/ttys0/../x", "x").await.is_err());
        assert!(show_in_terminal("/dev/null", "x").await.is_err());
    }
}

async fn execute_in(v: &Value, app: &str) -> Result<Value> {
    ensure!(
        matches!(v["operation"].as_str(), Some("read" | "append")),
        "Unknown note operation"
    );
    let mut v = v.clone();
    v["app"] = json!(app);
    if v["operation"] == "append" {
        ensure!(
            v["expected"].as_str().is_some_and(|t| t.len() <= 100_000),
            "Note is too large"
        );
        ensure!(
            v["suffix"].as_str().is_some_and(|t| t.len() <= 100_000),
            "Answer is too large"
        );
        ensure!(
            v["document"].as_str().is_some_and(|t| !t.is_empty()),
            "No document selected"
        );
        if app == "notes" {
            v["suffix_html"] = json!(notes_html(v["suffix"].as_str().unwrap_or("")));
        }
        if app == "terminal" {
            // Verify through the script, then print to the TTY and read back.
            let checked = run_script(&v, app).await?;
            let tty = checked["tty"].as_str().unwrap_or("").to_string();
            show_in_terminal(&tty, v["suffix"].as_str().unwrap_or("")).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut after = v.clone();
            after["operation"] = json!("read");
            let mut snapshot = run_script(&after, app).await?;
            snapshot["app"] = json!(app);
            return Ok(json!({"snapshot":snapshot}));
        }
    }
    let mut snapshot = run_script(&v, app).await?;
    snapshot["app"] = json!(app);
    Ok(json!({"snapshot":snapshot}))
}

/// One run of the fixed script for `app`, returning the validated snapshot.
async fn run_script(v: &Value, app: &str) -> Result<Value> {
    let temp = tempfile::NamedTempFile::new()?;
    tokio::fs::write(temp.path(), serde_json::to_vec(v)?).await?;
    let output = tokio::time::timeout(
        Duration::from_secs(25),
        tokio::process::Command::new("/usr/bin/osascript")
            .args([
                "-l",
                "JavaScript",
                "-e",
                match app {
                    "notes" => NOTES_SCRIPT,
                    "terminal" => TERMINAL_SCRIPT,
                    _ => SCRIPT,
                },
            ])
            .arg(temp.path())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| NoteError::TimedOut)?
    .map_err(|_| NoteError::OperationFailed)?;
    if !output.status.success() {
        return Err(NoteError::OperationFailed.into());
    }
    ensure!(output.stdout.len() <= 250_000, "Note is too large");
    let snapshot: Value = serde_json::from_slice(&output.stdout)?;
    if let Some(code) = snapshot.get("error_code") {
        let error =
            serde_json::from_value::<NoteError>(code.clone()).unwrap_or(NoteError::OperationFailed);
        return Err(error.into());
    }
    ensure!(
        snapshot["text"]
            .as_str()
            .is_some_and(|t| t.len() <= 200_000),
        "Invalid note snapshot"
    );
    let _ = app;
    Ok(snapshot)
}

#[cfg(test)]
mod unit {
    use super::*;
    #[test]
    fn safari_addresses_are_validated_and_notes_html_is_escaped() {
        assert!(safe_url("javascript:alert(1)").is_err());
        assert!(safe_url("file:///etc/passwd").is_err());
        assert!(safe_url("https://u:p@example.com").is_err());
        assert!(safe_url("https://example.com/a b").is_err());
        assert!(safe_url("-a").is_err());
        assert_eq!(
            safe_url("https://example.com/path?q=1").unwrap(),
            "https://example.com/path?q=1"
        );
        assert_eq!(
            notes_html("a < b\n\n[End Invisible AI]"),
            "<div>a &lt; b</div><div><br></div><div>[End Invisible AI]</div>"
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Opens a disposable TextEdit document; requires macOS Automation permission"]
    async fn live_textedit_read_append_and_reject_stale_text() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let name = format!("Invisible-AI-test-{}.txt", uuid::Uuid::new_v4());
        let path = dir.path().join(&name);
        let original = "Bitcoin is interesting.\n한글 note — no explicit prompt.";
        std::fs::write(&path, original)?;
        let opening = format!(
            "tell application \"TextEdit\"\n activate\n open POSIX file \"{}\"\nend tell",
            path.display()
        );
        let opened = tokio::time::timeout(
            Duration::from_secs(25),
            tokio::process::Command::new("/usr/bin/osascript")
                .args(["-e", &opening])
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        ensure!(opened.status.success(), "Could not open test note");
        let result=async {
            let first=execute(&json!({"operation":"read","app":"auto"})).await?;
            let snapshot=&first["snapshot"];
            ensure!(snapshot["app"]=="textedit","Auto detection must pick TextEdit for a TextEdit note");
            ensure!(snapshot["document"].as_str().unwrap_or("").ends_with(&name),"Test note is not selected");
            ensure!(snapshot["text"]==original,"Unexpected fixture content");
            let request=json!({"operation":"append","document":snapshot["document"],"expected":original,"suffix":"\nInvisible AI test answer.","expires_at":i64::MAX});
            let after=execute(&request).await?;
            ensure!(after["snapshot"]["text"]==format!("{original}\nInvisible AI test answer."),"Answer missing");
            let stale=execute(&request).await.unwrap_err();
            ensure!(stale.downcast_ref::<NoteError>()==Some(&NoteError::NoteChanged),"Stale text must have its own error: {stale}");
            let mut wrong=request.clone();wrong["document"]=json!("another-document");
            let wrong=execute(&wrong).await.unwrap_err();
            ensure!(wrong.downcast_ref::<NoteError>()==Some(&NoteError::DocumentChanged),"Wrong document must have its own error: {wrong}");
            let mut expired=request.clone();expired["expires_at"]=json!(0);
            let expired=execute(&expired).await.unwrap_err();
            ensure!(expired.downcast_ref::<NoteError>()==Some(&NoteError::Expired),"Expired response must have its own error: {expired}");
            let unchanged=execute(&json!({"operation":"read","document":snapshot["document"]})).await?;
            ensure!(unchanged==after,"Rejected writes must leave the note unchanged");
            Ok(())
        }.await;
        let closing = format!(
            "tell application \"TextEdit\" to close (first document whose name is \"{name}\") saving no"
        );
        let _ = tokio::process::Command::new("/usr/bin/osascript")
            .args(["-e", &closing])
            .output()
            .await;
        result
    }
}
