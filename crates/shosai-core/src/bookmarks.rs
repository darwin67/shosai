//! Bookmark and annotation management.
//!
//! Bookmarks are per-file, per-page markers with optional notes. They are
//! stored in the same SQLite database as the reading state and library.

use std::path::Path;

use anyhow::{Context, Result};
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

/// A single bookmark entry.
#[derive(Debug, Clone)]
pub struct Bookmark {
    pub id: i64,
    pub file_path: String,
    pub page: usize,
    pub title: Option<String>,
    pub note: Option<String>,
    pub color: String,
    pub created_at: String,
}

/// Bookmark store backed by SQLite.
#[derive(Debug, Clone)]
pub struct BookmarkStore {
    pool: SqlitePool,
}

impl BookmarkStore {
    /// Create a bookmark store from an existing connection pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Async API --

    /// Add a bookmark for a page. Returns the new bookmark.
    pub async fn add_async(
        &self,
        file_path: &Path,
        page: usize,
        title: Option<&str>,
        note: Option<&str>,
        color: &str,
    ) -> Result<Bookmark> {
        let key = canonical_key(file_path);

        let id = sqlx::query(
            "INSERT INTO bookmarks (file_path, page, title, note, color)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(file_path, page, note) DO UPDATE SET
                title = excluded.title,
                color = excluded.color
             RETURNING id",
        )
        .bind(&key)
        .bind(page as i64)
        .bind(title)
        .bind(note)
        .bind(color)
        .fetch_one(&self.pool)
        .await
        .context("failed to add bookmark")?
        .get::<i64, _>("id");

        self.get_by_id_async(id)
            .await?
            .context("bookmark not found after insert")
    }

    /// Toggle a bookmark: if one exists at this page (with no note), remove it;
    /// otherwise, create one. Returns `Some(bookmark)` if created, `None` if removed.
    pub async fn toggle_async(
        &self,
        file_path: &Path,
        page: usize,
        title: Option<&str>,
    ) -> Result<Option<Bookmark>> {
        let key = canonical_key(file_path);

        // Check for an existing bookmark on this page without a note.
        let existing = sqlx::query(
            "SELECT id FROM bookmarks
             WHERE file_path = ? AND page = ? AND note IS NULL",
        )
        .bind(&key)
        .bind(page as i64)
        .fetch_optional(&self.pool)
        .await
        .context("failed to check existing bookmark")?;

        if let Some(row) = existing {
            let id: i64 = row.get("id");
            self.remove_async(id).await?;
            Ok(None)
        } else {
            let bm = self
                .add_async(file_path, page, title, None, "yellow")
                .await?;
            Ok(Some(bm))
        }
    }

    /// List all bookmarks for a file, ordered by page.
    pub async fn list_for_file_async(&self, file_path: &Path) -> Result<Vec<Bookmark>> {
        let key = canonical_key(file_path);

        let rows = sqlx::query(
            "SELECT id, file_path, page, title, note, color, created_at
             FROM bookmarks
             WHERE file_path = ?
             ORDER BY page ASC, created_at ASC",
        )
        .bind(&key)
        .fetch_all(&self.pool)
        .await
        .context("failed to list bookmarks")?;

        Ok(rows.iter().filter_map(row_to_bookmark).collect())
    }

    /// Check if a specific page is bookmarked (has a no-note bookmark).
    pub async fn is_bookmarked_async(&self, file_path: &Path, page: usize) -> bool {
        let key = canonical_key(file_path);

        sqlx::query(
            "SELECT 1 FROM bookmarks
             WHERE file_path = ? AND page = ? AND note IS NULL
             LIMIT 1",
        )
        .bind(&key)
        .bind(page as i64)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .is_some()
    }

    /// Update the note on a bookmark.
    pub async fn update_note_async(&self, bookmark_id: i64, note: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE bookmarks SET note = ? WHERE id = ?")
            .bind(note)
            .bind(bookmark_id)
            .execute(&self.pool)
            .await
            .context("failed to update bookmark note")?;
        Ok(())
    }

    /// Update the title on a bookmark.
    pub async fn update_title_async(&self, bookmark_id: i64, title: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE bookmarks SET title = ? WHERE id = ?")
            .bind(title)
            .bind(bookmark_id)
            .execute(&self.pool)
            .await
            .context("failed to update bookmark title")?;
        Ok(())
    }

    /// Remove a bookmark by ID.
    pub async fn remove_async(&self, bookmark_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM bookmarks WHERE id = ?")
            .bind(bookmark_id)
            .execute(&self.pool)
            .await
            .context("failed to remove bookmark")?;
        Ok(())
    }

    /// Remove all bookmarks for a file.
    pub async fn remove_all_for_file_async(&self, file_path: &Path) -> Result<()> {
        let key = canonical_key(file_path);
        sqlx::query("DELETE FROM bookmarks WHERE file_path = ?")
            .bind(&key)
            .execute(&self.pool)
            .await
            .context("failed to remove bookmarks")?;
        Ok(())
    }

    /// Get a single bookmark by ID.
    async fn get_by_id_async(&self, id: i64) -> Result<Option<Bookmark>> {
        let row = sqlx::query(
            "SELECT id, file_path, page, title, note, color, created_at
             FROM bookmarks WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to get bookmark")?;

        Ok(row.as_ref().and_then(row_to_bookmark))
    }

    /// Export all bookmarks for a file as Markdown text.
    pub async fn export_markdown_async(&self, file_path: &Path) -> Result<String> {
        let bookmarks = self.list_for_file_async(file_path).await?;

        let filename = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        let mut md = format!("# Bookmarks: {filename}\n\n");

        for bm in &bookmarks {
            let page_label = bm.page + 1;
            let title = bm.title.as_deref().unwrap_or("Untitled");
            md.push_str(&format!("## Page {page_label}: {title}\n\n"));

            if let Some(note) = &bm.note {
                md.push_str(note);
                md.push_str("\n\n");
            }

            md.push_str(&format!("*Added: {}*\n\n---\n\n", bm.created_at));
        }

        Ok(md)
    }

    // -- Sync wrappers --

    pub fn add(
        &self,
        file_path: &Path,
        page: usize,
        title: Option<&str>,
        note: Option<&str>,
        color: &str,
    ) -> Result<Bookmark> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.add_async(file_path, page, title, note, color))
    }

    pub fn toggle(
        &self,
        file_path: &Path,
        page: usize,
        title: Option<&str>,
    ) -> Result<Option<Bookmark>> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.toggle_async(file_path, page, title))
    }

    pub fn list_for_file(&self, file_path: &Path) -> Result<Vec<Bookmark>> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.list_for_file_async(file_path))
    }

    pub fn is_bookmarked(&self, file_path: &Path, page: usize) -> bool {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.is_bookmarked_async(file_path, page))
    }

    pub fn export_markdown(&self, file_path: &Path) -> Result<String> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.export_markdown_async(file_path))
    }
}

fn canonical_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn row_to_bookmark(row: &sqlx::sqlite::SqliteRow) -> Option<Bookmark> {
    Some(Bookmark {
        id: row.try_get("id").ok()?,
        file_path: row.try_get("file_path").ok()?,
        page: row.try_get::<i64, _>("page").ok()? as usize,
        title: row.try_get("title").ok()?,
        note: row.try_get("note").ok()?,
        color: row.try_get("color").ok()?,
        created_at: row.try_get("created_at").ok()?,
    })
}
