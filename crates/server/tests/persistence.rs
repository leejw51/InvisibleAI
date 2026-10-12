use causewaybay_server::db::Database;
use causewaybay_shared::config;
use serde_json::{Value, json};
use std::io::Write;

#[test]
fn jsonl_survives_interrupted_append_and_retains_identity() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let value = json!({"client_id":"stable-id","backend":"http://studio:3000"});
    config::append(dir.path(), "client", value.clone())?;
    config::append(dir.path(), "client", value.clone())?;
    let path = dir.path().join("client.jsonl");
    let text = std::fs::read_to_string(&path)?;
    assert_eq!(text.lines().count(), 2);
    for line in text.lines() {
        let _: Value = serde_json::from_str(line)?;
    }
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)?
        .write_all(b"{\"config\":")?;
    assert_eq!(config::read(dir.path(), "client")?, value);
    let next = json!({"client_id":"stable-id","backend":"http://new-studio:3000"});
    config::append(dir.path(), "client", next.clone())?;
    assert_eq!(config::read(dir.path(), "client")?, next);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
    }
    Ok(())
}

#[test]
fn capture_delivery_is_bound_to_client_and_disconnect_requeues() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db = Database::open(&dir.path().join("test.sqlite3"))?;
    db.client_hello("macbook", "MacBook", "macos")?;
    db.client_hello("other", "Other", "macos")?;
    let memo = db.create("capture", "capture", "grok", "test")?;
    let request = db.queue_capture("macbook", &memo.id, "screen", 1)?;
    assert!(db.claim_capture("other")?.is_none());
    assert_eq!(db.claim_capture("macbook")?.unwrap().request_id, request);
    assert!(db.claim_capture("macbook")?.is_none());
    assert!(
        db.complete_capture("other", &request, Some(b"image"), None)
            .is_err()
    );
    db.release_captures("macbook")?;
    assert_eq!(db.capture_status(&request)?.0, "queued");
    assert_eq!(db.claim_capture("macbook")?.unwrap().request_id, request);
    db.complete_capture("macbook", &request, Some(b"image"), None)?;
    assert_eq!(db.image(&memo.id)?.unwrap(), b"image");
    assert!(
        db.complete_capture("macbook", &request, Some(b"duplicate"), None)
            .is_err()
    );
    let cancelled = db.create("cancel", "capture", "grok", "test")?;
    let request = db.queue_capture("macbook", &cancelled.id, "screen", 1)?;
    db.claim_capture("macbook")?;
    db.fail(&cancelled.id, "Cancelled")?;
    assert!(
        db.complete_capture("macbook", &request, Some(b"late"), None)
            .is_err()
    );
    Ok(())
}

#[test]
fn workspace_keeps_images_on_disk_text_in_sqlite_and_deletes_memos() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db = Database::open(&dir.path().join("causewaybay.sqlite3"))?;
    let memo = db.create("What is 7 + 5?", "capture", "grok", "grok-4")?;
    let image = b"\x89PNG\r\n\x1a\nfixture";
    db.set_image(&memo.id, image)?;
    db.event(&memo.id, "delta", "12")?;
    db.finish(&memo.id, "question")?;
    let screenshot = dir
        .path()
        .join("screenshots")
        .join(format!("{}.png", memo.id));
    assert_eq!(std::fs::read(&screenshot)?, image);
    // Text is never mirrored to disk: the memo is its row, and so is a text file.
    assert!(!dir.path().join("markdown").exists());
    let first = db.put_file("answer.rs", "text/plain", b"pub fn a() {}")?;
    let second = db.put_file("answer.rs", "text/plain", b"pub fn b() {}")?;
    assert!(first.path.is_empty() && second.path.is_empty());
    assert_eq!(db.file(&first.id)?.1, b"pub fn a() {}");
    assert_eq!(db.file(&second.id)?.1, b"pub fn b() {}");
    assert!(!dir.path().join("files").exists());
    // A picture goes to disk, and SQLite keeps only its relative path.
    let picture = db.put_file("dot.png", "image/png", image)?;
    assert!(picture.path.starts_with("files/"));
    assert_eq!(std::fs::read(dir.path().join(&picture.path))?, image);
    assert_eq!(db.file(&picture.id)?.1, image);
    // Images on disk are authoritative; SQLite stores relative paths, never a duplicate blob.
    std::fs::remove_file(&screenshot)?;
    drop(db);
    let db = Database::open(&dir.path().join("causewaybay.sqlite3"))?;
    assert!(!screenshot.exists());
    assert!(db.image(&memo.id).is_err());
    assert!(
        db.image_path(&memo.id)?
            .unwrap()
            .starts_with("screenshots/")
    );
    let bytes: Option<Vec<u8>> =
        db.0.lock()
            .unwrap()
            .query_row("SELECT image FROM memos WHERE id=?1", [&memo.id], |r| {
                r.get(0)
            })?;
    assert!(bytes.is_none());
    db.delete(&memo.id)?;
    assert!(!screenshot.exists());
    Ok(())
}
