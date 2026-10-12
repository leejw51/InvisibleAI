use anyhow::{Result, bail, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const PROVIDERS: &[(&str, &str, &str, &str)] = &[
    (
        "anthropic",
        "Anthropic",
        "claude-sonnet-4-5",
        "ANTHROPIC_API_KEY",
    ),
    ("openai", "OpenAI", "gpt-4.1-mini", "OPENAI_API_KEY"),
    ("grok", "Grok", "grok-4", "XAI_API_KEY"),
    ("gemini", "Gemini", "gemini-2.5-flash", "GEMINI_API_KEY"),
    ("apple", "Apple local", "system", ""),
    ("mlx", "MLX local", "mlx-community/Qwen3.5-9B-4bit", ""),
    // Command-line tools signed in on the backend Mac; an empty model means the CLI default.
    ("codex", "Codex CLI", "", ""),
    ("claude_code", "Claude Code", "", ""),
];

/// Where a job reports preparation steps (download, queue, model load) so
/// the user watching the chat sees them as they happen.
pub type ProgressSink = std::sync::Arc<dyn Fn(causewaybay_shared::ModelProgress) + Send + Sync>;

#[derive(Clone, Default)]
pub struct Config {
    pub owner: String,
    pub provider: String,
    pub model: String,
    pub key: String,
    pub local: causewaybay_shared::LocalAiSettings,
    pub runtime: crate::local_ai::Runtime,
    pub progress: Option<ProgressSink>,
    /// Output cap for the on-device models (default 2048); the coding agent
    /// raises it so a whole source file fits in one answer.
    pub max_tokens: Option<u32>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
#[derive(Clone, Debug)]
pub enum Message {
    User(String),
    Image {
        text: String,
        images: Vec<(String, String)>,
    },
    Assistant {
        text: String,
        calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        name: String,
        output: String,
    },
}
#[derive(Default, Debug)]
pub struct Turn {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub text: String,
    pub calls: Vec<ToolCall>,
}

pub fn request_body(config: &Config, system: &str, messages: &[Message], tools: &[Value]) -> Value {
    match config.provider.as_str() {
        "anthropic" => {
            let mut rows = Vec::<Value>::new();
            for m in messages {
                let (role, content) = match m {
                    Message::User(t) => ("user", vec![json!({"type":"text","text":t})]),
                    Message::Image { text, images } => {
                        let mut content = images.iter().map(|(mime, data)|
                            json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}})
                        ).collect::<Vec<_>>();
                        content.push(json!({"type":"text","text":text}));
                        ("user", content)
                    }
                    Message::Assistant { text, calls } => {
                        let mut content = vec![];
                        if !text.is_empty() {
                            content.push(json!({"type":"text","text":text}));
                        }
                        for c in calls {
                            content.push(json!({"type":"tool_use","id":c.id,"name":c.name,"input":serde_json::from_str::<Value>(&c.arguments).unwrap_or(json!({}))}));
                        }
                        ("assistant", content)
                    }
                    Message::Tool { id, output, .. } => (
                        "user",
                        vec![json!({"type":"tool_result","tool_use_id":id,"content":output})],
                    ),
                };
                if rows.last().is_some_and(|r| r["role"] == role) {
                    rows.last_mut().unwrap()["content"]
                        .as_array_mut()
                        .unwrap()
                        .extend(content);
                } else {
                    rows.push(json!({"role":role,"content":content}));
                }
            }
            let mut body = json!({"model":config.model,"system":system,"messages":rows,"max_tokens":8192,"stream":true});
            if !tools.is_empty() {
                body["tools"]=json!(tools.iter().map(|t|json!({"name":t["name"],"description":t["description"],"input_schema":t["parameters"]})).collect::<Vec<_>>());
            }
            body
        }
        "gemini" => {
            let mut rows = vec![];
            for m in messages {
                rows.push(match m {
                    Message::User(t)=>json!({"role":"user","parts":[{"text":t}]}),
                    Message::Image{text,images}=>{
                        let mut parts=images.iter().map(|(mime,data)|json!({"inlineData":{"mimeType":mime,"data":data}})).collect::<Vec<_>>();
                        parts.push(json!({"text":text}));
                        json!({"role":"user","parts":parts})
                    },
                    Message::Assistant{text,calls}=>{
                        let mut parts=vec![];if !text.is_empty(){parts.push(json!({"text":text}));}
                        for c in calls{parts.push(json!({"functionCall":{"name":c.name,"args":serde_json::from_str::<Value>(&c.arguments).unwrap_or(json!({}))}}));}
                        json!({"role":"model","parts":parts})
                    },
                    Message::Tool{name,output,..}=>json!({"role":"user","parts":[{"functionResponse":{"name":name,"response":{"result":output}}}]}),
                });
            }
            let mut body = json!({"systemInstruction":{"parts":[{"text":system}]},"contents":rows,"generationConfig":{"maxOutputTokens":8192}});
            if !tools.is_empty() {
                // Gemini rejects an OBJECT schema with no properties, so
                // parameterless tools are declared without `parameters`.
                let declarations = tools
                    .iter()
                    .map(|t| {
                        let mut t = t.clone();
                        if t["parameters"]["properties"]
                            .as_object()
                            .is_some_and(|p| p.is_empty())
                        {
                            t.as_object_mut().unwrap().remove("parameters");
                        }
                        t
                    })
                    .collect::<Vec<_>>();
                body["tools"] = json!([{"functionDeclarations":declarations}]);
            }
            body
        }
        _ => {
            let mut rows = vec![json!({"role":"system","content":system})];
            for m in messages {
                rows.push(match m {
                Message::User(t)=>json!({"role":"user","content":t}),
                Message::Image{text,images}=>{
                    let mut content=images.iter().map(|(mime,data)|json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{data}")}})).collect::<Vec<_>>();
                    content.push(json!({"type":"text","text":text}));
                    json!({"role":"user","content":content})
                },
                Message::Assistant{text,calls}=>{
                    let mut row=json!({"role":"assistant","content":text});
                    if !calls.is_empty(){row["tool_calls"]=json!(calls.iter().map(|c|json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments}})).collect::<Vec<_>>());}row
                },
                Message::Tool{id,output,..}=>json!({"role":"tool","tool_call_id":id,"content":output}),
            });
            }
            let mut body = json!({"model":config.model,"messages":rows,"stream":true,"max_completion_tokens":8192});
            body["stream_options"] = json!({"include_usage":true});
            if config.provider == "mlx" {
                body.as_object_mut()
                    .unwrap()
                    .remove("max_completion_tokens");
                body["max_tokens"] = json!(config.max_tokens.unwrap_or(2048));
            }
            if !tools.is_empty() {
                body["tools"] = json!(
                    tools
                        .iter()
                        .map(|t| json!({"type":"function","function":t}))
                        .collect::<Vec<_>>()
                );
            }
            body
        }
    }
}

