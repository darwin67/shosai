//! CBZ (Comic Book Zip) format support.
//!
//! A CBZ file is a ZIP archive containing image files (JPEG, PNG, GIF, WebP).
//! Pages are determined by sorting image entries in natural filename order.

use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::{Context, Result};
use zip::ZipArchive;

use crate::document::{DocumentMetadata, RenderedPage};

/// Image extensions we recognize as comic pages.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

/// A parsed CBZ document.
#[derive(Debug)]
pub struct CbzDoc {
    /// Sorted list of image entry paths within the archive.
    page_paths: Vec<String>,
    /// Raw ZIP data, kept for rendering pages on demand.
    data: Vec<u8>,
    /// Title derived from the filename.
    title: Option<String>,
}

impl CbzDoc {
    /// Open a CBZ file from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;

        let title = path.file_stem().map(|s| s.to_string_lossy().to_string());

        Self::from_bytes_with_title(data, title)
    }

    /// Open a CBZ from raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        Self::from_bytes_with_title(data, None)
    }

    fn from_bytes_with_title(data: Vec<u8>, title: Option<String>) -> Result<Self> {
        let cursor = Cursor::new(&data);
        let mut archive = ZipArchive::new(cursor).context("failed to open CBZ as ZIP archive")?;

        let mut page_paths: Vec<String> = (0..archive.len())
            .filter_map(|i| {
                let file = archive.by_index(i).ok()?;
                let name = file.name().to_string();

                // Skip directories and hidden files.
                if name.ends_with('/') || name.contains("/__MACOSX") || name.contains("/.") {
                    return None;
                }

                // Check extension.
                let ext = name.rsplit('.').next()?.to_lowercase();
                if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        // Natural sort order for correct page numbering (page2 before page10).
        page_paths.sort_by(|a, b| natord::compare(a, b));

        if page_paths.is_empty() {
            anyhow::bail!("CBZ archive contains no image files");
        }

        Ok(Self {
            page_paths,
            data,
            title,
        })
    }

    /// Number of pages.
    pub fn page_count(&self) -> usize {
        self.page_paths.len()
    }

    /// Render a page by index, decoding the image to RGBA.
    ///
    /// `scale` multiplies the native image dimensions.
    pub fn render_page(&self, index: usize, scale: f32) -> Result<RenderedPage> {
        if index >= self.page_paths.len() {
            anyhow::bail!(
                "page index {index} out of range (total: {})",
                self.page_paths.len()
            );
        }

        let path = &self.page_paths[index];
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor).context("failed to reopen CBZ archive")?;

        let mut file = archive
            .by_name(path)
            .with_context(|| format!("image not found in archive: {path}"))?;

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .with_context(|| format!("failed to read image: {path}"))?;

        let img = image::load_from_memory(&buf)
            .with_context(|| format!("failed to decode image: {path}"))?;

        let img = if (scale - 1.0).abs() > f32::EPSILON {
            let new_w = (img.width() as f32 * scale) as u32;
            let new_h = (img.height() as f32 * scale) as u32;
            img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };

        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        Ok(RenderedPage {
            width,
            height,
            pixels: bytes::Bytes::from(rgba.into_raw()),
        })
    }

    /// Get the dimensions of a page without full rendering.
    pub fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        if index >= self.page_paths.len() {
            anyhow::bail!(
                "page index {index} out of range (total: {})",
                self.page_paths.len()
            );
        }

        let path = &self.page_paths[index];
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)?;
        let mut file = archive.by_name(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        let img = image::load_from_memory(&buf)
            .with_context(|| format!("failed to decode image: {path}"))?;

        Ok((img.width() as f32, img.height() as f32))
    }

    /// Get document metadata.
    pub fn metadata(&self) -> DocumentMetadata {
        DocumentMetadata {
            title: self.title.clone(),
            author: None,
            subject: None,
            creator: None,
        }
    }

    /// Get the raw image bytes for a page (for cover extraction).
    pub fn page_image_bytes(&self, index: usize) -> Result<Vec<u8>> {
        if index >= self.page_paths.len() {
            anyhow::bail!("page index {index} out of range");
        }

        let path = &self.page_paths[index];
        let cursor = Cursor::new(&self.data);
        let mut archive = ZipArchive::new(cursor)?;
        let mut file = archive.by_name(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests use a sample CBZ fixture that must be created by the test harness.
    // See tests/cbz_tests.rs for integration tests.

    #[test]
    fn test_image_extensions() {
        assert!(IMAGE_EXTENSIONS.contains(&"jpg"));
        assert!(IMAGE_EXTENSIONS.contains(&"png"));
        assert!(!IMAGE_EXTENSIONS.contains(&"txt"));
    }
}
