//! Types representing the structure of an EPUB document.

use std::collections::HashMap;

use super::style::StyleMap;

/// Metadata extracted from the OPF `<metadata>` element.
#[derive(Debug, Clone, Default)]
pub struct EpubMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    /// Manifest ID of the cover image (from `<meta name="cover" content="..."/>`).
    pub cover_image_id: Option<String>,
}

/// An entry in the OPF manifest.
#[derive(Debug, Clone)]
pub struct ManifestItem {
    /// Manifest item ID (e.g. "chapter1").
    pub id: String,
    /// Path relative to the OPF file (e.g. "Text/chapter1.xhtml").
    pub href: String,
    /// MIME type (e.g. "application/xhtml+xml").
    pub media_type: String,
}

/// A chapter (spine item) with its content loaded.
#[derive(Debug, Clone)]
pub struct Chapter {
    /// Index in the spine (reading order).
    pub index: usize,
    /// Title from the TOC, if available.
    pub title: Option<String>,
    /// Path within the EPUB archive.
    pub path: String,
    /// Raw XHTML content of this chapter.
    pub content: String,
}

/// Table of contents entry.
#[derive(Debug, Clone)]
pub struct TocEntry {
    /// Display title.
    pub title: String,
    /// Path within the EPUB archive (may include fragment #id).
    pub href: String,
    /// Nested children.
    pub children: Vec<TocEntry>,
}

/// Complete parsed EPUB structure.
#[derive(Debug, Clone)]
pub struct EpubContent {
    /// Document metadata.
    pub metadata: EpubMetadata,
    /// Chapters in reading order.
    pub chapters: Vec<Chapter>,
    /// Table of contents.
    pub toc: Vec<TocEntry>,
    /// All manifest items by ID.
    pub manifest: HashMap<String, ManifestItem>,
    /// Raw resource data by archive path (images, CSS, fonts).
    pub resources: HashMap<String, Vec<u8>>,
    /// CSS class → style map extracted from the EPUB's stylesheets.
    pub styles: StyleMap,
}