/// Byte-oriented SSE decoder preserves UTF-8 split across network chunks.
#[derive(Default)]
pub struct SseDecoder {
    bytes: Vec<u8>,
    data: Vec<String>,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        self.bytes.extend_from_slice(bytes);
        ensure!(
            self.bytes.len() < 2_000_000,
            "Provider event exceeded the size limit"
        );
        let mut out = vec![];
        while let Some(pos) = self.bytes.iter().position(|b| *b == b'\n') {
            let line = self.bytes.drain(..=pos).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line)?.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_string());
            }
        }
        Ok(out)
    }
}

#[derive(Default)]
pub struct Accumulator {
    pub turn: Turn,
    calls: BTreeMap<usize, ToolCall>,
    pub finished: bool,
}
impl Accumulator {
    pub fn accept(&mut self, provider: &str, data: &str) -> Result<String> {
        if data == "[DONE]" {
            self.finished = true;
            return Ok(String::new());
        }
        let v: Value = serde_json::from_str(data)?;
        let usage = if provider == "anthropic" && v["type"] == "message_start" {
            &v["message"]["usage"]
        } else {
            &v["usage"]
        };
        let input = usage["prompt_tokens"]
            .as_u64()
            .or_else(|| usage["input_tokens"].as_u64())
            .or_else(|| v["usageMetadata"]["promptTokenCount"].as_u64());
        let output = usage["completion_tokens"]
            .as_u64()
            .or_else(|| usage["output_tokens"].as_u64())
            .or_else(|| v["usageMetadata"]["candidatesTokenCount"].as_u64());
        if input.is_some() {
            self.turn.input_tokens = input;
        }
        if output.is_some() {
            self.turn.output_tokens = output;
        }

        if v.get("error").is_some() {
            bail!(
                "The provider reported an error while streaming. Retry or check your provider account."
            );
        }
        let mut text = String::new();
        match provider {
            "anthropic" => match v["type"].as_str().unwrap_or("") {
                "content_block_start" => {
                    let block = &v["content_block"];
                    let idx = v["index"].as_u64().unwrap_or(0) as usize;
                    if block["type"] == "tool_use" {
                        self.calls.insert(
                            idx,
                            ToolCall {
                                id: block["id"].as_str().unwrap_or("").into(),
                                name: block["name"].as_str().unwrap_or("").into(),
                                arguments: String::new(),
                            },
                        );
                    }
                    if block["type"] == "text" {
                        text.push_str(block["text"].as_str().unwrap_or(""));
                    }
                }
                "content_block_delta" => {
                    let delta = &v["delta"];
                    text.push_str(delta["text"].as_str().unwrap_or(""));
                    if let Some(part) = delta["partial_json"].as_str() {
                        self.calls
                            .entry(v["index"].as_u64().unwrap_or(0) as usize)
                            .or_default()
                            .arguments
                            .push_str(part);
                    }
                }
                "message_delta" => {
                    if v["delta"]["stop_reason"] == "max_tokens" {
                        bail!(
                            "Response reached the provider's token limit. Ask to continue from the saved partial answer."
                        );
                    }
                }
                "message_stop" => self.finished = true,
                _ => {}
            },
            "gemini" => {
                if v["promptFeedback"]["blockReason"].is_string() {
                    bail!("Gemini blocked this prompt. Try rephrasing it.");
                }
                if let Some(parts) = v["candidates"][0]["content"]["parts"].as_array() {
                    for p in parts {
                        if p["thought"] != true {
                            text.push_str(p["text"].as_str().unwrap_or(""));
                        }
                        if let Some(c) = p.get("functionCall") {
                            let idx = self.calls.len();
                            self.calls.insert(
                                idx,
                                ToolCall {
                                    id: format!("call_{idx}"),
                                    name: c["name"].as_str().unwrap_or("").into(),
                                    arguments: c["args"].to_string(),
                                },
                            );
                        }
                    }
                }
                if let Some(reason) = v["candidates"][0]["finishReason"].as_str() {
                    if reason != "STOP" {
                        bail!(
                            "Gemini stopped the response ({reason}). Your partial answer was saved."
                        );
                    }
                    self.finished = true;
                }
            }
            _ => {
                let delta = &v["choices"][0]["delta"];
                text.push_str(delta["content"].as_str().unwrap_or(""));
                if let Some(calls) = delta["tool_calls"].as_array() {
                    for c in calls {
                        let call = self
                            .calls
                            .entry(c["index"].as_u64().unwrap_or(0) as usize)
                            .or_default();
                        if let Some(id) = c["id"].as_str() {
                            call.id.push_str(id);
                        }
                        if let Some(name) = c["function"]["name"].as_str() {
                            call.name.push_str(name);
                        }
                        if let Some(args) = c["function"]["arguments"].as_str() {
                            call.arguments.push_str(args);
                        }
                    }
                }
                if let Some(reason) = v["choices"][0]["finish_reason"].as_str() {
                    if reason == "length" || reason == "content_filter" {
                        bail!(
                            "The provider stopped the response ({reason}). Your partial answer was saved."
                        );
                    }
                    self.finished = true;
                }
            }
        }
        self.turn.text.push_str(&text);
        ensure!(
            self.turn.text.len() < 1_000_000,
            "Answer exceeded the size limit"
        );
        ensure!(
            self.calls.len() <= 12 && self.calls.values().all(|c| c.arguments.len() < 500_000),
            "Tool calls exceeded the size limit"
        );
        Ok(text)
    }
    pub fn finish(mut self) -> Result<Turn> {
        ensure!(
            self.finished,
            "Provider stream disconnected before completion. Your partial answer was saved."
        );
        self.turn.calls = self
            .calls
            .into_values()
            .map(|mut c| {
                if c.arguments.is_empty() {
                    c.arguments = "{}".into();
                }
                c
            })
            .collect();
        ensure!(
            !self.turn.text.is_empty() || !self.turn.calls.is_empty(),
            "The provider returned no answer"
        );
        Ok(self.turn)
    }
}

