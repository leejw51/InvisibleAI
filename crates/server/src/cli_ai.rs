//! Codex and Claude Code as providers: the backend runs the user's own `codex` and
//! `claude` command-line tools, signed in to their subscriptions, so no API key is
//! billed. Each request is one non-interactive subprocess in an empty scratch
//! directory with every tool disabled; the prompt goes in on stdin, the answer
//! comes back as JSON. Prompts still leave the Mac for OpenAI or Anthropic.
use crate::providers::{Config, Message, Turn};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};
use tokio::io::AsyncWriteExt;

pub const CODEX: &str = "codex";
pub const CLAUDE_CODE: &str = "claude_code";

pub fn is_cli(provider: &str) -> bool {
    matches!(provider, CODEX | CLAUDE_CODE)
}

/// Executable name and the environment variable that overrides its location.
fn binary(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        CODEX => Some(("codex", "CODEX_BIN")),
        CLAUDE_CODE => Some(("claude", "CLAUDE_BIN")),
        _ => None,
    }
}

/// Where the CLI lives: the override variable, PATH, then the usual install
/// locations. The backend is started detached, so its PATH may lack `~/.local/bin`.
fn locate(name: &str, env_key: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var(env_key)
        && !path.is_empty()
    {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Resolved once per process; a tool installed later needs a backend restart.
pub fn executable(provider: &str) -> Option<&'static Path> {
    static CODEX_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    static CLAUDE_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    let (name, key) = binary(provider)?;
    let cell = if provider == CODEX {
        &CODEX_PATH
    } else {
        &CLAUDE_PATH
    };
    cell.get_or_init(|| locate(name, key)).as_deref()
}

pub fn available(provider: &str) -> bool {
    executable(provider).is_some()
}

/// One prompt for a tool that takes a single message: the system text, the earlier
/// turns as a transcript, then the latest message. Images cannot be passed and are
/// described as omitted.
pub fn flatten(system: &str, messages: &[Message]) -> String {
    let mut prompt = String::new();
    if !system.trim().is_empty() {
        prompt.push_str("Instructions:\n");
        prompt.push_str(system.trim());
        prompt.push_str("\n\n");
    }
    let (history, last) = match messages.split_last() {
        Some((last, history)) => (history, Some(last)),
        None => (messages, None),
    };
    if !history.is_empty() {
        prompt.push_str("Earlier conversation:\n");
        for m in history {
            match m {
                Message::User(text) => {
                    prompt.push_str("User: ");
                    prompt.push_str(text.trim());
                }
                Message::Image { text, images } => {
                    prompt.push_str("User: ");
                    prompt.push_str(text.trim());
                    prompt.push_str(&format!(" [{} image(s) omitted]", images.len()));
                }
                Message::Assistant { text, .. } => {
                    prompt.push_str("Assistant: ");
                    prompt.push_str(text.trim());
                }
                Message::Tool { name, output, .. } => {
                    prompt.push_str(&format!("Tool {name}: "));
                    prompt.push_str(output.trim());
                }
            }
            prompt.push_str("\n\n");
        }
    }
    match last {
        Some(Message::User(text)) => prompt.push_str(text.trim()),
        Some(Message::Image { text, images }) => {
            prompt.push_str(text.trim());
            prompt.push_str(&format!(
                "\n[{} image(s) could not be attached to this tool]",
                images.len()
            ));
        }
        Some(Message::Assistant { text, .. }) => prompt.push_str(text.trim()),
        Some(Message::Tool { output, .. }) => prompt.push_str(output.trim()),
        None => {}
    }
    prompt
}

/// Command line for one request. `out` is where Codex writes its final message.
pub fn arguments(
    provider: &str,
    model: &str,
    system: &str,
    workdir: &Path,
    out: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = match provider {
        CLAUDE_CODE => {
            let mut a = vec![
                "-p".into(),
                "--output-format".into(),
                "json".into(),
                "--tools".into(),
                String::new(),
                "--no-session-persistence".into(),
                "--strict-mcp-config".into(),
                "--setting-sources".into(),
                String::new(),
            ];
            if !system.trim().is_empty() {
                a.push("--system-prompt".into());
                a.push(system.trim().into());
            }
            a
        }
        _ => vec![
            "exec".into(),
            "--skip-git-repo-check".into(),
            "--ephemeral".into(),
            "--ignore-user-config".into(),
            "--ignore-rules".into(),
            "--color".into(),
            "never".into(),
            "-s".into(),
            "read-only".into(),
            "-C".into(),
            workdir.to_string_lossy().into_owned(),
            "-o".into(),
            out.to_string_lossy().into_owned(),
            "--json".into(),
        ],
    };
    if !model.trim().is_empty() {
        args.push(
            if provider == CLAUDE_CODE {
                "--model"
            } else {
                "-m"
            }
            .into(),
        );
        args.push(model.trim().into());
    }
    if provider != CLAUDE_CODE {
        args.push("-".into());
    }
    args
}

/// Variables the child must not see: API keys would bill the API instead of the
/// subscription, and a nested Claude Code session refuses to start.
pub fn stripped_env(provider: &str) -> Vec<&'static str> {
    match provider {
        CLAUDE_CODE => vec!["ANTHROPIC_API_KEY", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"],
        _ => vec!["OPENAI_API_KEY", "CODEX_API_KEY"],
    }
}

/// The most useful line from a failed run: a JSON `error`/`turn.failed` message
/// (for example Codex's usage-limit notice), a JSON `result` flagged as an error,
/// else the last non-empty line of stderr or stdout, bounded.
pub fn failure_message(stdout: &str, stderr: &str) -> String {
    for line in stdout.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let message = v["error"]["message"]
            .as_str()
            .or_else(|| v["message"].as_str().filter(|_| v["type"] == "error"))
            .or_else(|| v["result"].as_str().filter(|_| v["is_error"] == true));
        if let Some(message) = message {
            return message.chars().take(300).collect();
        }
    }
    stderr
        .lines()
        .chain(stdout.lines())
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("no output")
        .chars()
        .take(300)
        .collect()
}

