//! Where kept data lives: every piece of text in SQLite (as TEXT, searchable),
//! only binary media on disk, and older layouts migrated automatically when
//! the server starts.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use causewaybay_server::{AppState, app, db::Database};
use rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn raw(r: Router, method: &str, path: &str) -> (StatusCode, String, Vec<u8>) {
    let res = r
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let mime = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = axum::body::to_bytes(res.into_body(), 50_000_000)
        .await
        .unwrap();
    (status, mime, body.to_vec())
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\npicture bytes";

#[tokio::test]
async fn text_files_are_text_rows_and_media_is_on_disk() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let note = s.db.put_file(
        "notes.md",
        "text/markdown",
        "# Plan\n\nShip the SQLite-only text layout. 한국어도 그대로.".as_bytes(),
    )?;
    let code =
        s.db.put_file("main.rs", "application/octet-stream", b"fn main() {}")?;
    let empty = s.db.put_file("empty.txt", "text/plain", b"")?;
    let picture = s.db.put_file("dot.png", "image/png", PNG)?;
    // A UTF-16 document is not UTF-8 text: its exact bytes stay on disk.
    let utf16 =
        s.db.put_file("wide.txt", "text/plain", b"\xff\xfeh\x00i\x00")?;

    // Text rows: no path, content in the TEXT column, nothing under files/.
    for f in [&note, &code, &empty] {
        assert_eq!(f.path, "", "{}", f.name);
    }
    {
        let conn = s.db.0.lock().unwrap();
        let (kind, text, blob): (String, String, Vec<u8>) = conn.query_row(
            "SELECT typeof(text),text,bytes FROM files WHERE id=?1",
            [&note.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        assert_eq!(kind, "text");
        assert!(text.contains("한국어도 그대로"));
        assert!(blob.is_empty());
        // Plain SQL finds words inside kept files: the point of TEXT over BLOB.
        let hits: Vec<String> = conn
            .prepare("SELECT name FROM files WHERE text LIKE '%SQLite-only%' ORDER BY name")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(hits, ["notes.md"]);
        let main: String =
            conn.query_row("SELECT text FROM files WHERE id=?1", [&code.id], |r| {
                r.get(0)
            })?;
        assert_eq!(main, "fn main() {}");
    }
    assert_eq!(
        s.db.file(&note.id)?.1,
        "# Plan\n\nShip the SQLite-only text layout. 한국어도 그대로.".as_bytes()
    );
    assert_eq!(s.db.file(&empty.id)?.1, b"");
    assert!(s.db.file_location(&note.id).is_err());

    // Binary rows: bytes on disk, path in SQLite, TEXT column empty.
    for f in [&picture, &utf16] {
        assert!(f.path.starts_with("files/"), "{}", f.name);
        assert!(dir.path().join(&f.path).is_file());
        let text: String = s.db.0.lock().unwrap().query_row(
            "SELECT text FROM files WHERE id=?1",
            [&f.id],
            |r| r.get(0),
        )?;
        assert_eq!(text, "");
    }
    assert_eq!(std::fs::read(dir.path().join(&picture.path))?, PNG);
    assert_eq!(
        std::fs::read(dir.path().join(&utf16.path))?,
        b"\xff\xfeh\x00i\x00"
    );
    assert_eq!(s.db.file(&utf16.id)?.1, b"\xff\xfeh\x00i\x00");
    // Only the two binary files made a folder.
    assert_eq!(std::fs::read_dir(dir.path().join("files"))?.count(), 2);

    // The API serves both kinds the same way.
    let (status, mime, body) = raw(
        r.clone(),
        "GET",
        &format!("/api/files/{}/download", note.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mime, "text/markdown");
    assert_eq!(body, s.db.file(&note.id)?.1);
    let (status, mime, body) =
        raw(r.clone(), "GET", &format!("/api/files/{}/stream", code.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mime, "application/octet-stream");
    assert_eq!(body, b"fn main() {}");
    let (status, _, body) = raw(
        r.clone(),
        "GET",
        &format!("/api/files/{}/download", picture.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, PNG);
    let (status, _, body) = raw(r.clone(), "GET", &format!("/api/files/{}/text", note.id)).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body)?;
    assert!(v["text"].as_str().unwrap().contains("# Plan"));

    // Deleting a text file is deleting its row; a picture's bytes go too.
    for f in [&note, &picture] {
        let (status, _, _) = raw(r.clone(), "DELETE", &format!("/api/files/{}", f.id)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(s.db.file_info(&f.id).is_err());
    }
    assert!(!dir.path().join(&picture.path).exists());
    assert_eq!(walk(&dir.path().join("files")).len(), 1);

    // No memo or Wiki Markdown is ever written beside the database: apart from
    // SQLite itself, the only file left on disk is the UTF-16 document (binary).
    let memo =
        s.db.create("A thought to keep #layout", "memo", "local", "")?;
    s.db.event(&memo.id, "delta", "the answer")?;
    s.db.finish(&memo.id, "memo")?;
    assert!(!dir.path().join("markdown").exists());
    assert!(!dir.path().join("wiki").exists());
    let written: Vec<_> = walk(dir.path())
        .into_iter()
        .filter(|p| !p.to_string_lossy().contains("causewaybay.sqlite3"))
        .collect();
    assert_eq!(written, [dir.path().join(&utf16.path)]);
    Ok(())
}

#[test]
fn startup_migrates_older_layouts_into_sqlite() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("causewaybay.sqlite3");
    let created = chrono::Utc::now().to_rfc3339();
    let ids: Vec<String> = (0..4).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let (blob_text, blob_png, disk_text, disk_png) = (&ids[0], &ids[1], &ids[2], &ids[3]);
    {
        // Lay out what older versions left behind.
        let db = Database::open(&database)?;
        let conn = db.0.lock().unwrap();
        // 1. Oldest layout: bytes in SQLite, no path at all.
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet) VALUES(?1,'old-note.txt','text/plain',?2,?3,NULL,0,'local')",params![blob_text,b"kept as a blob".as_slice(),created])?;
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet) VALUES(?1,'old.png','image/png',?2,?3,NULL,0,'local')",params![blob_png,PNG,created])?;
        // 2. Previous layout: everything on disk, SQLite holds the path.
        for (id, name, bytes) in [
            (disk_text, "report.md", "# Report\n\nMigrate me.".as_bytes()),
            (disk_png, "photo.png", PNG),
        ] {
            let path = dir.path().join("files").join(id).join(name);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, bytes)?;
            let mime = if name.ends_with(".md") {
                "text/markdown"
            } else {
                "image/png"
            };
            conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet) VALUES(?1,?2,?3,x'',?4,?5,?6,'local')",params![id,name,mime,created,format!("files/{id}/{name}"),bytes.len()])?;
        }
        // 3. Derived Markdown mirrors of memos and Wiki pages.
        std::fs::create_dir_all(dir.path().join("markdown"))?;
        std::fs::write(dir.path().join("markdown").join("memo.md"), "# memo")?;
        std::fs::create_dir_all(dir.path().join("wiki").join("owner-hash"))?;
        std::fs::write(
            dir.path().join("wiki").join("owner-hash").join("page.md"),
            "# page",
        )?;
    }

    // Opening the database again is the migration; no command to run.
    let db = Database::open(&database)?;
    {
        let conn = db.0.lock().unwrap();
        let row = |id: &str| -> rusqlite::Result<(Option<String>, String, Vec<u8>, i64)> {
            conn.query_row(
                "SELECT path,text,bytes,size FROM files WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        };
        let (path, text, bytes, size) = row(blob_text)?;
        assert_eq!(
            (path.as_deref(), text.as_str(), bytes.len(), size),
            (Some(""), "kept as a blob", 0, 14)
        );
        let (path, text, bytes, _) = row(blob_png)?;
        assert!(path.as_deref().is_some_and(|p| p.starts_with("files/")));
        assert!(text.is_empty() && bytes.is_empty());
        assert_eq!(std::fs::read(dir.path().join(path.unwrap()))?, PNG);
        let (path, text, bytes, size) = row(disk_text)?;
        assert_eq!(
            (path.as_deref(), text.as_str(), bytes.len(), size),
            (Some(""), "# Report\n\nMigrate me.", 0, 21)
        );
        let (path, text, _, _) = row(disk_png)?;
        assert_eq!(
            path.as_deref(),
            Some(&*format!("files/{disk_png}/photo.png"))
        );
        assert!(text.is_empty());
    }
    // The text file and its folder are gone from disk; the picture stays.
    assert!(!dir.path().join("files").join(disk_text).exists());
    assert_eq!(
        std::fs::read(dir.path().join("files").join(disk_png).join("photo.png"))?,
        PNG
    );
    // Mirrors are gone.
    assert!(!dir.path().join("markdown").exists());
    assert!(!dir.path().join("wiki").exists());
    // Everything still reads through the normal API.
    assert_eq!(db.file(blob_text)?.1, b"kept as a blob");
    assert_eq!(db.file(disk_text)?.1, "# Report\n\nMigrate me.".as_bytes());
    assert_eq!(db.file(blob_png)?.1, PNG);
    assert_eq!(db.file(disk_png)?.1, PNG);
    assert_eq!(db.files()?.len(), 4);
    // A second start changes nothing.
    drop(db);
    let db = Database::open(&database)?;
    assert_eq!(db.file(disk_text)?.1, "# Report\n\nMigrate me.".as_bytes());
    assert_eq!(db.files()?.len(), 4);
    Ok(())
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn as_text_decides_what_lives_in_sqlite() {
    use causewaybay_server::db::as_text;
    assert_eq!(as_text("text/plain", b"hello"), Some("hello"));
    assert_eq!(as_text("text/plain", b""), Some(""));
    assert_eq!(
        as_text("text/markdown; charset=utf-8", "# 제목".as_bytes()),
        Some("# 제목")
    );
    assert_eq!(as_text("application/json", b"{\"a\":1}"), Some("{\"a\":1}"));
    assert_eq!(
        as_text("application/octet-stream", b"fn main() {}"),
        Some("fn main() {}")
    );
    // Empty and undeclared: nothing to search, so it is not a text row.
    assert_eq!(as_text("application/octet-stream", b""), None);
    // Declared media and containers stay binary even when their bytes are ASCII.
    assert_eq!(as_text("image/png", b"not really a picture"), None);
    assert_eq!(as_text("video/mp4", b"ftyp"), None);
    assert_eq!(as_text("audio/wav", b"RIFF"), None);
    assert_eq!(as_text("application/pdf", b"%PDF-1.4 tiny"), None);
    assert_eq!(as_text("application/zip", b"PK"), None);
    assert_eq!(
        as_text(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            b"PK"
        ),
        None
    );
    // Bytes that are not UTF-8 text keep their exact form on disk.
    assert_eq!(as_text("text/plain", b"\xff\xfeh\x00i\x00"), None);
    assert_eq!(as_text("text/plain", b"nul\0inside"), None);
    assert_eq!(as_text("text/plain", b"\x80\x81"), None);
}

/// A database made before `files.path`, `files.size` and `files.text` existed:
/// every column is added on open and the rows are sorted by content.
#[test]
fn startup_adds_missing_columns_to_an_old_schema_and_sorts_its_rows() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("causewaybay.sqlite3");
    let (text_id, png_id) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    {
        let conn = rusqlite::Connection::open(&database)?;
        conn.execute_batch(
            "CREATE TABLE files(id TEXT PRIMARY KEY,name TEXT NOT NULL,mime TEXT NOT NULL,bytes BLOB NOT NULL,created_at TEXT NOT NULL);",
        )?;
        conn.execute(
            "INSERT INTO files(id,name,mime,bytes,created_at) VALUES(?1,'first.txt','text/plain',?2,'2025-01-01T00:00:00Z')",
            params![text_id, b"from the first version".as_slice()],
        )?;
        conn.execute(
            "INSERT INTO files(id,name,mime,bytes,created_at) VALUES(?1,'first.png','image/png',?2,'2025-01-01T00:00:00Z')",
            params![png_id, PNG],
        )?;
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(files)")?
            .query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(!columns.iter().any(|c| c == "text" || c == "path"));
    }
    let db = Database::open(&database)?;
    {
        let conn = db.0.lock().unwrap();
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(files)")?
            .query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<_>>()?;
        for needed in ["path", "size", "owner_wallet", "text"] {
            assert!(columns.iter().any(|c| c == needed), "{needed}: {columns:?}");
        }
        let (path, text, size): (String, String, i64) = conn.query_row(
            "SELECT path,text,size FROM files WHERE id=?1",
            [&text_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        assert_eq!(
            (path.as_str(), text.as_str(), size),
            ("", "from the first version", 22)
        );
        let (path, text): (String, String) =
            conn.query_row("SELECT path,text FROM files WHERE id=?1", [&png_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        assert_eq!(path, format!("files/{png_id}/first.png"));
        assert!(text.is_empty());
    }
    assert_eq!(db.file(&text_id)?.1, b"from the first version");
    assert_eq!(
        std::fs::read(dir.path().join("files").join(&png_id).join("first.png"))?,
        PNG
    );
    assert_eq!(db.file(&png_id)?.1, PNG);
    Ok(())
}

/// The on-disk pass is careful: a row whose file is missing, a file too large
/// to load, and a declared binary document are left exactly as they are, and
/// the server still starts.
#[test]
fn migration_leaves_missing_large_and_binary_disk_files_alone() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("causewaybay.sqlite3");
    let created = chrono::Utc::now().to_rfc3339();
    let ids: Vec<String> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let (missing, large, pdf) = (&ids[0], &ids[1], &ids[2]);
    {
        let db = Database::open(&database)?;
        let conn = db.0.lock().unwrap();
        let insert = |id: &str, name: &str, mime: &str, size: i64| {
            conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet) VALUES(?1,?2,?3,x'',?4,?5,?6,'local')",params![id,name,mime,created,format!("files/{id}/{name}"),size])
        };
        insert(missing, "gone.txt", "text/plain", 4)?;
        let large_path = dir.path().join("files").join(large).join("huge.txt");
        std::fs::create_dir_all(large_path.parent().unwrap())?;
        std::fs::write(&large_path, "small on disk, declared huge")?;
        insert(large, "huge.txt", "text/plain", 30 * 1024 * 1024)?;
        let pdf_path = dir.path().join("files").join(pdf).join("tiny.pdf");
        std::fs::create_dir_all(pdf_path.parent().unwrap())?;
        std::fs::write(&pdf_path, "%PDF-1.4 ascii only")?;
        insert(pdf, "tiny.pdf", "application/pdf", 19)?;
    }
    let db = Database::open(&database)?;
    let conn = db.0.lock().unwrap();
    for (id, name) in [
        (missing, "gone.txt"),
        (large, "huge.txt"),
        (pdf, "tiny.pdf"),
    ] {
        let (path, text): (String, String) =
            conn.query_row("SELECT path,text FROM files WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        assert_eq!(path, format!("files/{id}/{name}"), "{name}");
        assert!(text.is_empty(), "{name}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("files").join(large).join("huge.txt"))?,
        "small on disk, declared huge"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("files").join(pdf).join("tiny.pdf"))?,
        "%PDF-1.4 ascii only"
    );
    Ok(())
}

/// The HTTP surface: an upload of text becomes a row, it is processed and
/// kept from SQLite, and a picture upload still goes to disk.
#[tokio::test]
async fn api_uploads_keep_text_in_sqlite_and_process_it_from_there() -> anyhow::Result<()> {
    use base64::Engine;
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let post = |path: String, body: Value| {
        let r = r.clone();
        async move {
            let res = r
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 2_000_000)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let (status, note) = post(
        "/api/files".into(),
        json!({"name":"meeting.md","data_base64":b64("# Meeting\n\nDecision: ORCHID-77 ships Friday.".as_bytes())}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{note}");
    assert_eq!(note["path"], "");
    assert_eq!(
        note["size"],
        "# Meeting\n\nDecision: ORCHID-77 ships Friday.".len()
    );
    let (status, picture) = post(
        "/api/files".into(),
        json!({"name":"dot.png","data_base64":b64(PNG)}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{picture}");
    assert!(picture["path"].as_str().unwrap().starts_with("files/"));
    let note_id = note["id"].as_str().unwrap().to_owned();
    // Processing reads the row, not a file.
    let (status, processed) = post(
        format!("/api/files/{note_id}/process"),
        json!({"input_mode":"text"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{processed}");
    assert!(processed.to_string().contains("ORCHID-77"), "{processed}");
    // Keeping it as a memo copies the words into the searchable memo text.
    let (status, memo) = post(
        "/api/memos".into(),
        json!({"title":"kept","content":"Notes from today","file_ids":[note_id]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{memo}");
    let memo_id = memo["id"].as_str().unwrap().to_owned();
    for _ in 0..500 {
        if s.db.get(&memo_id)?.unwrap().status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let kept = s.db.get(&memo_id)?.unwrap();
    assert_eq!(kept.status, "complete", "{:?}", kept.error);
    assert!(kept.input.contains("ORCHID-77"), "{}", kept.input);
    assert!(!s.db.lexical("ORCHID-77")?.is_empty());
    // Still nothing textual beside the database.
    assert_eq!(walk(&dir.path().join("files")).len(), 1);
    assert!(!dir.path().join("markdown").exists() && !dir.path().join("wiki").exists());
    Ok(())
}

/// Deleting a text row erases its words from the database file itself
/// (`secure_delete` is on), so a removed file cannot be recovered from SQLite.
#[tokio::test]
async fn deleting_a_text_row_leaves_no_trace_in_the_database_file() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open_with_credentials(dir.path().into(), true, None, Default::default())?;
    let r = app(s.clone(), "dist".into());
    let secret = "PRIVATE-TEXT-ROW-7f3a9c";
    let file =
        s.db.put_file("secret.txt", "text/plain", secret.as_bytes())?;
    assert_eq!(file.path, "");
    let (status, _, _) = raw(r.clone(), "DELETE", &format!("/api/files/{}", file.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(s.db.file_info(&file.id).is_err());
    s.db.0
        .lock()
        .unwrap()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    for name in ["causewaybay.sqlite3", "causewaybay.sqlite3-wal"] {
        let path = dir.path().join(name);
        if path.exists() {
            assert!(
                !std::fs::read(&path)?
                    .windows(secret.len())
                    .any(|w| w == secret.as_bytes()),
                "deleted text still in {name}"
            );
        }
    }
    Ok(())
}
