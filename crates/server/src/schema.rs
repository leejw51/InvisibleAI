//! Automatic, forward-only schema migration. Every table is declared once as a
//! `CREATE TABLE IF NOT EXISTS` statement; on startup the statement creates a
//! missing table, and any column the statement names that an existing table
//! lacks is added with `ALTER TABLE … ADD COLUMN`. Adding a column to a table
//! definition anywhere in the code therefore migrates existing databases without
//! a hand-written step. Changes that are not additive (renames, type changes,
//! data rewrites) go into `VERSIONED`, which runs once per `PRAGMA user_version`.
use anyhow::{Result, ensure};
use rusqlite::Connection;

/// Create the tables in `sql` and add any columns they declare that are missing.
pub fn ensure(conn: &Connection, sql: &str) -> Result<()> {
    conn.execute_batch(sql)?;
    for (table, columns) in declared_tables(sql) {
        let existing: Vec<String> = conn
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if existing.is_empty() {
            // A virtual table or a view: nothing to reconcile.
            continue;
        }
        for (name, definition) in columns {
            if existing.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let definition = addable(&definition);
            tracing::info!(%table, %name, "Adding column to an existing table");
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {name} {definition}"
            ))?;
        }
    }
    Ok(())
}

/// `(table, [(column, definition)])` for every CREATE TABLE statement in `sql`.
/// Table constraints (PRIMARY KEY(...), UNIQUE(...), FOREIGN KEY, CHECK) are skipped.
pub fn declared_tables(sql: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut tables = vec![];
    let upper = sql.to_ascii_uppercase();
    let mut from = 0;
    while let Some(at) = upper[from..].find("CREATE TABLE") {
        let start = from + at;
        let Some(open) = sql[start..].find('(') else {
            break;
        };
        let head = &sql[start..start + open];
        let name = head
            .split_whitespace()
            .last()
            .unwrap_or("")
            .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
            .to_string();
        let body_start = start + open + 1;
        let Some(close) = matching_paren(&sql[body_start..]) else {
            break;
        };
        let body = &sql[body_start..body_start + close];
        let mut columns = vec![];
        for part in split_top_level(body) {
            let part = part.trim();
            let first = part
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            if part.is_empty()
                || matches!(
                    first.as_str(),
                    "PRIMARY" | "UNIQUE" | "FOREIGN" | "CHECK" | "CONSTRAINT"
                )
            {
                continue;
            }
            let mut words = part.splitn(2, char::is_whitespace);
            let column = words
                .next()
                .unwrap_or("")
                .trim_matches(|c| c == '"' || c == '`')
                .to_string();
            let definition = words.next().unwrap_or("").trim().to_string();
            if !column.is_empty() && !upper_contains(&head.to_ascii_uppercase(), "VIRTUAL") {
                columns.push((column, definition));
            }
        }
        if !name.is_empty() && !upper_contains(&head.to_ascii_uppercase(), "VIRTUAL") {
            tables.push((name, columns));
        }
        from = body_start + close;
    }
    tables
}

fn upper_contains(upper: &str, word: &str) -> bool {
    upper.contains(word)
}

fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = vec![];
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// SQLite cannot add a PRIMARY KEY, UNIQUE, or NOT NULL column without a default,
/// and REFERENCES on added columns is pointless for existing rows; rewrite the
/// definition so the ALTER succeeds and existing rows get a sensible value.
pub fn addable(definition: &str) -> String {
    let upper = definition.to_ascii_uppercase();
    let mut out: Vec<String> = vec![];
    let tokens: Vec<&str> = definition.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].to_ascii_uppercase();
        match t.as_str() {
            "PRIMARY" | "UNIQUE" | "AUTOINCREMENT" => {
                if t == "PRIMARY"
                    && tokens
                        .get(i + 1)
                        .is_some_and(|k| k.eq_ignore_ascii_case("KEY"))
                {
                    i += 1;
                }
            }
            "REFERENCES" => {
                // REFERENCES table(col) [ON DELETE …]: skip the whole clause.
                i += 1;
                while i < tokens.len()
                    && !matches!(
                        tokens[i].to_ascii_uppercase().as_str(),
                        "NOT" | "DEFAULT" | "CHECK" | "COLLATE"
                    )
                {
                    i += 1;
                }
                continue;
            }
            _ => out.push(tokens[i].to_string()),
        }
        i += 1;
    }
    let mut rebuilt = out.join(" ");
    if upper.contains("NOT NULL") && !upper.contains("DEFAULT") {
        let default =
            if upper.starts_with("INT") || upper.starts_with("REAL") || upper.contains("INTEGER") {
                "0"
            } else {
                "''"
            };
        rebuilt.push_str(&format!(" DEFAULT {default}"));
    }
    ensure_non_empty(rebuilt)
}

