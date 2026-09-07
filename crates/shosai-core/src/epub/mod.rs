//! EPUB format support.
//!
//! An EPUB file is a ZIP archive containing:
//! - `META-INF/container.xml` — points to the OPF (package) file
//! - `*.opf` — package document with metadata, manifest, and spine
//! - XHTML content documents (chapters)
//! - CSS stylesheets, images, fonts, and other resources

mod font;
mod math;
pub mod pagination;
mod parser;
mod presentation;
pub mod render;
mod resource;
pub mod style;
mod types;

mod computed_style;
mod limits;
mod native_text;

pub use font::{
    EpubFontAttempt, EpubFontBook, EpubFontFace, EpubFontFormat, EpubFontStyle, EpubFontWeight,
    EpubRejectedFontFace,
};
pub use limits::EpubLimits;
pub use math::{MathContent, MathDisplay, MathExpression};
pub use native_text::{
    EPUB_TEXT_MAX_ENDPOINTS, EPUB_TEXT_MAX_PIXELS, EPUB_TEXT_MAX_SCALARS, EpubTextAlign,
    EpubTextDirection, EpubTextEndpoint, EpubTextHighlight, EpubTextHit, EpubTextLayout,
    EpubTextLine, EpubTextRect, EpubTextRequest, EpubTextRun,
};
pub use parser::{EpubDoc, EpubInspection};
pub use presentation::{EpubChapterPresentation, EpubPresentation};
pub use resource::{CanonicalEpubPath, EpubReference};
pub use types::*;