/// Claude's `--output-format json` document.
pub fn parse_claude(output: &str) -> Result<Turn> {
    let v: Value = serde_json::from_str(output.trim())
        .context("Claude Code returned no JSON result; check that `claude` is signed in")?;
    ensure!(
        v["is_error"] != true,
        "Claude Code reported an error: {}",
        v["result"].as_str().unwrap_or("unknown")
    );
    let text = v["result"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Claude Code returned an empty answer"))?;
    Ok(Turn {
        text: text.trim().into(),
        input_tokens: v["usage"]["input_tokens"].as_u64(),
        output_tokens: v["usage"]["output_tokens"].as_u64(),
        calls: vec![],
    })
}

/// Codex: the answer comes from the `-o` file; tokens are best effort from the
/// JSONL event stream, whose exact shape varies between versions.
pub fn parse_codex(last_message: &str, events: &str) -> Result<Turn> {
    let text = last_message.trim();
    let mut input = None;
    let mut output = None;
    for line in events.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for usage in [
            &v["usage"],
            &v["info"]["total_token_usage"],
            &v["msg"]["info"]["total_token_usage"],
            &v["payload"]["usage"],
        ] {
            if let Some(n) = usage["input_tokens"].as_u64() {
                input = Some(n);
            }
            if let Some(n) = usage["output_tokens"].as_u64() {
                output = Some(n);
            }
        }
        if text.is_empty()
            && v["type"]
                .as_str()
                .is_some_and(|t| t.ends_with("item.completed"))
            && v["item"]["type"] == "agent_message"
            && let Some(t) = v["item"]["text"].as_str()
        {
            return Ok(Turn {
                text: t.trim().into(),
                input_tokens: input,
                output_tokens: output,
                calls: vec![],
            });
        }
    }
    ensure!(
        !text.is_empty(),
        "Codex returned no answer; check that `codex` is signed in (run `codex login` on the backend Mac)"
    );
    Ok(Turn {
        text: text.into(),
        input_tokens: input,
        output_tokens: output,
        calls: vec![],
    })
}

