//! Persistence for per-file reading state (last page, zoom level, etc.).
//!
//! State is stored in a SQLite database in the user's data directory:
//!   - Linux:   `~/.local/share/shosai/shosai.db`
//!   - macOS:   `~/Library/Application Support/shosai/shosai.db`
//!
//! Uses sqlx with SQLite so the same database can be extended for library
//! management in future phases.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqliteSynchronous};

const APP_DIR: &str = "shosai";
const DB_FILE: &str = "shosai.db";

/// Per-file reading state.
#[derive(Debug, Clone)]
pub struct FileReadingState {
    /// Last viewed page index (0-based).
    pub page: usize,
    /// Last zoom scale (1.0 = 100%).
    pub zoom: f32,
}

/// SQLite-backed store for reading state (and future library data).
///
/// The public API is synchronous — it bridges to async sqlx internally via
/// the tokio runtime that iced provides. Async methods are also available
/// for use in background tasks or future phases.
#[derive(Debug, Clone)]
pub struct ReadingStateStore {
    pool: SqlitePool,
}

impl ReadingStateStore {
    /// Open (or create) the store at the default platform path.
    pub fn open() -> Result<Self> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(Self::open_async())
    }

    /// Open (or create) the store at a specific database path.
    pub fn open_at(db_path: &Path) -> Result<Self> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(Self::open_at_async(db_path))
    }

    /// Async: open at default platform path.
    pub async fn open_async() -> Result<Self> {
        let path = db_file_path()?;
        Self::open_at_async(&path).await
    }

    /// Async: open at a specific database path.
    pub async fn open_at_async(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create data dir {}", parent.display()))?;
        }

        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);

        let pool = SqlitePool::connect_with(options)
            .await
            .with_context(|| format!("failed to open database at {}", db_path.display()))?;

        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Run database migrations.
    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS reading_state (
                file_path TEXT PRIMARY KEY NOT NULL,
                page      INTEGER NOT NULL DEFAULT 0,
                zoom      REAL    NOT NULL DEFAULT 1.0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await
        .context("failed to create reading_state table")?;

        Ok(())
    }

    /// Get the reading state for a file.
    pub fn get(&self, file_path: &Path) -> Option<FileReadingState> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.get_async(file_path))
    }

    /// Set the reading state for a file.
    pub fn set(&self, file_path: &Path, state: &FileReadingState) -> Result<()> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.set_async(file_path, state))
    }

    /// Async: get the reading state for a file.
    pub async fn get_async(&self, file_path: &Path) -> Option<FileReadingState> {
        let key = canonical_key(file_path);

        sqlx::query("SELECT page, zoom FROM reading_state WHERE file_path = ?")
            .bind(&key)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .map(|row| FileReadingState {
                page: row.get::<i64, _>("page") as usize,
                zoom: row.get::<f64, _>("zoom") as f32,
            })
    }

    /// Async: set the reading state for a file.
    pub async fn set_async(&self, file_path: &Path, state: &FileReadingState) -> Result<()> {
        let key = canonical_key(file_path);

        sqlx::query(
            "INSERT INTO reading_state (file_path, page, zoom, updated_at)
             VALUES (?, ?, ?, datetime('now'))
             ON CONFLICT(file_path) DO UPDATE SET
                page = excluded.page,
                zoom = excluded.zoom,
                updated_at = excluded.updated_at",
        )
        .bind(&key)
        .bind(state.page as i64)
        .bind(state.zoom as f64)
        .execute(&self.pool)
        .await
        .context("failed to save reading state")?;

        Ok(())
    }
}

/// Convert a file path to a canonical string key.
fn canonical_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Get the path to the database file.
fn db_file_path() -> Result<PathBuf> {
    let data_dir = data_dir()?;
    Ok(data_dir.join(APP_DIR).join(DB_FILE))
}

/// Get the platform-specific data directory.
fn data_dir() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg));
    }

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")?;

    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library").join("Application Support"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(home.join(".local").join("share"))
    }
}
