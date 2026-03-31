//! Library management: import, browse, and manage a collection of books.
//!
//! Uses the same SQLite database as the reading state store.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::cbz::CbzDoc;
use crate::document::Document;
use crate::epub::EpubDoc;
use crate::pdf::PdfDoc;

/// Supported book format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookFormat {
    Pdf,
    Epub,
    Cbz,
}

impl BookFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "epub" => Some(Self::Epub),
            "cbz" => Some(Self::Cbz),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Epub => "epub",
            Self::Cbz => "cbz",
        }
    }

    fn from_db(s: &str) -> Option<Self> {
        match s {
            "pdf" => Some(Self::Pdf),
            "epub" => Some(Self::Epub),
            "cbz" => Some(Self::Cbz),
            _ => None,
        }
    }
}

/// A book entry in the library.
#[derive(Debug, Clone)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub format: BookFormat,
    pub file_path: String,
    pub cover: Option<Vec<u8>>,
    pub progress: f64,
    pub date_added: String,
    pub last_read: Option<String>,
}

/// Library backed by SQLite.
#[derive(Debug, Clone)]
pub struct Library {
    pool: SqlitePool,
}

impl Library {
    /// Create a library handle from an existing connection pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Import a single book file into the library.
    ///
    /// Extracts metadata and cover image from the file. If the file
    /// already exists in the library, returns its existing book entry.
    pub async fn import_file(&self, path: &Path) -> Result<Book> {
        // Normalize paths so lookups and progress updates stay consistent.
        let path = canonical_path(path);
        let path_str = path.to_string_lossy().to_string();

        // Check if already imported.
        if let Some(book) = self.get_by_path(&path_str).await? {
            return Ok(book);
        }

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let format = BookFormat::from_extension(&ext)
            .with_context(|| format!("unsupported format: .{ext}"))?;

        let (title, author, cover) = extract_metadata_and_cover(&path, format)?;

        sqlx::query(
            "INSERT INTO books (title, author, format, file_path, cover_blob)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&title)
        .bind(&author)
        .bind(format.as_str())
        .bind(&path_str)
        .bind(&cover)
        .execute(&self.pool)
        .await
        .context("failed to insert book")?;

        self.get_by_path(&path_str)
            .await?
            .context("book not found after insert")
    }

    /// Import all supported files from a directory (recursively).
    pub async fn import_directory(&self, dir: &Path) -> Result<Vec<Book>> {
        let mut books = Vec::new();
        let mut dirs = vec![dir.to_path_buf()];

        while let Some(current) = dirs.pop() {
            let entries = std::fs::read_dir(&current)
                .with_context(|| format!("failed to read directory {}", current.display()))?;

            for entry in entries {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }

                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();

                if BookFormat::from_extension(&ext).is_some() {
                    match self.import_file(&path).await {
                        Ok(book) => books.push(book),
                        Err(e) => {
                            eprintln!("warning: failed to import {}: {e}", path.display());
                        }
                    }
                }
            }
        }

        Ok(books)
    }

