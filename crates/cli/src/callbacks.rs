use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;

#[derive(Default)]
struct Decoder(Vec<u8>);
impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.0.extend_from_slice(bytes);
        ensure!(self.0.len() < 64 * 1024, "Capture callback is too large");
        let mut events = vec![];
        while let Some((end, separator)) = self
            .0
            .windows(2)
            .position(|v| v == b"\n\n")
            .map(|p| (p, 2))
            .or_else(|| {
                self.0
                    .windows(4)
                    .position(|v| v == b"\r\n\r\n")
                    .map(|p| (p, 4))
            })
        {
            let frame = String::from_utf8(self.0.drain(..end + separator).collect())?;
            let data = frame
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("data:")
                        .map(|v| v.strip_prefix(' ').unwrap_or(v))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                events.push(serde_json::from_str(&data)?);
            }
        }
        Ok(events)
    }
}
pub async fn run(
    backend: String,
    id: String,
    session: String,
    token: Option<String>,
    sender: tokio::sync::mpsc::Sender<Value>,
) {
    let mut after = 0;
    loop {
        if let Err(error) = listen(
            &backend,
            &id,
            &session,
            token.as_deref(),
            &sender,
            &mut after,
        )
        .await
        {
            eprintln!("Capture callbacks disconnected: {error}. Retrying in 2 seconds…");
        }
        if sender.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
async fn listen(
    backend: &str,
    id: &str,
    session: &str,
    token: Option<&str>,
    sender: &tokio::sync::mpsc::Sender<Value>,
    after: &mut i64,
) -> Result<()> {
    let base = backend
        .replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1);
    let mut url = reqwest::Url::parse(base.trim_end_matches('/'))?.join("/api/client/events")?;
    url.query_pairs_mut()
        .append_pair("client_id", id)
        .append_pair("after", &after.to_string());
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    let mut request = client
        .get(url)
        .header("accept", "text/event-stream")
        .header("x-capture-session", session);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    ensure!(
        response.status().is_success(),
        "Backend callback response: {}",
        response.status()
    );
    println!("SSE callbacks connected.");
    let mut bytes = response.bytes_stream();
    let mut decoder = Decoder::default();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(35), bytes.next())
        .await
        .context("Callback heartbeat timed out")?
    {
        for event in decoder.push(&chunk?)? {
            let cursor = event["id"]
                .as_i64()
                .filter(|_| event["type"] == "notification");
            sender.send(event).await?;
            if let Some(cursor) = cursor {
                *after = cursor;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_handle_fragmented_utf8_comments_and_multiple_frames() -> Result<()> {
        let mut decoder = Decoder::default();
        let mut values = vec![];
        for byte in
            b": keep-alive\n\ndata: {\"name\":\"\xe6\xb8\xaf\"}\n\nid: 2\ndata: {\"id\":2}\n\n"
        {
            values.extend(decoder.push(&[*byte])?);
        }
        assert_eq!(
            values,
            vec![
                serde_json::json!({"name":"港"}),
                serde_json::json!({"id":2})
            ]
        );
        assert_eq!(
            decoder.push(b"data: {\r\ndata: \"id\":3}\r\n\r\n")?,
            vec![serde_json::json!({"id":3})]
        );
        Ok(())
    }
}
