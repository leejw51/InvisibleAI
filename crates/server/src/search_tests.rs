//! On-device semantic search, end to end: notes saved through Save anything
//! are embedded by the local Rust indexer (fastembed, multilingual MiniLM),
//! found by BM25 fused with cosine similarity, and handed to chat.
//!
//! Search never depends on the AI mode. The same code runs whether the user
//! picked an on-device model (MLX, Apple) or a cloud model; only the final
//! answer goes to a different model. The test proves it by running the same
//! writes and reads under both modes.
//!
//! Ignored by default: it needs the embedding model, which `make test-semantic`
//! downloads once into `crates/server/data/models`.
use super::*;
use crate::app;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rusqlite::OptionalExtension;
use tower::ServiceExt;

async fn call(s: &Arc<AppState>, method: &str, path: &str, v: Value) -> (StatusCode, Value) {
    let r = app(s.clone(), "dist".into())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let code = r.status();
    let b = axum::body::to_bytes(r.into_body(), 2_000_000)
        .await
        .unwrap();
    (code, serde_json::from_slice(&b).unwrap_or_default())
}

/// A real (non-demo) workspace whose indexer uses the cached model.
async fn workspace() -> (tempfile::TempDir, Arc<AppState>) {
    let dir = tempfile::tempdir().unwrap();
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/models");
    std::fs::create_dir_all(&cache).unwrap();
    std::os::unix::fs::symlink(&cache, dir.path().join("models")).unwrap();
    // Empty credentials: whatever keys this shell exports, no cloud model may
    // be reached from here. Search and embeddings must work without any.
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        crate::credentials::Credentials::default(),
    )
    .unwrap()
    .for_owner("local");
    s.start_indexer(false);
    for _ in 0..1200 {
        if s.semantic.ready() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        s.semantic.ready(),
        "embedding model: {}",
        s.semantic.status.lock().unwrap()
    );
    (dir, s)
}

