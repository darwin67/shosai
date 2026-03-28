//! EPUB format support.
//!
//! An EPUB file is a ZIP archive containing:
//! - `META-INF/container.xml` — points to the OPF (package) file
//! - `*.opf` — package document with metadata, manifest, and spine
//! - XHTML content documents (chapters)
//! - CSS stylesheets, images, fonts, and other resources

mod parser;
mod types;

pub use parser::EpubDoc;
pub use types::*;
