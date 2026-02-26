use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct LocalDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub path: String,
    pub blake3_hash: String,
    pub last_modified: i64,
    pub sync_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetryEntry {
    pub path: String,
    pub attempts: i32,
    pub last_error: String,
    pub next_retry: i64,
}

impl LocalDb {
    pub fn open() -> anyhow::Result<Self> {
        let db_path = Self::db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn db_path() -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        Ok(home.join(".local/share/entanglement/sync.db"))
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                blake3_hash TEXT NOT NULL,
                last_modified INTEGER NOT NULL,
                sync_cursor TEXT
            );

            CREATE TABLE IF NOT EXISTS failed_uploads (
                path TEXT PRIMARY KEY,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT NOT NULL DEFAULT '',
                next_retry INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS sync_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_files_hash ON files(blake3_hash);
            CREATE INDEX IF NOT EXISTS idx_retry_next ON failed_uploads(next_retry);
            "#,
        )?;
        Ok(())
    }

    pub fn get_file(&self, path: &str) -> anyhow::Result<Option<FileRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT path, blake3_hash, last_modified, sync_cursor FROM files WHERE path = ?",
        )?;
        let result = stmt.query_row([path], |row| {
            Ok(FileRecord {
                path: row.get(0)?,
                blake3_hash: row.get(1)?,
                last_modified: row.get(2)?,
                sync_cursor: row.get(3)?,
            })
        });
        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn upsert_file(&self, record: &FileRecord) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO files (path, blake3_hash, last_modified, sync_cursor)
             VALUES (?, ?, ?, ?)",
            (
                &record.path,
                &record.blake3_hash,
                &record.last_modified,
                &record.sync_cursor,
            ),
        )?;
        Ok(())
    }

    pub fn remove_file(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        conn.execute("DELETE FROM files WHERE path = ?", [path])?;
        Ok(())
    }

    pub fn list_files(&self) -> anyhow::Result<Vec<FileRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT path, blake3_hash, last_modified, sync_cursor FROM files",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileRecord {
                path: row.get(0)?,
                blake3_hash: row.get(1)?,
                last_modified: row.get(2)?,
                sync_cursor: row.get(3)?,
            })
        })?;
        let mut files = Vec::new();
        for row in rows {
            files.push(row?);
        }
        Ok(files)
    }

    pub fn add_retry(&self, path: &str, error: &str) -> anyhow::Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        conn.execute(
            "INSERT INTO failed_uploads (path, attempts, last_error, next_retry)
             VALUES (?, 1, ?, ? + 60)
             ON CONFLICT(path) DO UPDATE SET
               attempts = attempts + 1,
               last_error = excluded.last_error,
               next_retry = excluded.next_retry + (attempts * 60)",
            (path, error, now),
        )?;
        Ok(())
    }

    pub fn get_pending_retries(&self) -> anyhow::Result<Vec<RetryEntry>> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT path, attempts, last_error, next_retry FROM failed_uploads
             WHERE next_retry <= ? AND attempts < 5
             ORDER BY next_retry",
        )?;
        let rows = stmt.query_map([now], |row| {
            Ok(RetryEntry {
                path: row.get(0)?,
                attempts: row.get(1)?,
                last_error: row.get(2)?,
                next_retry: row.get(3)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    pub fn clear_retry(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        conn.execute("DELETE FROM failed_uploads WHERE path = ?", [path])?;
        Ok(())
    }

    pub fn get_last_sync_time(&self) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT value FROM sync_state WHERE key = 'last_sync_time'")?;
        let result = stmt.query_row([], |row| row.get(0));
        match result {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_last_sync_time(&self, time: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("db lock: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('last_sync_time', ?)",
            [time],
        )?;
        Ok(())
    }

    /// Open an in-memory database (for testing).
    #[cfg(test)]
    pub fn open_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_init() {
        let db = LocalDb::open_memory().expect("open_memory should succeed");
        // Verify tables exist by querying sqlite_master
        let conn = db.conn.lock().unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(tables.contains(&"files".to_string()), "files table missing");
        assert!(tables.contains(&"failed_uploads".to_string()), "failed_uploads table missing");
        assert!(tables.contains(&"sync_state".to_string()), "sync_state table missing");
    }

    #[test]
    fn test_upsert_file() {
        let db = LocalDb::open_memory().unwrap();
        let record = FileRecord {
            path: "docs/readme.md".to_string(),
            blake3_hash: "abc123".to_string(),
            last_modified: 1700000000,
            sync_cursor: Some("cursor_1".to_string()),
        };
        db.upsert_file(&record).unwrap();

        let fetched = db.get_file("docs/readme.md").unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.path, "docs/readme.md");
        assert_eq!(fetched.blake3_hash, "abc123");
        assert_eq!(fetched.last_modified, 1700000000);
        assert_eq!(fetched.sync_cursor, Some("cursor_1".to_string()));
    }

    #[test]
    fn test_update_file_hash() {
        let db = LocalDb::open_memory().unwrap();
        let record = FileRecord {
            path: "src/main.rs".to_string(),
            blake3_hash: "hash_v1".to_string(),
            last_modified: 1700000000,
            sync_cursor: None,
        };
        db.upsert_file(&record).unwrap();

        // Update with new hash
        let updated = FileRecord {
            blake3_hash: "hash_v2".to_string(),
            last_modified: 1700001000,
            ..record
        };
        db.upsert_file(&updated).unwrap();

        let fetched = db.get_file("src/main.rs").unwrap().unwrap();
        assert_eq!(fetched.blake3_hash, "hash_v2");
        assert_eq!(fetched.last_modified, 1700001000);
    }

    #[test]
    fn test_retry_queue() {
        let db = LocalDb::open_memory().unwrap();
        db.add_retry("failed/file.txt", "connection timeout").unwrap();

        // Query all retries (use a far-future timestamp to capture it)
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path, attempts, last_error FROM failed_uploads")
            .unwrap();
        let entries: Vec<(String, i32, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "failed/file.txt");
        assert_eq!(entries[0].1, 1);
        assert_eq!(entries[0].2, "connection timeout");
    }

    #[test]
    fn test_retry_clear() {
        let db = LocalDb::open_memory().unwrap();
        db.add_retry("failed/file.txt", "timeout").unwrap();
        db.clear_retry("failed/file.txt").unwrap();

        // Verify it's gone
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM failed_uploads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
