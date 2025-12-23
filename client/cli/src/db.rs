use rusqlite::Connection;
use std::path::Path;

/// Initialize local SQLite database for tracking sync state
pub fn init_local_db(path: &Path) -> anyhow::Result<()> {
    let conn = Connection::open(path)?;
    
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS files (
            path TEXT PRIMARY KEY,
            local_hash TEXT,
            remote_hash TEXT,
            remote_version_id TEXT,
            local_mtime INTEGER,
            synced_at INTEGER
        );
        
        CREATE TABLE IF NOT EXISTS sync_state (
            key TEXT PRIMARY KEY,
            value TEXT
        );
        
        CREATE INDEX IF NOT EXISTS idx_files_local_hash ON files(local_hash);
        "#,
    )?;
    
    Ok(())
}

#[allow(dead_code)]
pub struct LocalDb {
    conn: Connection,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileState {
    pub path: String,
    pub local_hash: Option<String>,
    pub remote_hash: Option<String>,
    pub remote_version_id: Option<String>,
    pub local_mtime: Option<i64>,
    pub synced_at: Option<i64>,
}

#[allow(dead_code)]
impl LocalDb {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    pub fn get_file_state(&self, path: &str) -> anyhow::Result<Option<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, local_hash, remote_hash, remote_version_id, local_mtime, synced_at 
             FROM files WHERE path = ?"
        )?;
        
        let result = stmt.query_row([path], |row| {
            Ok(FileState {
                path: row.get(0)?,
                local_hash: row.get(1)?,
                remote_hash: row.get(2)?,
                remote_version_id: row.get(3)?,
                local_mtime: row.get(4)?,
                synced_at: row.get(5)?,
            })
        });

        match result {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_file_state(&self, state: &FileState) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, local_hash, remote_hash, remote_version_id, local_mtime, synced_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            (
                &state.path,
                &state.local_hash,
                &state.remote_hash,
                &state.remote_version_id,
                &state.local_mtime,
                &state.synced_at,
            ),
        )?;
        Ok(())
    }

    pub fn get_cursor(&self) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM sync_state WHERE key = 'cursor'")?;
        let result = stmt.query_row([], |row| row.get(0));
        
        match result {
            Ok(cursor) => Ok(Some(cursor)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_cursor(&self, cursor: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('cursor', ?)",
            [cursor],
        )?;
        Ok(())
    }

    pub fn list_all_files(&self) -> anyhow::Result<Vec<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, local_hash, remote_hash, remote_version_id, local_mtime, synced_at FROM files"
        )?;
        
        let files = stmt.query_map([], |row| {
            Ok(FileState {
                path: row.get(0)?,
                local_hash: row.get(1)?,
                remote_hash: row.get(2)?,
                remote_version_id: row.get(3)?,
                local_mtime: row.get(4)?,
                synced_at: row.get(5)?,
            })
        })?;

        let mut result = Vec::new();
        for file in files {
            result.push(file?);
        }
        Ok(result)
    }
}














