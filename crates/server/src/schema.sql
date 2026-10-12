PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS auth_sessions(token_hash TEXT PRIMARY KEY,address TEXT,expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS users(address TEXT PRIMARY KEY,role TEXT NOT NULL,name TEXT NOT NULL,requested_at TEXT NOT NULL,updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS memos(
 id TEXT PRIMARY KEY,title TEXT NOT NULL,input TEXT NOT NULL,answer TEXT NOT NULL DEFAULT '',
 kind TEXT NOT NULL,tags TEXT NOT NULL DEFAULT '[]',provider TEXT NOT NULL,model TEXT NOT NULL,
 status TEXT NOT NULL,created_at TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,image BLOB,error TEXT
);
CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY AUTOINCREMENT,memo_id TEXT NOT NULL REFERENCES memos(id) ON DELETE CASCADE,kind TEXT NOT NULL,text TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS events_memo ON events(memo_id,id);
CREATE TABLE IF NOT EXISTS embeddings(memo_id TEXT PRIMARY KEY REFERENCES memos(id) ON DELETE CASCADE,model TEXT NOT NULL,vector BLOB NOT NULL);
-- A text file's content is `text` (searchable, readable in any SQLite tool);
-- a binary file's bytes are on disk at `path`. `bytes` is a legacy column.
CREATE TABLE IF NOT EXISTS files(id TEXT PRIMARY KEY,name TEXT NOT NULL,mime TEXT NOT NULL,bytes BLOB NOT NULL,created_at TEXT NOT NULL,text TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS compilations(id INTEGER PRIMARY KEY AUTOINCREMENT,file_id TEXT NOT NULL REFERENCES files(id),result TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS compilations_file ON compilations(file_id,id);
CREATE VIRTUAL TABLE IF NOT EXISTS memo_fts USING fts5(title,input,answer,tags,content='memos',content_rowid='rowid',tokenize='unicode61');
CREATE TRIGGER IF NOT EXISTS memos_ai AFTER INSERT ON memos BEGIN
 INSERT INTO memo_fts(rowid,title,input,answer,tags) VALUES(new.rowid,new.title,new.input,new.answer,new.tags);
END;
CREATE TRIGGER IF NOT EXISTS memos_ad AFTER DELETE ON memos BEGIN
 INSERT INTO memo_fts(memo_fts,rowid,title,input,answer,tags) VALUES('delete',old.rowid,old.title,old.input,old.answer,old.tags);
END;
CREATE TRIGGER IF NOT EXISTS memos_au AFTER UPDATE OF title,input,answer,tags ON memos BEGIN
 INSERT INTO memo_fts(memo_fts,rowid,title,input,answer,tags) VALUES('delete',old.rowid,old.title,old.input,old.answer,old.tags);
 INSERT INTO memo_fts(rowid,title,input,answer,tags) VALUES(new.rowid,new.title,new.input,new.answer,new.tags);
 DELETE FROM embeddings WHERE memo_id=new.id;
END;

CREATE TABLE IF NOT EXISTS capture_clients(id TEXT PRIMARY KEY,name TEXT NOT NULL,platform TEXT NOT NULL,last_seen INTEGER NOT NULL,device_key_hash TEXT);
CREATE TABLE IF NOT EXISTS capture_requests(id TEXT PRIMARY KEY,client_id TEXT NOT NULL REFERENCES capture_clients(id),memo_id TEXT NOT NULL REFERENCES memos(id) ON DELETE CASCADE,status TEXT NOT NULL DEFAULT 'queued',mode TEXT NOT NULL,display INTEGER NOT NULL,error TEXT,created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS capture_queue ON capture_requests(client_id,status,created_at);
CREATE TABLE IF NOT EXISTS web_clients(id TEXT PRIMARY KEY,name TEXT NOT NULL,platform TEXT NOT NULL,connection_id TEXT NOT NULL,connected INTEGER NOT NULL DEFAULT 0,last_seen INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS rooms(roomid TEXT PRIMARY KEY,title TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS chat_messages(messageid TEXT PRIMARY KEY,roomid TEXT NOT NULL REFERENCES rooms(roomid),memo_id TEXT NOT NULL,role TEXT NOT NULL,document TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS chat_messages_room ON chat_messages(roomid);
CREATE INDEX IF NOT EXISTS chat_messages_memo ON chat_messages(memo_id);
CREATE TABLE IF NOT EXISTS message_changes(messagesyncid INTEGER PRIMARY KEY AUTOINCREMENT,roomid TEXT NOT NULL,messageid TEXT NOT NULL,document TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS message_changes_room ON message_changes(roomid,messagesyncid);

CREATE TABLE IF NOT EXISTS chat_submissions(messageid TEXT PRIMARY KEY,request TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS file_processing(id INTEGER PRIMARY KEY AUTOINCREMENT,file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,document TEXT NOT NULL,created_at TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS sync_clock(id INTEGER PRIMARY KEY CHECK(id=1),value INTEGER NOT NULL);
INSERT OR IGNORE INTO sync_clock(id,value) VALUES(1,0);
CREATE TABLE IF NOT EXISTS notifications(id INTEGER PRIMARY KEY AUTOINCREMENT,owner_wallet TEXT,client_id TEXT,roomid TEXT,kind TEXT NOT NULL,data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS notifications_wallet ON notifications(owner_wallet,id);
CREATE INDEX IF NOT EXISTS notifications_client ON notifications(client_id,id);

-- Durable marker: interrupted user erasure resumes before accepting requests.
CREATE TABLE IF NOT EXISTS user_purges(owner_wallet TEXT PRIMARY KEY);