/// Run one request. The whole answer is delivered once, since neither tool streams
/// plain text; the usage ledger still measures the wall-clock wait.
pub async fn generate(
    config: &Config,
    system: &str,
    messages: &[Message],
    mut on_text: impl FnMut(&str) -> Result<()>,
) -> Result<Turn> {
    let provider = config.provider.as_str();
    let (name, _) = binary(provider).ok_or_else(|| anyhow::anyhow!("Unknown provider"))?;
    let exe = executable(provider).ok_or_else(|| {
        anyhow::anyhow!(
            "The `{name}` command was not found on the backend Mac. Install it, sign in, and restart the backend (or set {}).",
            binary(provider).map(|b| b.1).unwrap_or("")
        )
    })?;
    let prompt = flatten(system, messages);
    ensure!(prompt.len() <= 400_000, "Prompt is too large for the CLI");
    let workdir = tempfile::tempdir().context("Could not create a scratch directory")?;
    let out = workdir.path().join("answer.md");
    let args = arguments(provider, &config.model, system, workdir.path(), &out);
    let mut command = tokio::process::Command::new(exe);
    command
        .args(&args)
        .env_clear()
        .envs(
            crate::coding::CLI_PASSTHROUGH
                .iter()
                .chain([&"PATH", &"TMPDIR"])
                .filter_map(|key| std::env::var_os(key).map(|value| (*key, value))),
        )
        .current_dir(workdir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in stripped_env(provider) {
        command.env_remove(key);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Could not start `{name}`"))?;
    let mut stdin = child.stdin.take().context("No stdin for the CLI")?;
    let input = if provider == CLAUDE_CODE {
        prompt
    } else {
        // Codex reads the instructions from stdin when the prompt argument is `-`.
        prompt
    };
    tokio::spawn(async move {
        let _ = stdin.write_all(input.as_bytes()).await;
        let _ = stdin.shutdown().await;
    });
    let output = tokio::time::timeout(Duration::from_secs(300), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("`{name}` did not answer within 300 seconds"))??;
    ensure!(
        output.stdout.len() <= 4_000_000,
        "The CLI produced too much output"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`{name}` could not answer: {}",
            failure_message(&stdout, &stderr)
        );
    }
    let turn = if provider == CLAUDE_CODE {
        parse_claude(&stdout)?
    } else {
        let last = tokio::fs::read_to_string(&out).await.unwrap_or_default();
        parse_codex(&last, &stdout)?
    };
    on_text(&turn.text)?;
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_disable_tools_and_keep_keys_out() {
        let dir = Path::new("/tmp/scratch");
        let out = Path::new("/tmp/scratch/answer.md");
        let claude = arguments(CLAUDE_CODE, "", "Be brief.", dir, out);
        assert!(claude.windows(2).any(|w| w == ["--tools", ""]));
        assert!(
            claude
                .windows(2)
                .any(|w| w == ["--system-prompt", "Be brief."])
        );
        assert!(claude.contains(&"--no-session-persistence".to_string()));
        assert!(!claude.contains(&"--model".to_string()));
        let claude = arguments(CLAUDE_CODE, "sonnet", "", dir, out);
        assert!(claude.windows(2).any(|w| w == ["--model", "sonnet"]));
        let codex = arguments(CODEX, "gpt-5-codex", "", dir, out);
        assert_eq!(codex[0], "exec");
        assert!(codex.windows(2).any(|w| w == ["-s", "read-only"]));
        assert!(codex.windows(2).any(|w| w == ["-C", "/tmp/scratch"]));
        assert!(codex.windows(2).any(|w| w == ["-m", "gpt-5-codex"]));
        assert_eq!(codex.last().map(String::as_str), Some("-"));
        assert!(stripped_env(CLAUDE_CODE).contains(&"ANTHROPIC_API_KEY"));
        assert!(stripped_env(CODEX).contains(&"OPENAI_API_KEY"));
    }

    #[test]
    fn prompts_flatten_history_and_mark_images() {
        let prompt = flatten(
            "System text",
            &[
                Message::User("Hi".into()),
                Message::Assistant {
                    text: "Hello".into(),
                    calls: vec![],
                },
                Message::Image {
                    text: "What is on screen?".into(),
                    images: vec![("image/png".into(), "AAAA".into())],
                },
            ],
        );
        assert!(prompt.starts_with(
            "Instructions:\nSystem text\n\nEarlier conversation:\nUser: Hi\n\nAssistant: Hello\n\n"
        ));
        assert!(
            prompt.ends_with("What is on screen?\n[1 image(s) could not be attached to this tool]")
        );
        assert!(!prompt.contains("AAAA"));
    }

    #[test]
    fn failures_surface_the_tool_message() {
        let events = "{\"type\":\"thread.started\"}\n{\"type\":\"error\",\"message\":\"You’ve hit your usage limit. Try again at Oct 13th.\"}\n{\"type\":\"turn.failed\",\"error\":{\"message\":\"You’ve hit your usage limit. Try again at Oct 13th.\"}}\n";
        assert_eq!(
            failure_message(events, ""),
            "You’ve hit your usage limit. Try again at Oct 13th."
        );
        assert_eq!(
            failure_message("{\"is_error\":true,\"result\":\"Not logged in\"}", ""),
            "Not logged in"
        );
        assert_eq!(failure_message("", "boom\n"), "boom");
        assert_eq!(failure_message("", ""), "no output");
    }

    #[test]
    fn outputs_are_parsed_with_tokens() {
        let turn = parse_claude(
            r#"{"type":"result","is_error":false,"result":"  42  ","usage":{"input_tokens":12,"output_tokens":3},"total_cost_usd":0.001}"#,
        )
        .unwrap();
        assert_eq!(
            (turn.text.as_str(), turn.input_tokens, turn.output_tokens),
            ("42", Some(12), Some(3))
        );
        assert!(parse_claude(r#"{"is_error":true,"result":"Not logged in"}"#).is_err());
        assert!(parse_claude("Please run /login").is_err());
        let events = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"From events\"}}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"output_tokens\":20}}\n";
        let turn = parse_codex("Final answer\n", events).unwrap();
        assert_eq!(
            (turn.text.as_str(), turn.input_tokens, turn.output_tokens),
            ("Final answer", Some(100), Some(20))
        );
        let turn = parse_codex("", events).unwrap();
        assert_eq!(turn.text, "From events");
        assert!(parse_codex("", "not json\n").is_err());
    }
}
