//! Keeping files without AI: a memo that carries the files, reads their text on
//! the backend, lands in the BM25 index, and becomes a Wiki source with a
//! Markdown mirror on disk.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app};
use image::ImageFormat;
use serde_json::{Value, json};
use std::io::Cursor;
use tower::ServiceExt;

async fn call(r: Router, method: &str, path: &str, v: Value) -> (StatusCode, Value) {
    let res = r
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
    let status = res.status();
    let b = axum::body::to_bytes(res.into_body(), 4_000_000)
        .await
        .unwrap();
    (status, serde_json::from_slice(&b).unwrap_or_default())
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_fn(96, 64, |x, y| image::Rgb([(x * 2) as u8, 200, y as u8]));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

#[tokio::test]
async fn typed_memo_is_tidied_for_wiki_but_chat_keeps_original_text() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let router = app(s.clone(), "dist".into());
    let (_, room) = call(
        router.clone(),
        "POST",
        "/api/rooms",
        json!({"title":"Private notes"}),
    )
    .await;
    let room = room["roomid"].as_str().unwrap();
    let text = "Remember the ORCHID plan. Review tomorrow. #plans";
    let (status, saved) = call(
        router,
        "POST",
        "/api/memos",
        json!({"room_id":room,"message_id":uuid::Uuid::new_v4().to_string(),"content":text}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = saved["id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while s.db.get(id).unwrap().unwrap().kind != "note" {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    let memo = s.db.get(id)?.unwrap();
    assert_eq!(memo.input, text);
    assert!(memo.answer.starts_with("### "));
    assert!(memo.answer.contains("ORCHID") && memo.answer.contains("Review tomorrow"));
    let messages = causewaybay_server::chat::changes(&s.db, room, 0)?.messages;
    let visible: Vec<_> = messages.iter().filter(|m| !m.deleted).collect();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].text, text);
    assert_eq!(visible[0].role, "user");
    Ok(())
}

#[tokio::test]
async fn kept_files_become_an_indexed_wiki_source_without_ai() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let (status, room) = call(r.clone(), "POST", "/api/rooms", json!({"title":"Keep"})).await;
    assert_eq!(status, StatusCode::OK, "{room}");
    let room = room["roomid"].as_str().unwrap().to_string();
    let notes = s.db.put_file(
        "launch notes.md",
        "text/markdown",
        b"# Launch brief\nThe codeword is ORCHID-48. #launch",
    )?;
    let photo = s.db.put_file("photo.png", "image/png", &png())?;
    let message = uuid::Uuid::new_v4().to_string();
    let body =
        json!({"room_id":room,"message_id":message,"content":"","file_ids":[notes.id,photo.id]});

    // Nothing to keep is a client error; files alone are enough.
    let (status, _) = call(r.clone(), "POST", "/api/memos", json!({"room_id":room})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, memo) = call(r.clone(), "POST", "/api/memos", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    assert_eq!(memo["kind"], "memo");
    assert_eq!(memo["status"], "complete");
    assert_eq!(memo["title"], "launch notes.md, photo.png");
    let input = memo["input"].as_str().unwrap();
    assert!(input.contains("ORCHID-48"), "{input}");
    assert!(input.contains("Attached file: photo.png"), "{input}");
    assert_eq!(memo["tags"], json!(["launch"]));
    let id = memo["id"].as_str().unwrap().to_string();

    // A retry with the same GUID returns the same memo and no second message.
    let (status, again) = call(r.clone(), "POST", "/api/memos", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["id"], id);
    let sync = causewaybay_server::chat::changes(&s.db, &room, 0)?;
    let kept: Vec<_> = sync.messages.iter().filter(|m| !m.deleted).collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].messageid, message);
    assert_eq!(kept[0].provider, "memo");
    assert_eq!(kept[0].status, "complete");
    assert_eq!(kept[0].attachments.len(), 2);
    assert_eq!(kept[0].input_mode, causewaybay_shared::InputMode::Image);

    // BM25 over the memo index sees the file's text.
    assert!(s.db.lexical("orchid")?.contains(&id));
    assert!(s.db.lexical("codeword")?.contains(&id));

    // The Wiki organizes it in the background and mirrors the page to disk.
    // Demo seeds other sources; find the page that cites this memo by its hashtag.
    let mut detail = Value::Null;
    for _ in 0..400 {
        let (_, wiki) = call(r.clone(), "GET", "/api/wiki?q=ORCHID-48", Value::Null).await;
        // The memo is one feed item; each of its tags has a derived document.
        let documents: Vec<String> = wiki["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["id"] == id)
            .flat_map(|item| item["tags"].as_array().cloned().unwrap_or_default())
            .filter_map(|tag| {
                wiki["tags"].as_array()?.iter().find(|t| t["tag"] == tag)?["document"]
                    .as_str()
                    .map(str::to_owned)
            })
            .collect();
        for document in documents {
            let (_, candidate) = call(
                r.clone(),
                "GET",
                &format!("/api/wiki/{document}"),
                Value::Null,
            )
            .await;
            if candidate["sources"]
                .as_array()
                .is_some_and(|sources| sources.iter().any(|s| s["id"] == id))
            {
                detail = candidate;
            }
        }
        if detail.is_object() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(detail.is_object(), "no Wiki page cites the kept memo");
    assert_eq!(detail["title"], "launch");
    let markdown = detail["markdown"].as_str().unwrap();
    assert!(markdown.contains("ORCHID-48"), "{markdown}");
    assert!(
        markdown.contains(&format!("/api/files/{}/image", photo.id)),
        "{markdown}"
    );
    assert!(
        detail["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == id)
    );
    // Text stays in SQLite: no Markdown page or memo export is written to disk,
    // and the kept Markdown file itself is a row, not a file.
    let written: Vec<_> = walkdir(dir.path())
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    assert!(written.is_empty(), "{written:?}");
    assert!(!dir.path().join("wiki").exists() && !dir.path().join("markdown").exists());

    // A later question in the room gets the kept file as context.
    let later = s.db.create("later", "question", "grok", "test")?;
    causewaybay_server::chat::attach(
        &s.db,
        &room,
        &uuid::Uuid::new_v4().to_string(),
        &later,
        &causewaybay_shared::ThinkRequest {
            prompt: "later".into(),
            ..Default::default()
        },
    )?;
    let history = causewaybay_server::chat::history(
        &s.db,
        &room,
        &later.id,
        causewaybay_shared::InputMode::Text,
        4,
    )
    .await?;
    let text = format!("{history:?}");
    assert!(text.contains("ORCHID-48"), "{text}");
    Ok(())
}

fn walkdir(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[tokio::test]
async fn a_photo_without_text_still_gets_an_answer_in_text_mode() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let (_, room) = call(r.clone(), "POST", "/api/rooms", json!({"title":"Photos"})).await;
    let room = room["roomid"].as_str().unwrap().to_string();
    let photo = s.db.put_file("flowers.png", "image/png", &png())?;
    let message = uuid::Uuid::new_v4().to_string();
    let (status, memo) = call(
        r.clone(),
        "POST",
        "/api/think",
        json!({"room_id":room,"message_id":message,"prompt":"flower","file_ids":[photo.id],"input_mode":"text"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    let id = memo["id"].as_str().unwrap().to_string();
    let mut memo = None;
    for _ in 0..400 {
        let m = s.db.get(&id)?.unwrap();
        if m.status != "running" {
            memo = Some(m);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let memo = memo.expect("the answer finished");
    assert_eq!(memo.status, "complete", "{:?}", memo.error);
    assert!(memo.input.contains("[Photo attached."), "{}", memo.input);
    assert!(memo.input.starts_with("flower"), "{}", memo.input);
    Ok(())
}

/// A line that reads like a record is not answered: the AI tidies it into a
/// `note` memo, both messages carry the memo kind, and the original stays.
#[tokio::test]
async fn a_record_is_tidied_into_a_note_instead_of_answered() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let (_, room) = call(r.clone(), "POST", "/api/rooms", json!({"title":"Notes"})).await;
    let room = room["roomid"].as_str().unwrap().to_string();
    let line = "Had dinner with mom tonight, she seemed tired. I should visit more often.";
    let message = uuid::Uuid::new_v4().to_string();
    let (status, memo) = call(
        r.clone(),
        "POST",
        "/api/think",
        json!({"room_id":room,"message_id":message,"prompt":line}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    assert_eq!(memo["kind"], "note");
    let id = memo["id"].as_str().unwrap().to_string();
    let mut done = None;
    for _ in 0..400 {
        let m = s.db.get(&id)?.unwrap();
        if m.status != "running" {
            done = Some(m);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let m = done.expect("the tidy pass finished");
    assert_eq!(m.status, "complete", "{:?}", m.error);
    assert_eq!(m.kind, "note");
    assert_eq!(m.input, line);
    assert!(
        m.answer.starts_with("### Had dinner with mom tonight"),
        "{}",
        m.answer
    );
    assert!(
        m.answer.contains("- I should visit more often"),
        "{}",
        m.answer
    );
    let (_, sync) = call(
        r.clone(),
        "GET",
        &format!("/api/rooms/{room}/sync?after=0"),
        json!({}),
    )
    .await;
    let messages = sync["messages"].as_array().unwrap();
    assert!(messages.iter().all(|m| m["memo_kind"] == "note"), "{sync}");
    assert!(
        messages
            .iter()
            .any(|m| m["role"] == "user" && m["text"] == line)
    );
    assert!(messages.iter().any(
        |m| m["role"] == "assistant" && m["text"].as_str().unwrap().contains("### Had dinner")
    ));
    // A question is still answered, and "ask" forces an answer for a record.
    let (_, asked) = call(
        r.clone(),
        "POST",
        "/api/think",
        json!({"room_id":room,"message_id":uuid::Uuid::new_v4().to_string(),"prompt":line,"ask":true}),
    )
    .await;
    assert_eq!(asked["kind"], "question", "{asked}");
    let (_, question) = call(
        r.clone(),
        "POST",
        "/api/think",
        json!({"room_id":room,"message_id":uuid::Uuid::new_v4().to_string(),"prompt":"What is Zig?"}),
    )
    .await;
    assert_eq!(question["kind"], "question", "{question}");
    Ok(())
}
