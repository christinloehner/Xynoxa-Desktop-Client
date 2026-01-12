use rusqlite::{params, Connection, Result};
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: Option<String>, // UUID from server
    pub path: String,
    pub hash: String,
    pub modified_at: i64,
    pub server_version: i64,
    pub group_folder_id: Option<String>,
    pub is_group_root: bool,
}

impl Database {
    pub fn new(db_path: &Path) -> Result<Self> {
        log::info!("Opening Database at: {:?}", db_path);
        let conn = Connection::open(db_path)?;

        // Files table with ID support
        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                id TEXT,
                hash TEXT NOT NULL,
                modified_at INTEGER NOT NULL,
                server_version INTEGER NOT NULL,
                group_folder_id TEXT,
                is_group_root INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        {
            let mut stmt = conn.prepare("PRAGMA table_info(files)")?;
            let mut rows = stmt.query([])?;
            let mut has_group_folder_id = false;
            let mut has_is_group_root = false;
            while let Some(row) = rows.next()? {
                let col_name: String = row.get(1)?;
                if col_name == "group_folder_id" {
                    has_group_folder_id = true;
                }
                if col_name == "is_group_root" {
                    has_is_group_root = true;
                }
            }
            if !has_group_folder_id {
                let _ = conn.execute("ALTER TABLE files ADD COLUMN group_folder_id TEXT", []);
            }
            if !has_is_group_root {
                let _ = conn.execute(
                    "ALTER TABLE files ADD COLUMN is_group_root INTEGER NOT NULL DEFAULT 0",
                    [],
                );
            }
        }

        // Global state (cursor)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS globals (
                key TEXT PRIMARY KEY,
                val INTEGER NOT NULL
            )",
            [],
        )?;

        // Log initial cursor state
        let instance = Self {
            conn: Mutex::new(conn),
        };

        let cursor = instance.get_cursor().unwrap_or(0);
        log::info!("Database initialized. Current Cursor: {}", cursor);

        Ok(instance)
    }

    pub fn insert_or_update(&self, record: &FileRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO files (path, id, hash, modified_at, server_version, group_folder_id, is_group_root) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.path,
                record.id,
                record.hash,
                record.modified_at,
                record.server_version,
                record.group_folder_id,
                if record.is_group_root { 1 } else { 0 }
            ],
        )?;
        Ok(())
    }

    pub fn get_file(&self, path: &str) -> Result<Option<FileRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT path, id, hash, modified_at, server_version, group_folder_id, is_group_root FROM files WHERE path = ?1",
        )?;

        let mut rows = stmt.query(params![path])?;

        if let Some(row) = rows.next()? {
            Ok(Some(FileRecord {
                path: row.get(0)?,
                id: row.get(1)?,
                hash: row.get(2)?,
                modified_at: row.get(3)?,
                server_version: row.get(4)?,
                group_folder_id: row.get(5)?,
                is_group_root: row.get::<_, i64>(6)? == 1,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_file_by_id(&self, id: &str) -> Result<Option<FileRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT path, id, hash, modified_at, server_version, group_folder_id, is_group_root FROM files WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(FileRecord {
                path: row.get(0)?,
                id: row.get(1)?,
                hash: row.get(2)?,
                modified_at: row.get(3)?,
                server_version: row.get(4)?,
                group_folder_id: row.get(5)?,
                is_group_root: row.get::<_, i64>(6)? == 1,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_file_by_hash(&self, hash: &str) -> Result<Option<FileRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT path, id, hash, modified_at, server_version, group_folder_id, is_group_root FROM files WHERE hash = ?1 LIMIT 1",
        )?;

        let mut rows = stmt.query(params![hash])?;

        if let Some(row) = rows.next()? {
            Ok(Some(FileRecord {
                path: row.get(0)?,
                id: row.get(1)?,
                hash: row.get(2)?,
                modified_at: row.get(3)?,
                server_version: row.get(4)?,
                group_folder_id: row.get(5)?,
                is_group_root: row.get::<_, i64>(6)? == 1,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_file(&self, path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn get_all_files(&self) -> Result<Vec<FileRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT path, id, hash, modified_at, server_version, group_folder_id, is_group_root FROM files")?;

        let file_iter = stmt.query_map([], |row| {
            Ok(FileRecord {
                path: row.get(0)?,
                id: row.get(1)?,
                hash: row.get(2)?,
                modified_at: row.get(3)?,
                server_version: row.get(4)?,
                group_folder_id: row.get(5)?,
                is_group_root: row.get::<_, i64>(6)? == 1,
            })
        })?;

        let mut files = Vec::new();
        for file in file_iter {
            files.push(file?);
        }
        Ok(files)
    }

    pub fn get_cursor(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT val FROM globals WHERE key = 'cursor'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(0)
        }
    }

    pub fn set_cursor(&self, cursor: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO globals (key, val) VALUES ('cursor', ?1)",
            params![cursor],
        )?;
        Ok(())
    }
}
