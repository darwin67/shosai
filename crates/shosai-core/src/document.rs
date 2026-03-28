/// Common metadata for any document format.
#[derive(Debug, Clone, Default)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
}

/// A rendered page as raw RGBA pixel data.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub pixels: bytes::Bytes,
}

/// Trait representing any openable document (PDF, EPUB, CBZ, etc.).
pub trait Document {
    /// Total number of pages in the document.
    fn page_count(&self) -> usize;

    /// Get the dimensions of a page in points (1 point = 1/72 inch).
    fn page_size(&self, index: usize) -> anyhow::Result<(f32, f32)>;

    /// Render a page at the given scale factor, returning raw RGBA pixels.
    fn render_page(&self, index: usize, scale: f32) -> anyhow::Result<RenderedPage>;

    /// Get document metadata (title, author, etc.).
    fn metadata(&self) -> DocumentMetadata;

    /// Convenience: get the document title.
    fn title(&self) -> Option<String> {
        self.metadata().title
    }
}