    /// List all books, ordered by most recently read first, then by date added.
    pub async fn list_all(&self) -> Result<Vec<Book>> {
        let rows = sqlx::query(
            "SELECT id, title, author, format, file_path, cover_blob, progress,
                    date_added, last_read
             FROM books
             ORDER BY last_read DESC NULLS LAST, date_added DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to list books")?;

        Ok(rows.iter().filter_map(row_to_book).collect())
    }

    /// Search books by title or author.
    pub async fn search(&self, query: &str) -> Result<Vec<Book>> {
        let pattern = format!("%{query}%");
        let rows = sqlx::query(
            "SELECT id, title, author, format, file_path, cover_blob, progress,
                    date_added, last_read
             FROM books
             WHERE title LIKE ? OR author LIKE ?
             ORDER BY last_read DESC NULLS LAST, date_added DESC",
        )
        .bind(&pattern)
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await
        .context("failed to search books")?;

        Ok(rows.iter().filter_map(row_to_book).collect())
    }

    /// Filter books by format.
    pub async fn filter_by_format(&self, format: BookFormat) -> Result<Vec<Book>> {
        let rows = sqlx::query(
            "SELECT id, title, author, format, file_path, cover_blob, progress,
                    date_added, last_read
             FROM books
             WHERE format = ?
             ORDER BY last_read DESC NULLS LAST, date_added DESC",
        )
        .bind(format.as_str())
        .fetch_all(&self.pool)
        .await
        .context("failed to filter books")?;

        Ok(rows.iter().filter_map(row_to_book).collect())
    }

    /// Update reading progress (0.0 to 1.0) and last_read timestamp.
    pub async fn update_progress(&self, book_id: i64, progress: f64) -> Result<()> {
        let progress = progress.clamp(0.0, 1.0);
        sqlx::query("UPDATE books SET progress = ?, last_read = datetime('now') WHERE id = ?")
            .bind(progress)
            .bind(book_id)
            .execute(&self.pool)
            .await
            .context("failed to update progress")?;
        Ok(())
    }

    /// Update reading progress using a file path (0.0 to 1.0).
    pub async fn update_progress_by_path(&self, path: &Path, progress: f64) -> Result<()> {
        // Use canonical paths so the reader and library always converge on one row.
        let progress = progress.clamp(0.0, 1.0);
        let key = canonical_path(path).to_string_lossy().to_string();

        sqlx::query(
            "UPDATE books SET progress = ?, last_read = datetime('now') WHERE file_path = ?",
        )
        .bind(progress)
        .bind(&key)
        .execute(&self.pool)
        .await
        .context("failed to update progress by path")?;
        Ok(())
    }

    /// Remove a book from the library (does not delete the file).
    pub async fn remove(&self, book_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(book_id)
            .execute(&self.pool)
            .await
            .context("failed to remove book")?;
        Ok(())
    }

    /// Get a book by file path.
    async fn get_by_path(&self, path: &str) -> Result<Option<Book>> {
        let row = sqlx::query(
            "SELECT id, title, author, format, file_path, cover_blob, progress,
                    date_added, last_read
             FROM books WHERE file_path = ?",
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await
        .context("failed to query book by path")?;

        Ok(row.as_ref().and_then(row_to_book))
    }
}

fn row_to_book(row: &sqlx::sqlite::SqliteRow) -> Option<Book> {
    let format_str: String = row.try_get("format").ok()?;
    let format = BookFormat::from_db(&format_str)?;

    Some(Book {
        id: row.try_get("id").ok()?,
        title: row.try_get("title").ok()?,
        author: row.try_get("author").ok()?,
        format,
        file_path: row.try_get("file_path").ok()?,
        cover: row.try_get("cover_blob").ok()?,
        progress: row.try_get("progress").ok()?,
        date_added: row.try_get("date_added").ok()?,
        last_read: row.try_get("last_read").ok()?,
    })
}

/// Best-effort canonicalization used for stable database keys.
fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Cover & metadata extraction
// ---------------------------------------------------------------------------

const COVER_MAX_WIDTH: u32 = 300;
const COVER_MAX_HEIGHT: u32 = 400;

fn extract_metadata_and_cover(
    path: &Path,
    format: BookFormat,
) -> Result<(String, Option<String>, Option<Vec<u8>>)> {
    match format {
        BookFormat::Pdf => extract_pdf_metadata(path),
        BookFormat::Epub => extract_epub_metadata(path),
        BookFormat::Cbz => extract_cbz_metadata(path),
    }
}

fn extract_pdf_metadata(path: &Path) -> Result<(String, Option<String>, Option<Vec<u8>>)> {
    let doc = PdfDoc::open(path)?;
    let meta = doc.metadata();
    let title = meta.title.unwrap_or_else(|| filename_title(path));
    let author = meta.author;

    // Render first page as cover thumbnail.
    let cover = doc
        .render_page(0, 0.5) // half-scale for thumbnail
        .ok()
        .and_then(|page| encode_cover_png(page.width, page.height, &page.pixels));

    Ok((title, author, cover))
}

fn extract_epub_metadata(path: &Path) -> Result<(String, Option<String>, Option<Vec<u8>>)> {
    let doc = EpubDoc::open(path)?;
    let meta = &doc.content.metadata;
    let title = meta.title.clone().unwrap_or_else(|| filename_title(path));
    let author = meta.author.clone();

    // Extract cover image from manifest.
    let cover = meta
        .cover_image_id
        .as_ref()
        .and_then(|id| doc.content.manifest.get(id))
        .and_then(|item| doc.content.resources.get(&item.href))
        .and_then(|data| resize_cover_image(data));

    Ok((title, author, cover))
}

fn extract_cbz_metadata(path: &Path) -> Result<(String, Option<String>, Option<Vec<u8>>)> {
    let doc = CbzDoc::open(path)?;
    let title = doc.metadata().title.unwrap_or_else(|| filename_title(path));

    // Use first page as cover.
    let cover = doc
        .page_image_bytes(0)
        .ok()
        .and_then(|data| resize_cover_image(&data));

    Ok((title, None, cover))
}

fn filename_title(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Resize an image to fit within cover thumbnail bounds and encode as PNG.
fn resize_cover_image(data: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let thumb = img.resize(
        COVER_MAX_WIDTH,
        COVER_MAX_HEIGHT,
        image::imageops::FilterType::Triangle,
    );
    let rgba = thumb.to_rgba8();
    encode_cover_png(rgba.width(), rgba.height(), rgba.as_raw())
}

/// Encode RGBA pixels as PNG bytes.
fn encode_cover_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    image::ImageEncoder::write_image(
        encoder,
        rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(buf)
}