/// Save anything, exactly as the page does, and wait until it is embedded.
async fn save(s: &Arc<AppState>, content: &str) -> String {
    let (code, memo) = call(s, "POST", "/api/memos", json!({ "content": content })).await;
    assert_eq!(code, StatusCode::OK, "{memo}");
    let id = memo["id"].as_str().unwrap().to_string();
    for _ in 0..400 {
        if s.db.embedding(&id).unwrap().is_some() {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("note {id} was never embedded");
}

fn choose(s: &AppState, mode: &str) {
    // Written straight to the account setting: PATCH would require a
    // configured cloud key, and search must not care either way.
    s.db.set_setting(
        "user.ai.local",
        &json!({"mode": mode, "local_provider": "mlx", "cloud_provider": "grok"}).to_string(),
    )
    .unwrap();
    assert_eq!(ai_preferences::read(s).unwrap().unwrap().mode, mode);
}

async fn ids(s: &Arc<AppState>, query: &str, kind: &str) -> Vec<String> {
    let found = search(s, query, kind, 0, 10).await.unwrap();
    assert!(found.semantic, "search ran without the embedding model");
    found.memos.into_iter().map(|m| m.id).collect()
}

#[tokio::test]
#[ignore = "Needs the multilingual embedding model; run make test-semantic"]
async fn saved_notes_are_found_by_meaning_in_on_device_and_cloud_mode() {
    let (_dir, s) = workspace().await;
    let bread = save(&s, "Sourdough: feed the starter, then bake at 230°C.").await;
    let train = save(&s, "The airport train leaves every 12 minutes.").await;
    for (mode, phone, question) in [
        (
            "ondevice",
            "내 전화번호 +82 10 1234 5678",
            "what is my phone number",
        ),
        (
            "cloudai",
            "My office phone is +852 2345 6789",
            "내 사무실 전화번호 알려줘",
        ),
    ] {
        choose(&s, mode);
        // Writing: Save anything keeps the text without any AI call.
        let id = save(&s, phone).await;
        assert_eq!(s.db.get(&id).unwrap().unwrap().input, phone);

        // Embedding generation: the local Rust indexer wrote a 384-d vector
        // under the current model name. No provider was called for it, in
        // cloud mode included: ai_usage stays empty.
        let vector = s.db.embedding(&id).unwrap().expect("embedded");
        assert_eq!(vector.len(), 384, "{mode}");
        let model: String =
            s.db.0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT model FROM embeddings WHERE memo_id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(model, semantic::MODEL_NAME);
        let calls: Vec<(String, String, String, String)> =
            s.db.0
                .lock()
                .unwrap()
                .prepare("SELECT provider,model,status,memo_id FROM ai_usage")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
        assert!(
            calls.is_empty(),
            "{mode}: embedding must not call an AI provider: {calls:?}"
        );

        // BM25 alone: the exact words of the note find it (with a particle).
        let words = if mode == "ondevice" {
            "전화번호는"
        } else {
            "office phone"
        };
        assert!(
            s.db.lexical(words).unwrap().contains(&id),
            "{mode}: BM25 {words}"
        );
        // BM25 cannot cross languages; that is what semantic search adds.
        assert!(
            !s.db.lexical(question).unwrap().contains(&id),
            "{mode}: {question}"
        );

        // Semantic alone: the question in the other language is among the
        // phone notes at the top, above the chat threshold, with bread and
        // the train below.
        let q = s.semantic.embed(question).unwrap().expect("query embedded");
        let nearest = s.db.semantic_scored(&q).unwrap();
        let rank = nearest.iter().position(|x| x.0 == id);
        assert!(rank.is_some_and(|r| r < 2), "{mode}: {nearest:?}");
        assert!(
            nearest[rank.unwrap()].1 >= think::KEPT_NOTE_SIMILARITY,
            "{mode}: {nearest:?}"
        );
        for other in [&bread, &train] {
            let o = nearest.iter().position(|x| &x.0 == other);
            assert!(o.is_none_or(|o| o > rank.unwrap()), "{mode}: {nearest:?}");
        }

        // Reading 1: Find anything / Notes search, in another language.
        // Another phone note may rank beside it; unrelated notes never ahead.
        for kind in ["all", "notes"] {
            let found = ids(&s, question, kind).await;
            let rank = found.iter().position(|x| x == &id);
            assert!(
                rank.is_some_and(|r| r < 2),
                "{mode} {kind}: {question} -> {found:?}"
            );
            for other in [&bread, &train] {
                if let Some(o) = found.iter().position(|x| x == other) {
                    assert!(
                        rank.unwrap() < o,
                        "{mode} {kind}: unrelated note ranked first"
                    );
                }
            }
        }

        // Reading 2: what chat hands to the chosen model (MLX or cloud).
        let shared = think::kept_notes(&s, "", question).await.unwrap();
        assert!(
            shared.contains(phone),
            "{mode}: chat context was {shared:?}"
        );
        assert!(
            !shared.contains("Sourdough") && !shared.contains("airport"),
            "{mode}: {shared}"
        );

        // Unrelated questions bring no personal notes into chat.
        let unrelated = think::kept_notes(&s, "", "how long should I boil an egg")
            .await
            .unwrap();
        assert!(!unrelated.contains(phone), "{mode}: {unrelated}");
    }
    // Earlier notes are still found by meaning after both modes.
    assert_eq!(ids(&s, "baking bread", "notes").await.first(), Some(&bread));
    assert_eq!(
        ids(&s, "공항 기차 시간", "notes").await.first(),
        Some(&train)
    );
}

/// Live: Save anything under each AI mode tags the note with the model that
/// mode calls for, and the note joins the Wiki with those tags at once.
/// On-device and Hybrid tag with the built-in Rust MLX worker (Qwen3.5 4B);
/// Cloud AI tags with Grok. Needs `make mlx-ai`, the installed 4B model and
/// XAI_API_KEY; makes one small paid Grok call.
#[tokio::test]
#[ignore = "Runs the on-device MLX model and one Grok call; run make test-tags-live"]
async fn saved_notes_are_tagged_and_kept_in_every_ai_mode() {
    let dir = tempfile::tempdir().unwrap();
    let s = AppState::open_with_credentials(
        dir.path().into(),
        false,
        None,
        crate::credentials::Credentials::from_environment(),
    )
    .unwrap();
    assert!(
        s.local_runtime.mlx_worker.is_file(),
        "run make mlx-ai first"
    );
    assert!(s.provider_configured("grok"), "export XAI_API_KEY");
    crate::preferences::update(
        &s,
        causewaybay_shared::SettingsUpdate {
            provider: Some("mlx".into()),
            model: Some("mlx-community/Qwen3.5-4B-4bit".into()),
            local_ai: Some(causewaybay_shared::LocalAiSettings {
                mlx_enabled: true,
                mlx_engine: "builtin".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let s = s.for_owner("local");
    assistance::save(&s, json!({"wiki_enabled": true})).unwrap();
    for (mode, tagger, text) in [
        ("ondevice", "mlx", "내 자전거는 외발자전거"),
        ("hybrid", "mlx", "My sister's birthday is on 14 March"),
        (
            "cloudai",
            "grok",
            "우리 집 와이파이 비밀번호는 sunflower-42",
        ),
    ] {
        choose(&s, mode);
        let (code, memo) = call(&s, "POST", "/api/memos", json!({ "content": text })).await;
        assert_eq!(code, StatusCode::OK, "{memo}");
        let id = memo["id"].as_str().unwrap().to_string();
        // Tagging runs in the background right after the save.
        let row = tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                let row: Option<(String, bool, String)> =
                    s.db.0
                        .lock()
                        .unwrap()
                        .query_row(
                            "SELECT provider,keep,topics FROM wiki_topics WHERE memo_id=?1",
                            [&id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .optional()
                        .unwrap();
                if let Some(row) = row {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{mode}: never tagged"));
        eprintln!("{mode}: {row:?}");
        assert_eq!(row.0, tagger, "{mode}: tagged by {}", row.0);
        assert!(row.1, "{mode}: a deliberate save is always kept");
        let tags: Vec<(String, String)> =
            s.db.0
                .lock()
                .unwrap()
                .prepare("SELECT tag,origin FROM wiki_tags WHERE memo_id=?1")
                .unwrap()
                .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
        assert!(
            tags.iter().any(|(_, o)| o == "model"),
            "{mode}: no model tags: {tags:?}"
        );
        assert!(
            !tags.iter().any(|(t, _)| t == "test" || t == "greeting"),
            "{mode}: {tags:?}"
        );
        // The Wiki was updated in the same transaction: the note is a source.
        let in_wiki: bool =
            s.db.0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM wiki_sources WHERE memo_id=?1)",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
        assert!(in_wiki, "{mode}: not in the Wiki");
        // And the AI call is on record under the model that made it.
        let used: String =
            s.db.0
                .lock()
                .unwrap()
                .query_row(
                    "SELECT group_concat(provider) FROM ai_usage WHERE memo_id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(used, tagger, "{mode}");
    }
}
