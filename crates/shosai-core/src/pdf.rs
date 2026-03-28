use std::path::Path;

use anyhow::{Context, Result};
use pdfium_render::prelude::*;

use crate::document::{Document, DocumentMetadata, RenderedPage};

/// Create a short-lived Pdfium instance.
///
/// `pdfium-render`'s `thread_safe` feature serializes all PDFium access behind a
/// global mutex. The lock is acquired on `FPDF_InitLibrary` (when a `Pdfium` is
/// created) and released on `FPDF_DestroyLibrary` (when it is dropped). Creating
/// a `Pdfium`, doing work, and dropping it promptly is the intended usage pattern
/// — it keeps the lock held only as long as needed and allows other threads to
/// proceed in between.
fn create_pdfium() -> Result<Pdfium> {
    let bindings = Pdfium::bind_to_system_library().map_err(|e| {
        anyhow::anyhow!(
            "failed to load PDFium library: {e}. \
             Ensure pdfium-binaries is available via LD_LIBRARY_PATH \
             (enter the Nix dev shell with `nix develop`)"
        )
    })?;

    Ok(Pdfium::new(bindings))
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
        let pdfium = create_pdfium()?;
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

        // Explicitly drop document and pdfium before moving `data` into the struct.
        // This releases the borrow on `data` and the global PDFium mutex lock.
        drop(document);
        drop(pdfium);

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

        let pdfium = create_pdfium()?;

        // Re-open the document from stored bytes for rendering.
        // PdfDocument borrows from Pdfium, so both must live together
        // and are dropped at the end of this call, releasing the lock.
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
