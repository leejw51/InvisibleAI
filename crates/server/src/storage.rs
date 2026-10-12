//! Startup storage migration. Text (memos, Wiki pages, uploaded documents and
//! code) lives in SQLite so one database holds everything the user wrote or
//! kept; only binary media (pictures, video, sound, PDFs, archives) is kept on
//! disk beside it. Older layouts are moved into place before serving.
use anyhow::Result;
use rusqlite::{Connection, params};
use std::path::Path;
pub fn migrate(conn: &Connection, root: &Path) -> Result<()> {
    for (table, column, definition) in [
        (
            "auth_sessions",
            "wallet_index",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("capture_clients", "owner_wallet", "TEXT"),
        ("memos", "owner_wallet", "TEXT NOT NULL DEFAULT 'local'"),
        ("files", "owner_wallet", "TEXT NOT NULL DEFAULT 'local'"),
        ("rooms", "owner_wallet", "TEXT NOT NULL DEFAULT 'local'"),
        ("rooms", "deleted", "INTEGER NOT NULL DEFAULT 0"),
        ("rooms", "cleared_through", "INTEGER NOT NULL DEFAULT 0"),
        ("rooms", "updated_at", "TEXT NOT NULL DEFAULT ''"),
        (
            "web_clients",
            "owner_wallet",
            "TEXT NOT NULL DEFAULT 'local'",
        ),
        ("capture_requests", "requester", "TEXT"),
        ("memos", "image_path", "TEXT"),
        ("memos", "roomid", "TEXT"),
        ("files", "roomid", "TEXT"),
        ("files", "origin_memo", "TEXT"),
        ("memos", "origin_memo", "TEXT"),
        ("compilations", "roomid", "TEXT"),
        ("files", "path", "TEXT"),
        ("files", "size", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !columns.iter().any(|c| c == column) {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
    }
    let images = conn
        .prepare("SELECT id,image FROM memos WHERE image IS NOT NULL")?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, bytes) in images {
        let path = crate::artifacts::memo_path(
            root,
            "screenshots",
            &id,
            crate::artifacts::image_extension(&bytes),
        )?;
        crate::artifacts::write(&path, &bytes)?;
        conn.execute(
            "UPDATE memos SET image_path=?2,image=NULL WHERE id=?1",
            params![id, path.strip_prefix(root)?.to_string_lossy()],
        )?;
    }
    // Rows from before paths existed hold their bytes: text stays in SQLite
    // (path ''), binary media moves to disk.
    let files = conn
        .prepare("SELECT id,name,mime,bytes FROM files WHERE path IS NULL")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, name, mime, bytes) in files {
        if let Some(text) = crate::db::as_text(&mime, &bytes) {
            conn.execute(
                "UPDATE files SET path='',size=?2,text=?3,bytes=x'' WHERE id=?1",
                params![id, bytes.len(), text],
            )?;
            continue;
        }
        let path = crate::artifacts::file_path(root, &id, &name)?;
        crate::artifacts::write(&path, &bytes)?;
        conn.execute(
            "UPDATE files SET path=?2,size=?3,bytes=x'' WHERE id=?1",
            params![id, path.strip_prefix(root)?.to_string_lossy(), bytes.len()],
        )?;
    }
    // Text files an earlier version wrote to disk come back into SQLite.
    let on_disk = conn
        .prepare("SELECT id,name,mime,path,size FROM files WHERE path IS NOT NULL AND path!='' AND mime NOT LIKE 'image/%' AND mime NOT LIKE 'video/%' AND mime NOT LIKE 'audio/%' AND mime NOT IN ('application/pdf','application/zip','application/gzip','application/msword') AND mime NOT LIKE 'application/vnd.%' AND size<=?1")?
        .query_map([crate::db::MAX_LOADED_FILE as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, _name, mime, relative, _) in on_disk {
        let Ok(path) = crate::artifacts::resolve(root, &relative) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some(text) = crate::db::as_text(&mime, &bytes) else {
            continue;
        };
        conn.execute(
            "UPDATE files SET text=?2,path='',size=?3 WHERE id=?1",
            params![id, text, bytes.len()],
        )?;
        let _ = std::fs::remove_file(&path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
    // Memo and Wiki Markdown mirrors were derived copies of what SQLite holds;
    // nothing reads them any more.
    for folder in ["markdown", "wiki"] {
        let dir = root.join(folder);
        if dir.is_dir() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS memos_owner ON memos(owner_wallet); CREATE INDEX IF NOT EXISTS files_owner ON files(owner_wallet); CREATE INDEX IF NOT EXISTS rooms_owner ON rooms(owner_wallet);")?;
    conn.execute_batch("PRAGMA secure_delete=ON;
        UPDATE memos SET roomid=(SELECT roomid FROM chat_messages WHERE memo_id=memos.id LIMIT 1) WHERE roomid IS NULL;
        UPDATE files SET roomid=(SELECT c.roomid FROM chat_messages c,json_each(c.document,'$.attachments') a WHERE json_extract(a.value,'$.id')=files.id LIMIT 1) WHERE roomid IS NULL;")?;
    conn.execute(
        "INSERT INTO memo_fts(memo_fts,rank) VALUES('secure-delete',1)",
        [],
    )?;
    // Replaying the latest state is sufficient for offline synchronization.
    // Remove superseded message bodies left by earlier versions.
    conn.execute("DELETE FROM message_changes WHERE messagesyncid NOT IN (SELECT MAX(messagesyncid) FROM message_changes GROUP BY messageid)", [])?;
    let submissions = conn
        .prepare(
            "SELECT messageid,request FROM chat_submissions WHERE request NOT LIKE 'sha256:%'",
        )?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, request) in submissions {
        conn.execute(
            "UPDATE chat_submissions SET request=?2 WHERE messageid=?1",
            params![id, crate::chat::submission_digest(&request)],
        )?;
    }
    // Recover generated-output ownership from legacy tool records before those
    // records are erased by a message edit or deletion.
    let outputs = conn
        .prepare("SELECT memo_id,text FROM events WHERE kind='tool_result'")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (memo, text) in outputs {
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(output) = event["output"].as_str()
            && let Ok(output) = serde_json::from_str::<serde_json::Value>(output)
        {
            if matches!(event["name"].as_str(), Some("write_file" | "edit_file"))
                && let Some(id) = output["file"]["id"].as_str()
            {
                conn.execute("UPDATE files SET origin_memo=?2 WHERE id=?1 AND origin_memo IS NULL AND owner_wallet=(SELECT owner_wallet FROM memos WHERE id=?2)",params![id,memo])?;
            }
            if event["name"] == "save_memo"
                && let Some(id) = output["saved"].as_str()
            {
                conn.execute("UPDATE memos SET origin_memo=?2 WHERE id=?1 AND origin_memo IS NULL AND owner_wallet=(SELECT owner_wallet FROM memos WHERE id=?2)",params![id,memo])?;
            }
        }
    }
    conn.pragma_update(None, "user_version", 5)?;
    Ok(())
}
