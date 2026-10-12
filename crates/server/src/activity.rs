//! Per-session progress log and measured inference view for the Cloud page.
//! Stage labels are fixed strings (plus the capture device's own name); note text,
//! prompts, answers and attachment names never enter this table.
use crate::db::{self, Database};
use crate::{Api, PrivateState};
use anyhow::Result;
use axum::Json;
use causewaybay_shared::{
    ActiveInference, ActivityEntry, ActivitySummary, ActivityTotals, InferenceRecord,
    SessionActivity, UsageBucket, ai_location,
};
use rusqlite::{Connection, OptionalExtension, params};

/// Date of the list prices below. Estimates are labelled with it in the UI.
pub const PRICE_DATE: &str = "2026-10-06";
/// USD per million input/output tokens, keyed by the exact model id an admin can
/// configure. Unknown models and local providers produce no estimate.
const PRICES: &[(&str, f64, f64)] = &[
    ("claude-sonnet-4-5", 3.0, 15.0),
    ("claude-sonnet-4-5-20250929", 3.0, 15.0),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("grok-4", 3.0, 15.0),
    ("gemini-2.5-flash", 0.30, 2.50),
];
const KEEP_PER_ROOM: usize = 200;
const SHOW_ENTRIES: usize = 40;
const SHOW_INFERENCES: usize = 12;

pub fn init(conn: &Connection) -> Result<()> {
    crate::schema::ensure(conn,
        "CREATE TABLE IF NOT EXISTS session_activity(id INTEGER PRIMARY KEY AUTOINCREMENT,
        owner_wallet TEXT NOT NULL, roomid TEXT NOT NULL, created_at TEXT NOT NULL,
        source TEXT NOT NULL, stage TEXT NOT NULL, provider TEXT NOT NULL DEFAULT '',
        model TEXT NOT NULL DEFAULT '');
        CREATE INDEX IF NOT EXISTS session_activity_room ON session_activity(owner_wallet,roomid,id);",
    )?;
    Ok(())
}

pub fn estimate_usd(
    provider: &str,
    model: &str,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
) -> Option<f64> {
    if ai_location(provider) != "cloud" {
        return None;
    }
    // Subscription CLIs bill nothing per request here; tokens are informational.
    if causewaybay_shared::cli_provider(provider) {
        return Some(0.0);
    }
    let (_, input, output) = PRICES.iter().find(|(id, _, _)| *id == model)?;
    Some((input_tokens? as f64 * input + output_tokens? as f64 * output) / 1_000_000.0)
}

/// Append one step. Callers hold no database lock.
pub fn log(
    database: &Database,
    room: &str,
    source: &str,
    stage: &str,
    provider: &str,
    model: &str,
) {
    let conn = database.0.lock().unwrap();
    if let Err(error) = log_with(
        &conn,
        database.owner(),
        room,
        source,
        stage,
        provider,
        model,
    ) {
        tracing::warn!(%error, %room, "Could not record session activity");
    }
}

pub fn log_with(
    conn: &Connection,
    owner: &str,
    room: &str,
    source: &str,
    stage: &str,
    provider: &str,
    model: &str,
) -> Result<()> {
    let stage = stage.trim();
    if stage.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO session_activity(owner_wallet,roomid,created_at,source,stage,provider,model) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![owner, room, db::now(), source, stage, provider, model],
    )?;
    conn.execute(
        "DELETE FROM session_activity WHERE owner_wallet=?1 AND roomid=?2 AND id NOT IN
        (SELECT id FROM session_activity WHERE owner_wallet=?1 AND roomid=?2 ORDER BY id DESC LIMIT ?3)",
        params![owner, room, KEEP_PER_ROOM as i64],
    )?;
    Ok(())
}

/// Record a stage for a memo's room, used by chat and capture jobs. Provider and
/// model are attached once the job has entered its AI phase.
pub fn log_memo_stage(conn: &Connection, memo: &str, stage: &str) -> Result<()> {
    let Some((room, owner, provider, model)) = conn
        .query_row(
            "SELECT roomid,owner_wallet,provider,model FROM memos WHERE id=?1 AND roomid IS NOT NULL",
            [memo],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(());
    };
    let captured: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE memo_id=?1 AND kind='phase' AND text='capture')",
        [memo],
        |r| r.get(0),
    )?;
    let phase: Option<String> = conn
        .query_row(
            "SELECT text FROM events WHERE memo_id=?1 AND kind='phase' ORDER BY id DESC LIMIT 1",
            [memo],
            |r| r.get(0),
        )
        .optional()?;
    let ai = phase.as_deref() == Some("ai");
    // File-context stages name the attachment; the progress log keeps only the step.
    let stage = if stage.starts_with("Reading ") {
        "Reading attached files"
    } else {
        stage
    };
    log_with(
        conn,
        &owner,
        &room,
        if captured { "capture" } else { "chat" },
        stage,
        if ai { &provider } else { "" },
        if ai { &model } else { "" },
    )
}