pub async fn stream(
    config: &Config,
    system: &str,
    messages: &[Message],
    tools: &[Value],
    mut on_text: impl FnMut(&str) -> Result<()>,
) -> Result<Turn> {
    let local = crate::local_ai::is_local(&config.provider);
    ensure!(
        !config.local.local_only || local,
        "Cloud inference is disabled in Local-only mode"
    );
    ensure!(
        crate::local_ai::valid_model(&config.provider, &config.model),
        "Invalid model name"
    );
    let location = if local && !(config.provider == "mlx" && config.local.mlx_engine == "service") {
        "local"
    } else if local {
        "service"
    } else {
        "cloud"
    };
    let _activity =
        config
            .runtime
            .activity
            .begin(&config.owner, &config.provider, &config.model, location);
    if config.provider == "apple" {
        ensure!(
            config.local.apple_enabled,
            "Apple local is disabled by the admin"
        );
        return crate::local_ai::generate(config, system, messages, on_text).await;
    }
    if crate::cli_ai::is_cli(&config.provider) {
        return crate::cli_ai::generate(config, system, messages, on_text).await;
    }
    if config.provider == "mlx" {
        ensure!(
            config.local.mlx_enabled,
            "MLX local is disabled by the admin"
        );
        ensure!(
            !messages.iter().any(|m| matches!(m, Message::Image { .. })),
            "MLX local currently requires OCR text mode"
        );
        if config.local.mlx_engine != "service" {
            return crate::local_ai::generate_mlx(config, system, messages, on_text).await;
        }
    }
    let builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    let builder = if local { builder.no_proxy() } else { builder };
    let client = builder
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(180))
        .build()?;
    let url = match config.provider.as_str() {
        "mlx" => crate::local_ai::mlx_endpoint(&config.local.mlx_url, "chat/completions")?,
        "anthropic" => "https://api.anthropic.com/v1/messages".into(),
        "openai" => "https://api.openai.com/v1/chat/completions".into(),
        "grok" => "https://api.x.ai/v1/chat/completions".into(),
        "gemini" => format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
            config.model
        ),
        _ => bail!("Unknown provider"),
    };
    let request = client
        .post(url)
        .json(&request_body(config, system, messages, tools));
    let request = match config.provider.as_str() {
        "anthropic" => request
            .header("x-api-key", &config.key)
            .header("anthropic-version", "2023-06-01"),
        "gemini" => request.header("x-goog-api-key", &config.key),
        "mlx" => request,
        _ => request.bearer_auth(&config.key),
    };
    let response = request.send().await.map_err(|_| {
        anyhow::anyhow!("Could not connect to the AI provider. Check your network and try again.")
    })?;
    let status = response.status();
    if !status.is_success() {
        bail!(
            "{} returned HTTP {}. Check the backend API key and quota, and the model selected in Settings.",
            config.provider,
            status.as_u16()
        );
    }
    let mut chunks = response.bytes_stream();
    let mut decoder = SseDecoder::default();
    let mut acc = Accumulator::default();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| {
            anyhow::anyhow!("Provider connection interrupted. Your partial answer was saved.")
        })?;
        for data in decoder.push(&chunk)? {
            let text = acc.accept(&config.provider, &data)?;
            if !text.is_empty() {
                on_text(&text)?;
            }
        }
    }
    acc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_handles_utf8_every_byte_and_multiline() -> Result<()> {
        let mut decoder = SseDecoder::default();
        let mut out = vec![];
        for byte in "event: message\r\ndata: 안녕\r\ndata: world\r\n\r\n".as_bytes() {
            out.extend(decoder.push(&[*byte])?);
        }
        assert_eq!(out, vec!["안녕\nworld"]);
        Ok(())
    }
    #[test]
    fn all_providers_stream_text() -> Result<()> {
        for (p, d) in [
            (
                "openai",
                r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":"stop"}]}"#,
            ),
            (
                "grok",
                r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":"stop"}]}"#,
            ),
            (
                "anthropic",
                r#"{"type":"content_block_delta","delta":{"text":"hello"}}"#,
            ),
            (
                "gemini",
                r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]},"finishReason":"STOP"}]}"#,
            ),
        ] {
            let mut a = Accumulator::default();
            assert_eq!(a.accept(p, d)?, "hello");
            if p == "anthropic" {
                a.accept(p, r#"{"type":"message_stop"}"#)?;
            }
            assert_eq!(a.finish()?.text, "hello");
        }
        Ok(())
    }
    #[test]
    fn tool_calls_assemble_and_serialize() -> Result<()> {
        let mut a = Accumulator::default();
        a.accept("openai",r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#)?;
        a.accept("openai",r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"main.rs\"}"}}]},"finish_reason":"tool_calls"}]}"#)?;
        let turn = a.finish()?;
        assert_eq!(
            serde_json::from_str::<Value>(&turn.calls[0].arguments)?["path"],
            "main.rs"
        );
        for p in ["anthropic", "openai", "grok", "gemini"] {
            let cfg = Config {
                provider: p.into(),
                model: "test".into(),
                key: "secret".into(),
                ..Default::default()
            };
            let body = request_body(
                &cfg,
                "sys",
                &[
                    Message::User("hi".into()),
                    Message::Assistant {
                        text: String::new(),
                        calls: turn.calls.clone(),
                    },
                    Message::Tool {
                        id: "c1".into(),
                        name: "read_file".into(),
                        output: "ok".into(),
                    },
                ],
                &[],
            );
            assert!(!body.to_string().contains("secret"));
            assert!(body.to_string().contains("read_file"));
        }
        Ok(())
    }
    #[test]
    fn incomplete_streams_and_provider_errors_fail() {
        assert!(Accumulator::default().finish().is_err());
        assert!(
            Accumulator::default()
                .accept(
                    "anthropic",
                    r#"{"type":"error","error":{"message":"secret"}}"#
                )
                .is_err()
        );
    }
    #[test]
    fn parameterless_tool_calls_parse_on_every_provider() -> Result<()> {
        let mut a = Accumulator::default();
        a.accept("anthropic", r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"list_files","input":{}}}"#)?;
        a.accept("anthropic", r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#)?;
        a.accept("anthropic", r#"{"type":"message_stop"}"#)?;
        let turn = a.finish()?;
        assert_eq!(turn.calls[0].arguments, "{}");
        let mut a = Accumulator::default();
        a.accept("gemini", r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"list_files"}}]},"finishReason":"STOP"}]}"#)?;
        let turn = a.finish()?;
        assert_eq!(turn.calls[0].name, "list_files");
        assert!(serde_json::from_str::<Value>(&turn.calls[0].arguments).is_ok());
        let mut a = Accumulator::default();
        a.accept("openai", r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"list_files","arguments":""}}]},"finish_reason":null}]}"#)?;
        a.accept(
            "openai",
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        )?;
        a.accept("openai", "[DONE]")?;
        let turn = a.finish()?;
        assert_eq!(turn.calls[0].arguments, "{}");
        Ok(())
    }
    #[test]
    fn gemini_declares_parameterless_tools_without_a_schema() {
        let cfg = Config {
            provider: "gemini".into(),
            model: "m".into(),
            ..Default::default()
        };
        let tools = crate::agent::definitions();
        let body = request_body(&cfg, "sys", &[Message::User("hi".into())], &tools);
        let declared = body["tools"][0]["functionDeclarations"].as_array().unwrap();
        let list = declared.iter().find(|t| t["name"] == "list_files").unwrap();
        assert!(list.get("parameters").is_none());
        let read = declared.iter().find(|t| t["name"] == "read_file").unwrap();
        assert_eq!(read["parameters"]["type"], "object");
    }
}
