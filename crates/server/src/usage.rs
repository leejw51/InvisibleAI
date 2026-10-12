//! Local, wallet-scoped measurements. Missing provider usage is never a zero balance.
use crate::{Api, AppState, PrivateState, providers};
use anyhow::Result;
use axum::Json;
use serde_json::{Value, json};
use std::time::Instant;

pub fn init(conn: &rusqlite::Connection) -> Result<()> {
    crate::schema::ensure(
        conn,
        "CREATE TABLE IF NOT EXISTS ai_usage(id TEXT PRIMARY KEY, memo_id TEXT NOT NULL REFERENCES memos(id) ON DELETE CASCADE, owner_wallet TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL, created_at TEXT NOT NULL, elapsed_ms INTEGER NOT NULL DEFAULT 0, first_token_ms INTEGER, input_tokens INTEGER, output_tokens INTEGER, status TEXT NOT NULL); CREATE INDEX IF NOT EXISTS ai_usage_owner ON ai_usage(owner_wallet,created_at);",
    )?;
    Ok(())
}
struct Measurement<'a> {
    s: &'a AppState,
    id: String,
    start: Instant,
    finished: bool,
}
impl Drop for Measurement<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.s.db.0.lock().unwrap().execute(
                "UPDATE ai_usage SET status='interrupted',elapsed_ms=?2 WHERE id=?1",
                rusqlite::params![self.id, self.start.elapsed().as_millis() as i64],
            );
        }
    }
}
pub async fn stream(
    s: &AppState,
    memo: &str,
    config: &providers::Config,
    system: &str,
    messages: &[providers::Message],
    tools: &[Value],
    mut on_text: impl FnMut(&str) -> Result<()>,
) -> Result<providers::Turn> {
    let id = crate::db::id();
    s.db.require("memos", memo)?;
    s.db.0.lock().unwrap().execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,status) VALUES(?1,?2,?3,?4,?5,?6,'running')",rusqlite::params![id,memo,s.db.owner(),config.provider,config.model,crate::db::now()])?;
    let mut measurement = Measurement {
        s,
        id,
        start: Instant::now(),
        finished: false,
    };
    let mut first = None;
    let result = providers::stream(config, system, messages, tools, |text| {
        if !text.is_empty() && first.is_none() {
            first = Some(measurement.start.elapsed().as_millis() as i64);
        }
        on_text(text)
    })
    .await;
    let tokens = result
        .as_ref()
        .ok()
        .map(|t| (t.input_tokens, t.output_tokens))
        .unwrap_or_default();
    s.db.0.lock().unwrap().execute("UPDATE ai_usage SET elapsed_ms=?2,first_token_ms=?3,input_tokens=?4,output_tokens=?5,status=?6 WHERE id=?1",rusqlite::params![measurement.id,measurement.start.elapsed().as_millis() as i64,first,tokens.0,tokens.1,if result.is_ok(){"complete"}else{"error"}])?;
    measurement.finished = true;
    result
}
/// Record a provider call that is not a chat turn, such as a web search, with
/// token counts left unknown.
pub fn record_external(
    s: &AppState,
    memo: &str,
    provider: &str,
    model: &str,
    elapsed_ms: i64,
    ok: bool,
) -> Result<()> {
    s.db.require("memos", memo)?;
    s.db.0.lock().unwrap().execute(
        "INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,elapsed_ms,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![crate::db::id(),memo,s.db.owner(),provider,model,crate::db::now(),elapsed_ms,if ok {"complete"} else {"error"}],
    )?;
    Ok(())
}
/// Demo mode never calls a provider, but it still records a measured turn with
/// unknown token counts so progress and usage views behave like production.
pub async fn demo_turn(
    s: &AppState,
    memo: &str,
    config: &providers::Config,
    delay: std::time::Duration,
) -> Result<()> {
    let id = crate::db::id();
    s.db.require("memos", memo)?;
    s.db.0.lock().unwrap().execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,status) VALUES(?1,?2,?3,?4,?5,?6,'running')",rusqlite::params![id,memo,s.db.owner(),config.provider,config.model,crate::db::now()])?;
    let measurement = Measurement {
        s,
        id,
        start: Instant::now(),
        finished: false,
    };
    tokio::time::sleep(delay).await;
    let elapsed = measurement.start.elapsed().as_millis() as i64;
    s.db.0.lock().unwrap().execute(
        "UPDATE ai_usage SET elapsed_ms=?2,first_token_ms=?2,status='complete' WHERE id=?1",
        rusqlite::params![measurement.id, elapsed],
    )?;
    std::mem::forget(measurement);
    Ok(())
}
pub async fn get(PrivateState(s): PrivateState) -> Api<Json<Value>> {
    let conn = s.db.0.lock().unwrap();
    let mut query=conn.prepare("SELECT provider,COUNT(*),SUM(elapsed_ms),SUM(input_tokens),SUM(output_tokens),SUM(CASE WHEN input_tokens IS NULL OR output_tokens IS NULL THEN 1 ELSE 0 END) FROM ai_usage WHERE owner_wallet=?1 GROUP BY provider")?;
    let rows=query.query_map([s.db.owner()],|r|Ok(json!({"provider":r.get::<_,String>(0)?,"requests":r.get::<_,i64>(1)?,"elapsed_ms":r.get::<_,i64>(2)?,"input_tokens":r.get::<_,Option<i64>>(3)?,"output_tokens":r.get::<_,Option<i64>>(4)?,"unreported_requests":r.get::<_,i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(
        json!({"providers":rows,"credit_balance":null,"estimated_usd_saved":null,"demo":s.demo,"note":"Tokens are provider-reported where available. Credit balance and USD savings are unavailable. Local elapsed time includes prompt processing and generation."}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_usage_preserves_missing_values_and_does_not_double_count() {
        let mut a = providers::Accumulator::default();
        a.accept(
            "openai",
            r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert_eq!(a.turn.input_tokens, None);
        let usage = r#"{"usage":{"prompt_tokens":12,"completion_tokens":3},"choices":[]}"#;
        a.accept("openai", usage).unwrap();
        a.accept("openai", usage).unwrap();
        let result = a.finish().unwrap();
        assert_eq!(result.input_tokens, Some(12));
        assert_eq!(result.output_tokens, Some(3));
        let mut a = providers::Accumulator::default();
        a.accept(
            "anthropic",
            r#"{"type":"message_start","message":{"usage":{"input_tokens":9,"output_tokens":0}}}"#,
        )
        .unwrap();
        a.accept(
            "anthropic",
            r#"{"type":"message_delta","usage":{"output_tokens":5}}"#,
        )
        .unwrap();
        assert_eq!(a.turn.input_tokens, Some(9));
        assert_eq!(a.turn.output_tokens, Some(5));
        let mut a = providers::Accumulator::default();
        a.accept(
            "gemini",
            r#"{"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":4}}"#,
        )
        .unwrap();
        assert_eq!(a.turn.input_tokens, Some(7));
        assert_eq!(a.turn.output_tokens, Some(4));
    }
    #[test]
    fn usage_request_options_follow_each_provider_protocol() {
        for provider in ["mlx", "openai", "grok", "gemini", "anthropic"] {
            let config = providers::Config {
                provider: provider.into(),
                model: "fixture".into(),
                ..Default::default()
            };
            let body = providers::request_body(&config, "test", &[], &[]);
            if matches!(provider, "mlx" | "openai" | "grok") {
                assert_eq!(body["stream_options"]["include_usage"], true);
            } else {
                assert!(body.get("stream_options").is_none());
            }
            assert_eq!(
                body.get("max_tokens").is_some(),
                matches!(provider, "mlx" | "anthropic")
            );
        }
    }

    #[tokio::test]
    async fn usage_is_wallet_scoped_and_erased_with_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let s = AppState::open_with_credentials(
            dir.path().into(),
            false,
            None,
            crate::credentials::Credentials::default(),
        )
        .unwrap();
        let owner = s.for_owner("alice");
        let other = s.for_owner("bob");
        let memo = owner
            .db
            .create("test", "question", "mlx", "fixture")
            .unwrap();
        owner.db.0.lock().unwrap().execute("INSERT INTO ai_usage(id,memo_id,owner_wallet,provider,model,created_at,elapsed_ms,input_tokens,output_tokens,status) VALUES('id',?1,'alice','mlx','fixture','now',1200,10,5,'complete')",[&memo.id]).unwrap();
        let result = get(PrivateState(owner.clone())).await.unwrap().0;
        assert_eq!(result["providers"][0]["elapsed_ms"], 1200);
        assert!(result["credit_balance"].is_null());
        assert!(
            get(PrivateState(other)).await.unwrap().0["providers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        owner.db.delete(&memo.id).unwrap();
        assert!(
            get(PrivateState(owner)).await.unwrap().0["providers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
