use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use pdfium_render::prelude::*;

use crate::document::{Document, DocumentMetadata, RenderedPage};

/// Global Pdfium instance. PDFium is a C library that should only be loaded once.
/// With the `thread_safe` feature enabled, Pdfium is Send + Sync.
static PDFIUM: OnceLock<Pdfium> = OnceLock::new();

/// Get or initialize the global Pdfium instance.
fn pdfium() -> Result<&'static Pdfium> {
    if let Some(p) = PDFIUM.get() {
        return Ok(p);
    }

    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
        .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./lib")))
        .or_else(|_| Pdfium::bind_to_system_library())
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to load PDFium library: {e}. \
                 Run `./scripts/download-pdfium.sh` to download it, \
                 or see https://github.com/bblanchon/pdfium-binaries"
            )
        })?;

    let pdfium = Pdfium::new(bindings);
    // Ignore the error if another thread initialized it first
    let _ = PDFIUM.set(pdfium);
    Ok(PDFIUM.get().unwrap())
}

/// A PDF document backed by pdfium-render.
#[derive(Debug)]
pub struct PdfDoc {
    page_count: usize,
    page_sizes: Vec<(f32, f32)>,
    metadata: DocumentMetadata,
    /// Raw PDF bytes, kept for re-opening during render calls.
    data: Vec<u8>,
}

impl PdfDoc {
    /// Open a PDF file from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_bytes(data)
    }

    /// Open a PDF from raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let pdfium = pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(&data, None)
            .map_err(|e| anyhow::anyhow!("failed to load PDF: {e}"))?;

        let page_count = document.pages().len() as usize;

        let mut page_sizes = Vec::with_capacity(page_count);
        for i in 0..page_count {
            let page = document
                .pages()
                .get(i as u16)
                .map_err(|e| anyhow::anyhow!("failed to get page {i}: {e}"))?;
            let w = page.width().value;
            let h = page.height().value;
            page_sizes.push((w, h));
        }

        let meta = document.metadata();
        let metadata = DocumentMetadata {
            title: meta
                .get(PdfDocumentMetadataTagType::Title)
                .map(|t| t.value().to_string()),
            author: meta
                .get(PdfDocumentMetadataTagType::Author)
                .map(|t| t.value().to_string()),
            subject: meta
                .get(PdfDocumentMetadataTagType::Subject)
                .map(|t| t.value().to_string()),
            creator: meta
                .get(PdfDocumentMetadataTagType::Creator)
                .map(|t| t.value().to_string()),
        };

        // Drop the document before moving `data` into the struct.
        drop(document);

        Ok(Self {
            page_count,
            page_sizes,
            metadata,
            data,
        })
    }
}

impl Document for PdfDoc {
    fn page_count(&self) -> usize {
        self.page_count
    }

    fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        self.page_sizes
            .get(index)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("page index {index} out of range"))
    }

    fn render_page(&self, index: usize, scale: f32) -> Result<RenderedPage> {
        if index >= self.page_count {
            anyhow::bail!(
                "page index {index} out of range (total: {})",
                self.page_count
            );
        }

        let pdfium = pdfium()?;

        // Re-open the document from stored bytes for rendering.
        // This is necessary because PdfDocument borrows from Pdfium and cannot
        // be stored alongside it due to lifetime constraints.
        let document = pdfium
            .load_pdf_from_byte_slice(&self.data, None)
            .map_err(|e| anyhow::anyhow!("failed to load PDF for rendering: {e}"))?;

        let page = document
            .pages()
            .get(index as u16)
            .map_err(|e| anyhow::anyhow!("failed to get page {index}: {e}"))?;

        let (pt_w, pt_h) = self.page_sizes[index];
        let pixel_w = (pt_w * scale) as i32;
        let pixel_h = (pt_h * scale) as i32;

        let config = PdfRenderConfig::new()
            .set_target_width(pixel_w)
            .set_maximum_height(pixel_h);

        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| anyhow::anyhow!("failed to render page {index}: {e}"))?;

        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;
        let pixels = bytes::Bytes::from(bitmap.as_rgba_bytes());

        Ok(RenderedPage {
            width,
            height,
            pixels,
        })
    }

    fn metadata(&self) -> DocumentMetadata {
        self.metadata.clone()
    }
}