pub fn purge(conn: &Connection, room: &str) -> Result<()> {
    conn.execute("DELETE FROM session_activity WHERE roomid=?1", [room])?;
    Ok(())
}

pub fn read(database: &Database, room: &str) -> Result<SessionActivity> {
    let conn = database.0.lock().unwrap();
    let owner = database.owner();
    let entries = conn
        .prepare(
            "SELECT created_at,source,stage,provider,model FROM session_activity
            WHERE owner_wallet=?1 AND roomid=?2 ORDER BY id DESC LIMIT ?3",
        )?
        .query_map(params![owner, room, SHOW_ENTRIES as i64], |r| {
            let provider: String = r.get(3)?;
            Ok(ActivityEntry {
                at: r.get(0)?,
                source: r.get(1)?,
                stage: r.get(2)?,
                location: if provider.is_empty() {
                    String::new()
                } else {
                    ai_location(&provider).into()
                },
                provider,
                model: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let rows = conn
        .prepare(
            "SELECT u.memo_id,u.created_at,u.provider,u.model,u.status,u.elapsed_ms,u.first_token_ms,u.input_tokens,u.output_tokens
            FROM ai_usage u JOIN memos m ON m.id=u.memo_id
            WHERE u.owner_wallet=?1 AND m.roomid=?2 ORDER BY u.created_at DESC,u.rowid DESC",
        )?
        .query_map(params![owner, room], |r| {
            let provider: String = r.get(2)?;
            let model: String = r.get(3)?;
            let input_tokens = r.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64);
            let output_tokens = r.get::<_, Option<i64>>(8)?.map(|v| v.max(0) as u64);
            Ok(InferenceRecord {
                memo_id: r.get(0)?,
                created_at: r.get(1)?,
                location: ai_location(&provider).into(),
                estimated_usd: estimate_usd(&provider, &model, input_tokens, output_tokens),
                provider,
                model,
                status: r.get(4)?,
                elapsed_ms: r.get::<_, i64>(5)?.max(0) as u64,
                first_token_ms: r.get::<_, Option<i64>>(6)?.map(|v| v.max(0) as u64),
                input_tokens,
                output_tokens,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(conn);
    let mut totals = ActivityTotals::default();
    let mut priced = false;
    let mut counted_tokens = false;
    for row in &rows {
        totals.requests += 1;
        totals.elapsed_ms += row.elapsed_ms;
        if let (Some(input), Some(output)) = (row.input_tokens, row.output_tokens) {
            counted_tokens = true;
            *totals.input_tokens.get_or_insert(0) += input;
            *totals.output_tokens.get_or_insert(0) += output;
        }
        if row.location == "cloud" {
            match row.estimated_usd {
                Some(usd) => {
                    priced = true;
                    *totals.estimated_usd.get_or_insert(0.0) += usd;
                }
                None => totals.unpriced_cloud_requests += 1,
            }
        }
    }
    if !counted_tokens {
        totals.input_tokens = None;
        totals.output_tokens = None;
    }
    let thinking = rows
        .iter()
        .find(|row| row.status == "running")
        .map(|row| ActiveInference {
            memo_id: row.memo_id.clone(),
            provider: row.provider.clone(),
            model: row.model.clone(),
            location: row.location.clone(),
            waited_ms: chrono::DateTime::parse_from_rfc3339(&row.created_at)
                .ok()
                .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_milliseconds())
                .unwrap_or_default()
                .max(0) as u64,
        });
    Ok(SessionActivity {
        thinking,
        entries,
        inferences: rows.into_iter().take(SHOW_INFERENCES).collect(),
        totals,
        price_date: if priced {
            PRICE_DATE.into()
        } else {
            String::new()
        },
    })
}

/// Stage labels that mean a capture was dropped before any model ran.
pub const SKIPPED_STAGES: [&str; 2] = [
    "Screen text unchanged · AI skipped",
    "Unchanged screenshot discarded",
];

fn bucket_add(
    bucket: &mut UsageBucket,
    provider: &str,
    status: &str,
    elapsed_ms: u64,
    tokens: (Option<u64>, Option<u64>),
    usd: Option<f64>,
) {
    bucket.requests += 1;
    bucket.elapsed_ms += elapsed_ms;
    match status {
        "running" => bucket.running += 1,
        "error" | "interrupted" => bucket.errors += 1,
        _ => {}
    }
    if let (Some(i), Some(o)) = tokens {
        *bucket.input_tokens.get_or_insert(0) += i;
        *bucket.output_tokens.get_or_insert(0) += o;
    }
    if let Some(usd) = usd {
        *bucket.estimated_usd.get_or_insert(0.0) += usd;
    }
    match bucket.providers.iter_mut().find(|(p, _)| p == provider) {
        Some((_, n)) => *n += 1,
        None => bucket.providers.push((provider.to_owned(), 1)),
    }
}

/// Wallet-wide counts of OCR runs, skipped captures, and local/cloud inference.
pub fn summary(database: &Database) -> Result<ActivitySummary> {
    let conn = database.0.lock().unwrap();
    let owner = database.owner();
    let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    let mut out = ActivitySummary::default();
    let mut priced = false;
    let mut rows = conn.prepare(
        "SELECT provider,model,created_at,status,elapsed_ms,input_tokens,output_tokens FROM ai_usage WHERE owner_wallet=?1 ORDER BY created_at DESC",
    )?;
    for row in rows.query_map([owner], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?.max(0) as u64,
            r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
            r.get::<_, Option<i64>>(6)?.map(|v| v.max(0) as u64),
        ))
    })? {
        let (provider, model, created_at, status, elapsed, input, output) = row?;
        if out.last_at.is_empty() {
            out.last_at = created_at.clone();
        }
        let usd = estimate_usd(&provider, &model, input, output);
        priced |= usd.is_some();
        let local = ai_location(&provider) == "local";
        for (stats, include) in [(&mut out.total, true), (&mut out.day, created_at >= since)] {
            if include {
                let bucket = if local {
                    &mut stats.local
                } else {
                    &mut stats.cloud
                };
                bucket_add(bucket, &provider, &status, elapsed, (input, output), usd);
            }
        }
    }
    for stats in [&mut out.total, &mut out.day] {
        for bucket in [&mut stats.local, &mut stats.cloud] {
            bucket
                .providers
                .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        }
    }
    let mut ocr = conn.prepare(
        "SELECT m.created_at FROM events e JOIN memos m ON m.id=e.memo_id WHERE e.kind='ocr_text' AND m.owner_wallet=?1",
    )?;
    for created_at in ocr.query_map([owner], |r| r.get::<_, String>(0))? {
        let created_at = created_at?;
        out.total.ocr_runs += 1;
        if created_at >= since {
            out.day.ocr_runs += 1;
        }
    }
    let mut skipped = conn.prepare(
        "SELECT created_at FROM session_activity WHERE owner_wallet=?1 AND stage IN (?2,?3)",
    )?;
    for created_at in skipped
        .query_map(params![owner, SKIPPED_STAGES[0], SKIPPED_STAGES[1]], |r| {
            r.get::<_, String>(0)
        })?
    {
        let created_at = created_at?;
        out.total.skipped_unchanged += 1;
        if created_at >= since {
            out.day.skipped_unchanged += 1;
        }
    }
    // The local triage in expensive modes: checked, declined, escalated; the rest
    // were answered locally. "Thinking with cloud help…" also counts Hybrid escalations.
    let mut triage = conn.prepare(
        "SELECT created_at, stage FROM session_activity WHERE owner_wallet=?1 AND stage IN (?2,?3,?4,?5)",
    )?;
    for row in triage.query_map(
        params![
            owner,
            "Checking with the local LLM…",
            "No request found · cloud AI skipped",
            "Answered on-device · cloud AI skipped",
            "Thinking with cloud help…"
        ],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )? {
        let (created_at, stage) = row?;
        for stats in [
            Some(&mut out.total),
            (created_at >= since).then_some(&mut out.day),
        ]
        .into_iter()
        .flatten()
        {
            match stage.as_str() {
                "Checking with the local LLM…" => stats.local_checks += 1,
                "No request found · cloud AI skipped" => stats.no_request += 1,
                "Answered on-device · cloud AI skipped" => stats.answered_locally += 1,
                _ => stats.escalated += 1,
            }
        }
    }

    if priced {
        out.price_date = PRICE_DATE.into();
    }
    Ok(out)
}

pub async fn summary_route(PrivateState(s): PrivateState) -> Api<Json<ActivitySummary>> {
    Ok(Json(summary(&s.db)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimates_only_priced_cloud_models_with_reported_tokens() {
        assert_eq!(
            estimate_usd("grok", "grok-4", Some(1_000_000), Some(1_000_000)),
            Some(18.0)
        );
        assert_eq!(
            estimate_usd("openai", "gpt-4.1-mini", Some(500_000), Some(0)),
            Some(0.20)
        );
        assert_eq!(estimate_usd("grok", "grok-4", None, Some(5)), None);
        assert_eq!(
            estimate_usd("grok", "grok-4-custom", Some(5), Some(5)),
            None
        );
        assert_eq!(estimate_usd("mlx", "grok-4", Some(5), Some(5)), None);
        assert_eq!(estimate_usd("apple", "system", Some(5), Some(5)), None);
    }
}
