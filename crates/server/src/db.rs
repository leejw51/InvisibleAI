use anyhow::{Context, Result};
use causewaybay_shared::{FileInfo, Memo, StreamEvent};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// The largest file the backend reads whole into memory (the non-video upload limit).
pub const MAX_LOADED_FILE: u64 = 25 * 1024 * 1024;
#[derive(Clone)]
pub struct Database(
    pub Arc<Mutex<Connection>>,
    pub(crate) PathBuf,
    pub(crate) Option<String>,
);

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Database {
    pub fn for_owner(&self, owner: &str) -> Self {
        Self(self.0.clone(), self.1.clone(), Some(owner.into()))
    }
    pub fn owner(&self) -> &str {
        self.2.as_deref().unwrap_or("local")
    }
    pub fn writable(&self, conn: &Connection) -> Result<()> {
        let deleting: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_purges WHERE owner_wallet=?1)",
            [self.owner()],
            |r| r.get(0),
        )?;
        anyhow::ensure!(!deleting, "Your data is being deleted. Please wait.");
        Ok(())
    }
    pub fn require(&self, table: &str, id: &str) -> Result<()> {
        let key = match table {
            "rooms" => "roomid",
            "memos" | "files" => "id",
            _ => anyhow::bail!("Invalid resource"),
        };
        let exists: bool = self.0.lock().unwrap().query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key}=?1 AND (?2 IS NULL OR owner_wallet=?2))"),
            params![id, self.2], |r|r.get(0))?;
        anyhow::ensure!(exists, "Resource not found");
        Ok(())
    }

    pub fn assign_room(&self, table: &str, id: &str, room: &str) -> Result<()> {
        self.require(table, id)?;
        crate::chat::get_room(self, room)?;
        anyhow::ensure!(matches!(table, "memos" | "files"), "Invalid resource");
        let changed = self.0.lock().unwrap().execute(
            &format!("UPDATE {table} SET roomid=?2 WHERE id=?1 AND (roomid IS NULL OR roomid=?2)"),
            params![id, room],
        )?;
        anyhow::ensure!(
            changed == 1,
            "This file belongs to another chatroom. Upload a separate copy."
        );
        Ok(())
    }
    /// Claim the entire set atomically: an invalid last file cannot move earlier files.
    pub fn assign_files_to_room(&self, ids: &[&str], room: &str) -> Result<()> {
        crate::chat::get_room(self, room)?;
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        for id in ids {
            let changed = tx.execute(
                "UPDATE files SET roomid=?2 WHERE id=?1 AND (roomid IS NULL OR roomid=?2) AND (?3 IS NULL OR owner_wallet=?3)",
                params![id, room, self.2],
            )?;
            anyhow::ensure!(
                changed == 1,
                "File unavailable or belongs to another chatroom. Upload a separate copy."
            );
        }
        tx.commit()?;
        Ok(())
    }
    pub fn room_for_memo(&self, memo: &str) -> Result<Option<String>> {
        self.require("memos", memo)?;
        Ok(self.0.lock().unwrap().query_row(
            "SELECT roomid FROM memos WHERE id=?1",
            [memo],
            |r| r.get(0),
        )?)
    }
    /// Link a file or memo an agent tool created to the answer that created it,
    /// and file it in that answer's room.
    pub fn set_origin(&self, table: &str, id: &str, memo: &str) -> Result<()> {
        anyhow::ensure!(matches!(table, "files" | "memos"), "Invalid resource");
        self.0.lock().unwrap().execute(
            &format!("UPDATE {table} SET origin_memo=?2 WHERE id=?1"),
            params![id, memo],
        )?;
        if let Some(room) = self.room_for_memo(memo)? {
            self.assign_room(table, id, &room)?;
        }
        Ok(())
    }
    pub fn save_compilation(&self, file: &str, result: &serde_json::Value) -> Result<()> {
        self.save_compilation_for(file, result, None)
    }
    pub fn save_compilation_for(
        &self,
        file: &str,
        result: &serde_json::Value,
        room: Option<&str>,
    ) -> Result<()> {
        self.require("files", file)?;
        if let Some(room) = room {
            crate::chat::get_room(self, room)?;
        }
        self.0.lock().unwrap().execute(
            "INSERT INTO compilations(file_id,result,created_at,roomid) VALUES(?1,?2,?3,COALESCE(?4,(SELECT roomid FROM files WHERE id=?1)))",
            params![file, result.to_string(), now(), room],
        )?;
        Ok(())
    }
    pub fn last_compilation(&self, file: &str) -> Result<Option<serde_json::Value>> {
        self.require("files", file)?;
        let text: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT result FROM compilations WHERE file_id=?1 ORDER BY id DESC LIMIT 1",
                [file],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Every table is declared once; missing tables and columns are added
        // automatically, so a changed definition migrates an existing database.
        crate::schema::ensure(&conn, include_str!("schema.sql"))?;
        crate::storage::migrate(&conn, path.parent().unwrap_or_else(|| Path::new(".")))?;
        // A new process has no surviving sockets, even if the last heartbeat was recent.
        conn.execute("UPDATE web_clients SET connected=0", [])?;
        crate::invisible_sessions::init(&conn)?;
        crate::usage::init(&conn)?;
        crate::uploads::init(&conn)?;
        crate::activity::init(&conn)?;
        crate::wiki::init(&conn)?;
        crate::code::init(&conn)?;
        crate::coding::init(&conn)?;
        crate::schema::run_versioned(&conn)?;
        conn.execute(
            "UPDATE ai_usage SET status='interrupted' WHERE status='running'",
            [],
        )?;
        conn.execute("UPDATE capture_clients SET last_seen=0", [])?;
        let root = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let db = Self(Arc::new(Mutex::new(conn)), root, None);
        let pending =
            db.0.lock()
                .unwrap()
                .prepare("SELECT roomid,owner_wallet,deleted FROM rooms WHERE deleted IN (2,3)")?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        for (room, owner, state) in pending {
            crate::chat::finish_purge(&db.for_owner(&owner), &room, state == 3)?;
        }
        let owners =
            db.0.lock()
                .unwrap()
                .prepare("SELECT owner_wallet FROM user_purges")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        for owner in owners {
            crate::chat::finish_user_purge(&db.for_owner(&owner))?;
        }
        let interrupted = db
            .all()?
            .into_iter()
            .filter(|m| m.status == "running")
            .collect::<Vec<_>>();
        for memo in interrupted {
            db.fail(
                &memo.id,
                "The server restarted during this response. Your partial answer was saved.",
            )?;
        }
        crate::chat::restore_history(&db)?;
        crate::wiki::restore(&db)?;
        Ok(db)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])?;
        Ok(())
    }
    pub fn create(&self, input: &str, kind: &str, provider: &str, model: &str) -> Result<Memo> {
        let memo = Memo {
            id: id(),
            title: if input.is_empty() {
                "Screen capture".into()
            } else {
                input
                    .lines()
                    .next()
                    .unwrap_or(input)
                    .chars()
                    .take(80)
                    .collect()
            },
            input: input.into(),
            kind: kind.into(),
            provider: provider.into(),
            model: model.into(),
            status: "running".into(),
            created_at: now(),
            tags: extract_tags(input),
            ..Memo::default()
        };
        let conn = self.0.lock().unwrap();
        self.writable(&conn)?;
        conn.execute("INSERT INTO memos(id,title,input,kind,provider,model,status,created_at,tags,owner_wallet) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![memo.id,memo.title,memo.input,memo.kind,memo.provider,memo.model,memo.status,memo.created_at,serde_json::to_string(&memo.tags)?,self.owner()])?;
        Ok(memo)
    }
    pub fn get(&self, id: &str) -> Result<Option<Memo>> {
        Ok(self.0.lock().unwrap().query_row("SELECT id,title,input,answer,kind,tags,provider,model,status,created_at,pinned,image_path IS NOT NULL,error FROM memos WHERE id=?1 AND (?2 IS NULL OR owner_wallet=?2)",params![id,self.2], memo_row).optional()?)
    }
    pub fn all(&self) -> Result<Vec<Memo>> {
        let conn = self.0.lock().unwrap();
        let mut stmt=conn.prepare("SELECT id,title,input,answer,kind,tags,provider,model,status,created_at,pinned,image_path IS NOT NULL,error FROM memos WHERE (?1 IS NULL OR owner_wallet=?1) ORDER BY created_at DESC")?;
        Ok(stmt
            .query_map([&self.2], memo_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn count(&self) -> Result<usize> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM memos", [], |r| r.get(0))?)
    }
    pub fn set_input(&self, id: &str, text: &str) -> Result<()> {
        self.require("memos", id)?;
        let title: String = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Screen capture")
            .chars()
            .take(80)
            .collect();
        self.0.lock().unwrap().execute(
            "UPDATE memos SET input=?2,title=?3,tags=?4 WHERE id=?1",
            params![id, text, title, serde_json::to_string(&extract_tags(text))?],
        )?;
        Ok(())
    }
    pub fn set_image(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let conn = self.0.lock().unwrap();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memos WHERE id=?1 AND (?2 IS NULL OR owner_wallet=?2))",
            params![id, self.2],
            |r| r.get(0),
        )?;
        anyhow::ensure!(exists, "Resource not found");
        let path = self.persist_image(id, bytes)?;
        conn.execute(
            "UPDATE memos SET image=NULL,image_path=?2 WHERE id=?1",
            params![id, path],
        )?;
        Ok(())
    }
    pub fn persist_image(&self, id: &str, bytes: &[u8]) -> Result<String> {
        let path = crate::artifacts::memo_path(
            &self.1,
            "screenshots",
            id,
            crate::artifacts::image_extension(bytes),
        )?;
        crate::artifacts::write(&path, bytes)?;
        Ok(path.strip_prefix(&self.1)?.to_string_lossy().into_owned())
    }
    pub fn image_path(&self, id: &str) -> Result<Option<String>> {
        self.require("memos", id)?;
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT image_path FROM memos WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten())
    }
    /// Bring the owner's Wiki up to date after a memo changed (its tags,
    /// text, answer, status or picture): the tag documents are derived rows.
    pub fn sync_wiki(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().unwrap();
        let owner: Option<String> = conn
            .query_row(
                "SELECT owner_wallet FROM memos WHERE id=?1 AND (?2 IS NULL OR owner_wallet=?2)",
                params![id, self.2],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(owner) = owner {
            crate::wiki::sync_locked(&conn, &owner, false)?;
        }
        Ok(())
    }
    pub fn image(&self, id: &str) -> Result<Option<Vec<u8>>> {
        self.image_path(id)?
            .map(|path| {
                std::fs::read(crate::artifacts::resolve(&self.1, &path)?)
                    .context("Stored screenshot is missing from the backend persistence folder")
            })
            .transpose()
    }
    pub fn event(&self, memo: &str, kind: &str, text: &str) -> Result<StreamEvent> {
        self.require("memos", memo)?;
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO events(memo_id,kind,text) VALUES(?1,?2,?3)",
            params![memo, kind, text],
        )?;
        let event = StreamEvent {
            id: tx.last_insert_rowid(),
            kind: kind.into(),
            text: text.into(),
        };
        if kind == "delta" {
            tx.execute(
                "UPDATE memos SET answer=answer||?2 WHERE id=?1",
                params![memo, text],
            )?;
        }
        if kind == "stage" {
            crate::activity::log_memo_stage(&tx, memo, text)?;
        }
        crate::chat::refresh(&tx, memo, kind, text)?;
        tx.commit()?;
        Ok(event)
    }
    pub fn events(&self, memo: &str, after: i64) -> Result<Vec<StreamEvent>> {
        self.require("memos", memo)?;
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,kind,text FROM events WHERE memo_id=?1 AND id>?2 ORDER BY id LIMIT 256",
        )?;
        Ok(stmt
            .query_map(params![memo, after], |r| {
                Ok(StreamEvent {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    text: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn finish(&self, id: &str, kind: &str) -> Result<()> {
        self.require("memos", id)?;
        self.0.lock().unwrap().execute(
            "UPDATE memos SET status='complete',kind=?2 WHERE id=?1",
            params![id, kind],
        )?;
        self.event(id, "kind", kind)?;
        self.sync_wiki(id)?;
        self.event(id, "done", "Saved to your library")?;
        let owner: String = self.0.lock().unwrap().query_row(
            "SELECT owner_wallet FROM memos WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        let room = self.room_for_memo(id)?;
        crate::notifications::wallet(
            &self.for_owner(&owner),
            room.as_deref(),
            "answer.ready",
            serde_json::json!({"memo_id":id,"roomid":room}),
        )?;
        Ok(())
    }
    pub fn fail(&self, id: &str, error: &str) -> Result<()> {
        self.require("memos", id)?;
        self.0.lock().unwrap().execute(
            "UPDATE memos SET status='error',error=?2 WHERE id=?1",
            params![id, error],
        )?;
        self.event(id, "error", error)?;
        self.sync_wiki(id)?;
        Ok(())
    }
    pub fn update(
        &self,
        id: &str,
        title: Option<&str>,
        tags: Option<&[String]>,
        pinned: Option<bool>,
    ) -> Result<()> {
        self.require("memos", id)?;
        let conn = self.0.lock().unwrap();
        if let Some(v) = title {
            conn.execute("UPDATE memos SET title=?2 WHERE id=?1", params![id, v])?;
        }
        if let Some(v) = tags {
            conn.execute(
                "UPDATE memos SET tags=?2 WHERE id=?1",
                params![id, serde_json::to_string(v)?],
            )?;
        }
        if let Some(v) = pinned {
            conn.execute("UPDATE memos SET pinned=?2 WHERE id=?1", params![id, v])?;
        }
        drop(conn);
        self.sync_wiki(id)?;
        Ok(())
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        self.require("memos", id)?;
        if self.get(id)?.is_some() {
            for (folder, extension) in [
                ("thumbnails", "jpg"),
                ("screenshots", "png"),
                ("screenshots", "jpg"),
                ("screenshots", "webp"),
                ("screenshots", "bin"),
            ] {
                let path = crate::artifacts::memo_path(&self.1, folder, id, extension)?;
                if let Err(e) = std::fs::remove_file(path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(e.into());
                }
            }
        }
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        crate::chat::tombstone_memo(&tx, id)?;
        let owner: String =
            tx.query_row("SELECT owner_wallet FROM memos WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        tx.execute("INSERT INTO notifications(client_id,kind,data) SELECT client_id,'capture.remove',json_object('request_id',id) FROM capture_requests WHERE memo_id=?1", [id])?;
        tx.execute(
            "DELETE FROM notifications WHERE json_extract(data,'$.memo_id')=?1",
            [id],
        )?;
        tx.execute("DELETE FROM memos WHERE id=?1", [id])?;
        crate::wiki::sync_locked(&tx, &owner, false)?;
        tx.commit()?;
        Ok(())
    }
    pub fn lexical(&self, query: &str) -> Result<Vec<String>> {
        // Quote individual terms, never interpolate raw FTS syntax from the search box.
        let terms: Vec<_> = query
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty())
            .take(32)
            .flat_map(|s| [Some(s), korean_stem(s)])
            .flatten()
            .map(|s| format!("\"{}\"*", s.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.0.lock().unwrap();
        let mut stmt=conn.prepare("SELECT m.id FROM memo_fts f JOIN memos m ON m.rowid=f.rowid WHERE memo_fts MATCH ?1 AND (?2 IS NULL OR m.owner_wallet=?2) ORDER BY bm25(memo_fts,3.0,1.0,1.0,2.0) LIMIT 200")?;
        Ok(stmt
            .query_map(params![terms.join(" OR "), self.2], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn put_embedding(&self, id: &str, vector: &[f32]) -> Result<()> {
        let bytes: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.0.lock().unwrap().execute("INSERT INTO embeddings(memo_id,model,vector) VALUES(?1,?3,?2) ON CONFLICT(memo_id) DO UPDATE SET model=excluded.model,vector=excluded.vector",params![id,bytes,crate::semantic::MODEL_NAME])?;
        Ok(())
    }
    /// Background inference may finish after a message was edited or erased.
    pub fn put_embedding_if_current(&self, memo: &Memo, vector: &[f32]) -> Result<()> {
        let bytes: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.0.lock().unwrap().execute(
            "INSERT INTO embeddings(memo_id,model,vector) SELECT id,?7,?2 FROM memos WHERE id=?1 AND title=?3 AND input=?4 AND answer=?5 AND status='complete' AND (?6 IS NULL OR owner_wallet=?6) ON CONFLICT(memo_id) DO UPDATE SET model=excluded.model,vector=excluded.vector",
            params![
                memo.id,
                bytes,
                memo.title,
                memo.input,
                memo.answer,
                self.2,
                crate::semantic::MODEL_NAME
            ],
        )?;
        Ok(())
    }
    /// The stored vector of one memo, if the indexer has reached it.
    pub fn embedding(&self, id: &str) -> Result<Option<Vec<f32>>> {
        let conn = self.0.lock().unwrap();
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT e.vector FROM embeddings e JOIN memos m ON m.id=e.memo_id WHERE e.memo_id=?1 AND e.model=?3 AND (?2 IS NULL OR m.owner_wallet=?2)",
                params![id, self.2, crate::semantic::MODEL_NAME],
                |r| r.get(0),
            )
            .optional()?;
        Ok(bytes.map(|b| {
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect()
        }))
    }
    pub fn semantic(&self, query: &[f32]) -> Result<Vec<String>> {
        Ok(self
            .semantic_scored(query)?
            .into_iter()
            .map(|x| x.0)
            .collect())
    }
    /// Memo ids with their cosine similarity, best first, above a loose floor.
    pub fn semantic_scored(&self, query: &[f32]) -> Result<Vec<(String, f32)>> {
        let conn = self.0.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT e.memo_id,e.vector FROM embeddings e JOIN memos m ON m.id=e.memo_id WHERE e.model=?2 AND (?1 IS NULL OR m.owner_wallet=?1)")?;
        let mut scores = stmt
            .query_map(params![self.2, crate::semantic::MODEL_NAME], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter_map(|(id, bytes)| {
                let vector = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect::<Vec<_>>();
                let score = cosine(query, &vector);
                (score > 0.25).then_some((id, score))
            })
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        scores.truncate(200);
        Ok(scores)
    }
    pub fn unindexed(&self) -> Result<Vec<Memo>> {
        let conn = self.0.lock().unwrap();
        let mut stmt=conn.prepare("SELECT id,title,input,answer,kind,tags,provider,model,status,created_at,pinned,image_path IS NOT NULL,error FROM memos WHERE status='complete' AND id NOT IN (SELECT memo_id FROM embeddings WHERE model=?1) LIMIT 50")?;
        Ok(stmt
            .query_map([crate::semantic::MODEL_NAME], memo_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// Keep a file. Text (documents, code, Markdown, JSON, CSV: anything that
    /// reads as UTF-8) is stored in SQLite's `files.text` column, so one
    /// database holds every word the user kept and it can be searched; binary
    /// media (pictures, video, sound, PDFs, archives) goes under
    /// `files/<id>/<name>` and SQLite keeps its path.
    pub fn put_file(&self, name: &str, mime: &str, bytes: &[u8]) -> Result<FileInfo> {
        let conn = self.0.lock().unwrap();
        self.writable(&conn)?;
        let mut file = FileInfo {
            id: id(),
            path: String::new(),
            name: name.into(),
            mime: mime.into(),
            size: bytes.len(),
            created_at: now(),
        };
        if let Some(text) = as_text(mime, bytes) {
            conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet,text) VALUES(?1,?2,?3,x'',?4,'',?5,?6,?7)",params![file.id,file.name,file.mime,file.created_at,file.size,self.owner(),text])?;
            return Ok(file);
        }
        let path = crate::artifacts::file_path(&self.1, &file.id, name)?;
        crate::artifacts::write(&path, bytes)?;
        file.path = path.strip_prefix(&self.1)?.to_string_lossy().into_owned();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet) VALUES(?1,?2,?3,x'',?4,?5,?6,?7)",params![file.id,file.name,file.mime,file.created_at,file.path,file.size,self.owner()])?;
        Ok(file)
    }
    /// Store a file that is already on disk (a finished chunked upload) by
    /// moving it under the persistence folder. Only its path goes into SQLite.
    pub fn put_file_from(
        &self,
        name: &str,
        mime: &str,
        source: &std::path::Path,
        size: u64,
    ) -> Result<FileInfo> {
        let conn = self.0.lock().unwrap();
        self.writable(&conn)?;
        let mut file = FileInfo {
            id: id(),
            path: String::new(),
            name: name.into(),
            mime: mime.into(),
            size: size as usize,
            created_at: now(),
        };
        let path = crate::artifacts::file_path(&self.1, &file.id, name)?;
        crate::artifacts::private_dir(path.parent().context("Missing file directory")?)?;
        std::fs::rename(source, &path)
            .with_context(|| format!("Could not save {}", path.display()))?;
        file.path = path.strip_prefix(&self.1)?.to_string_lossy().into_owned();
        conn.execute("INSERT INTO files(id,name,mime,bytes,created_at,path,size,owner_wallet) VALUES(?1,?2,?3,x'',?4,?5,?6,?7)",params![file.id,file.name,file.mime,file.created_at,file.path,file.size,self.owner()])?;
        Ok(file)
    }
    pub fn files(&self) -> Result<Vec<FileInfo>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,name,mime,size,created_at,path FROM files WHERE (?1 IS NULL OR owner_wallet=?1) ORDER BY created_at DESC",
        )?;
        Ok(stmt
            .query_map([&self.2], file_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// The message a stored file belongs to: the memo it was kept or generated
    /// for, else the chat message it was attached to. None for a loose file.
    pub fn file_memo(&self, id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().unwrap();
        let origin: Option<String> = conn
            .query_row(
                "SELECT origin_memo FROM files WHERE id=?1 AND (?2 IS NULL OR owner_wallet=?2)",
                params![id, self.2],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if origin.is_some() {
            return Ok(origin);
        }
        Ok(conn
            .query_row(
                "SELECT c.memo_id FROM chat_messages c,json_each(c.document,'$.attachments') a JOIN memos m ON m.id=c.memo_id WHERE json_extract(a.value,'$.id')=?1 AND json_extract(c.document,'$.deleted')=0 AND c.memo_id!='' AND (?2 IS NULL OR m.owner_wallet=?2) ORDER BY c.rowid DESC LIMIT 1",
                params![id, self.2],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn file_info(&self, id: &str) -> Result<FileInfo> {
        self.require("files", id)?;
        self.0
            .lock()
            .unwrap()
            .query_row(
                "SELECT id,name,mime,size,created_at,path FROM files WHERE id=?1",
                [id],
                file_row,
            )
            .context("File not found")
    }
    /// A stored file's metadata and its location under the persistence folder.
    /// A text file has no location: its bytes are in SQLite (see [`Self::file`]).
    pub fn file_location(&self, id: &str) -> Result<(FileInfo, std::path::PathBuf)> {
        let info = self.file_info(id)?;
        anyhow::ensure!(
            !info.path.is_empty(),
            "This file is stored in the database, not on disk"
        );
        let path = crate::artifacts::resolve(&self.1, &info.path)?;
        Ok((info, path))
    }
    /// A stored file read into memory: a text file from SQLite, a binary one
    /// from disk. Large videos are refused here: they are streamed, downloaded,
    /// and previewed straight from disk instead.
    pub fn file(&self, id: &str) -> Result<(FileInfo, Vec<u8>)> {
        let info = self.file_info(id)?;
        if info.path.is_empty() {
            let text: String = self.0.lock().unwrap().query_row(
                "SELECT text FROM files WHERE id=?1",
                [id],
                |r| r.get(0),
            )?;
            return Ok((info, text.into_bytes()));
        }
        let path = crate::artifacts::resolve(&self.1, &info.path)?;
        let size = std::fs::metadata(&path)
            .context("Stored file is missing from the backend persistence folder")?
            .len();
        anyhow::ensure!(
            size <= MAX_LOADED_FILE,
            "This file is too large to open in memory; stream or download it instead"
        );
        let bytes = std::fs::read(&path)
            .context("Stored file is missing from the backend persistence folder")?;
        Ok((info, bytes))
    }
}

/// A file's content as text when it is text, and therefore lives in SQLite's
/// `files.text` rather than on disk: bytes that read as UTF-8 without NUL
/// characters and are not declared as a picture, video or sound. Anything
/// else (media, PDFs, archives, UTF-16) is binary and keeps its exact bytes
/// on disk.
pub fn as_text<'a>(mime: &str, bytes: &'a [u8]) -> Option<&'a str> {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Declared media and document containers are binary even when a small
    // one happens to be ASCII (a tiny PDF, for instance).
    if mime.starts_with("image/")
        || mime.starts_with("video/")
        || mime.starts_with("audio/")
        || mime.starts_with("application/vnd.")
        || matches!(
            mime.as_str(),
            "application/pdf" | "application/zip" | "application/gzip" | "application/msword"
        )
    {
        return None;
    }
    std::str::from_utf8(bytes)
        .ok()
        .filter(|s| !s.contains('\0'))
        .filter(|s| !s.is_empty() || mime.starts_with("text/"))
}
fn file_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileInfo> {
    Ok(FileInfo {
        id: r.get(0)?,
        name: r.get(1)?,
        mime: r.get(2)?,
        size: r.get(3)?,
        created_at: r.get(4)?,
        path: r.get(5)?,
    })
}
fn memo_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Memo> {
    Ok(Memo {
        id: r.get(0)?,
        title: r.get(1)?,
        input: r.get(2)?,
        answer: r.get(3)?,
        kind: r.get(4)?,
        tags: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        provider: r.get(6)?,
        model: r.get(7)?,
        status: r.get(8)?,
        created_at: r.get(9)?,
        pinned: r.get(10)?,
        has_image: r.get(11)?,
        error: r.get(12)?,
        usage: None,
    })
}

/// Korean attaches particles to words ("전화번호는" = "phone number" + topic
/// marker), so a question rarely shares a whole token with the note. The stem
/// without one trailing particle is searched as a prefix too.
fn korean_stem(word: &str) -> Option<&str> {
    const PARTICLES: [&str; 14] = [
        "은", "는", "이", "가", "을", "를", "의", "에", "도", "로", "와", "과", "야", "요",
    ];
    let chars = word.chars().count();
    if chars < 3 || !word.chars().all(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)) {
        return None;
    }
    PARTICLES
        .iter()
        .find_map(|p| word.strip_suffix(p))
        .filter(|stem| !stem.is_empty())
}
pub fn extract_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = text
        .split_whitespace()
        .filter_map(|s| s.strip_prefix('#'))
        .map(|s| {
            s.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .to_lowercase()
        })
        .filter(|s| !s.is_empty())
        .take(20)
        .collect();
    tags.sort();
    tags.dedup();
    tags
}
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    let norm = (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if norm > 0.0 { dot / norm } else { 0.0 }
}
pub fn fuse(lexical: Vec<String>, semantic: Vec<String>) -> Vec<String> {
    let mut scores = HashMap::<String, f64>::new();
    for list in [lexical, semantic] {
        for (rank, id) in list.into_iter().enumerate() {
            *scores.entry(id).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
        }
    }
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().map(|v| v.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fts_tracks_edits_and_deletes() -> Result<()> {
        let db = Database::open(Path::new(":memory:"))?;
        let m = db.create("Rust ownership #rust", "code", "test", "test")?;
        db.event(&m.id, "delta", "Borrowing keeps references safe")?;
        assert_eq!(db.lexical("borrowing")?, vec![m.id.clone()]);
        assert!(db.lexical("\" OR * : ( )")?.is_empty());
        db.set_input(&m.id, "Python iteration #python")?;
        assert!(db.lexical("ownership")?.is_empty());
        db.delete(&m.id)?;
        db.put_embedding_if_current(&m, &[1., 0.])?;
        assert!(db.semantic(&[1., 0.])?.is_empty());
        assert!(db.events(&m.id, 0).is_err());
        assert!(db.lexical("borrowing")?.is_empty());
        Ok(())
    }
    #[test]
    fn events_are_replayable_and_answer_is_durable() -> Result<()> {
        let db = Database::open(Path::new(":memory:"))?;
        let m = db.create("hello", "question", "test", "test")?;
        let a = db.event(&m.id, "delta", "안녕")?;
        db.event(&m.id, "delta", " world")?;
        assert_eq!(db.get(&m.id)?.unwrap().answer, "안녕 world");
        assert_eq!(db.events(&m.id, a.id)?.len(), 1);
        Ok(())
    }
    #[test]
    fn semantic_and_rrf() -> Result<()> {
        let db = Database::open(Path::new(":memory:"))?;
        let m = db.create("dogs", "memo", "test", "test")?;
        db.put_embedding(&m.id, &[1., 0.])?;
        assert_eq!(db.semantic(&[0.9, 0.1])?, vec![m.id]);
        assert_eq!(fuse(vec!["a".into(), "b".into()], vec!["b".into()])[0], "b");
        assert_eq!(cosine(&[], &[]), 0.);
        Ok(())
    }
    #[test]
    fn korean_questions_match_notes_despite_particles() -> Result<()> {
        let db = Database::open(Path::new(":memory:"))?;
        let m = db.create("내 전화번호 +82 10 1234 5678", "memo", "local", "")?;
        // "전화번호는" carries a topic particle the note does not.
        assert_eq!(db.lexical("내 전화번호는?")?, vec![m.id.clone()]);
        assert_eq!(db.lexical("전화번호")?, vec![m.id]);
        assert_eq!(korean_stem("전화번호는"), Some("전화번호"));
        assert_eq!(korean_stem("나는"), None);
        assert_eq!(korean_stem("phones"), None);
        Ok(())
    }
    #[test]
    fn vectors_from_an_older_model_are_reindexed_not_compared() -> Result<()> {
        let db = Database::open(Path::new(":memory:"))?;
        let m = db.create("dogs", "memo", "test", "test")?;
        db.finish(&m.id, "memo")?;
        let bytes: Vec<u8> = [1f32, 0.].iter().flat_map(|x| x.to_le_bytes()).collect();
        db.0.lock().unwrap().execute(
            "INSERT INTO embeddings(memo_id,model,vector) VALUES(?1,'all-MiniLM-L6-v2',?2)",
            params![m.id, bytes],
        )?;
        assert!(db.semantic(&[1., 0.])?.is_empty());
        assert!(db.unindexed()?.iter().any(|u| u.id == m.id));
        db.put_embedding(&m.id, &[1., 0.])?;
        assert_eq!(db.semantic(&[1., 0.])?, vec![m.id.clone()]);
        assert!(!db.unindexed()?.iter().any(|u| u.id == m.id));
        Ok(())
    }
}