fn ensure_non_empty(definition: String) -> String {
    if definition.trim().is_empty() {
        "TEXT".into()
    } else {
        definition
    }
}

/// One-off steps for changes that are not additive, run in order once each.
/// Add a new entry at the end; never edit or reorder earlier entries.
type Step = fn(&Connection) -> Result<()>;
pub const VERSIONED: &[(&str, Step)] = &[];

pub fn run_versioned(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(current >= 0, "Negative schema version");
    for (index, (label, step)) in VERSIONED.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }
        tracing::info!(%label, version, "Applying schema migration");
        step(conn)?;
        conn.execute_batch(&format!("PRAGMA user_version = {version}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_columns_are_parsed_and_constraints_skipped() {
        let sql = "CREATE TABLE IF NOT EXISTS a(id TEXT PRIMARY KEY, n INTEGER NOT NULL DEFAULT 0, note TEXT);
            CREATE INDEX IF NOT EXISTS a_n ON a(n);
            CREATE TABLE IF NOT EXISTS b(x TEXT NOT NULL, y TEXT, PRIMARY KEY(x,y), FOREIGN KEY(x) REFERENCES a(id));
            CREATE VIRTUAL TABLE IF NOT EXISTS f USING fts5(title, content='a');";
        let tables = declared_tables(sql);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].0, "a");
        assert_eq!(
            tables[0].1,
            [
                ("id".to_string(), "TEXT PRIMARY KEY".to_string()),
                ("n".to_string(), "INTEGER NOT NULL DEFAULT 0".to_string()),
                ("note".to_string(), "TEXT".to_string())
            ]
        );
        assert_eq!(tables[1].0, "b");
        assert_eq!(tables[1].1.len(), 2);
    }

    #[test]
    fn added_columns_lose_constraints_sqlite_cannot_add() {
        assert_eq!(addable("TEXT PRIMARY KEY"), "TEXT");
        assert_eq!(addable("TEXT NOT NULL"), "TEXT NOT NULL DEFAULT ''");
        assert_eq!(addable("INTEGER NOT NULL"), "INTEGER NOT NULL DEFAULT 0");
        assert_eq!(
            addable("TEXT NOT NULL REFERENCES memos(id) ON DELETE CASCADE"),
            "TEXT NOT NULL DEFAULT ''"
        );
        assert_eq!(
            addable("INTEGER NOT NULL DEFAULT 5"),
            "INTEGER NOT NULL DEFAULT 5"
        );
        assert_eq!(addable(""), "TEXT");
    }

    #[test]
    fn missing_columns_are_added_to_existing_tables_and_versions_advance() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE things(id TEXT PRIMARY KEY, name TEXT NOT NULL); INSERT INTO things VALUES('1','old');")
            .unwrap();
        ensure(
            &conn,
            "CREATE TABLE IF NOT EXISTS things(id TEXT PRIMARY KEY, name TEXT NOT NULL, count INTEGER NOT NULL DEFAULT 0, note TEXT, owner TEXT NOT NULL REFERENCES users(id));
             CREATE TABLE IF NOT EXISTS fresh(id INTEGER PRIMARY KEY, label TEXT NOT NULL);",
        )
        .unwrap();
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(things)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(columns, ["id", "name", "count", "note", "owner"]);
        let (count, owner): (i64, String) = conn
            .query_row("SELECT count, owner FROM things WHERE id='1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((count, owner.as_str()), (0, ""));
        assert_eq!(
            conn.query_row("SELECT count(*) FROM fresh", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        // Running again is a no-op.
        ensure(&conn, "CREATE TABLE IF NOT EXISTS things(id TEXT PRIMARY KEY, name TEXT NOT NULL, count INTEGER NOT NULL DEFAULT 0, note TEXT, owner TEXT NOT NULL)").unwrap();
        run_versioned(&conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, VERSIONED.len() as i64);
    }
}
