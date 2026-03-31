//! In-document search for PDF and EPUB formats.
//!
//! Provides case-insensitive text search across document pages or chapters,
//! returning a list of [`SearchMatch`] values that the GUI can use to navigate
//! results and highlight matches.

use crate::document::Document;
use crate::epub::EpubDoc;
use crate::epub::render::{ContentNode, TextSpan};
use crate::pdf::PdfDoc;

/// A single search match within a document.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    /// Page index (PDF/CBZ) or chapter index (EPUB).
    pub page: usize,
    /// Character offset within the page/chapter text where the match starts.
    pub offset: usize,
    /// Length of the match in characters.
    pub length: usize,
    /// A short snippet of surrounding context text.
    pub context: String,
}

/// Search all pages of a PDF document for the given query (case-insensitive).
///
/// Returns matches across all pages. For large documents this may be slow;
/// callers should consider running this on a background thread.
pub fn search_pdf(doc: &PdfDoc, query: &str) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for page_idx in 0..doc.page_count() {
        let page_text = match doc.page_text(page_idx) {
            Ok(t) => t,
            Err(_) => continue,
        };

        find_matches_in_text(&page_text, &query_lower, page_idx, &mut results);
    }

    results
}

/// Search all chapters of an EPUB document for the given query (case-insensitive).
///
/// Extracts plain text from the chapter content nodes and searches within them.
pub fn search_epub(doc: &EpubDoc, query: &str) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for (chapter_idx, chapter) in doc.content.chapters.iter().enumerate() {
        // Parse the chapter XHTML into content nodes.
        let base_path = chapter
            .path
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let nodes = crate::epub::render::parse_chapter_xhtml(
            &chapter.content,
            base_path,
            &doc.content.styles,
        );

        // Flatten content nodes into plain text for searching.
        let plain_text = extract_text_from_nodes(&nodes);
        find_matches_in_text(&plain_text, &query_lower, chapter_idx, &mut results);
    }

    results
}

/// Extract plain text from a list of content nodes.
pub fn extract_text_from_nodes(nodes: &[ContentNode]) -> String {
    let mut text = String::new();
    for node in nodes {
        extract_node_text(node, &mut text);
        text.push('\n');
    }
    text
}

fn extract_node_text(node: &ContentNode, out: &mut String) {
    match node {
        ContentNode::Heading { text, .. } => out.push_str(text),
        ContentNode::Paragraph(spans, _) => extract_spans_text(spans, out),
        ContentNode::BlockQuote(children) => {
            for child in children {
                extract_node_text(child, out);
                out.push('\n');
            }
        }
        ContentNode::UnorderedList(items) | ContentNode::OrderedList(items) => {
            for item_spans in items {
                extract_spans_text(item_spans, out);
                out.push('\n');
            }
        }
        ContentNode::CodeBlock { code, .. } => out.push_str(code),
        ContentNode::InlineCode(code) => out.push_str(code),
        ContentNode::Image { alt, .. } => out.push_str(alt),
        ContentNode::HorizontalRule => {}
    }
}

fn extract_spans_text(spans: &[TextSpan], out: &mut String) {
    for span in spans {
        out.push_str(&span.text);
    }
}

/// Find all occurrences of `query_lower` (already lowercased) in `text`
/// (case-insensitive) and append to `results`.
///
/// This is the public entry point for callers that already have extracted text
/// (e.g. the app layer extracting PDF text page-by-page via pdfium).
pub fn find_matches_in_text_pub(
    text: &str,
    query_lower: &str,
    page: usize,
    results: &mut Vec<SearchMatch>,
) {
    find_matches_in_text(text, query_lower, page, results);
}

/// Find all occurrences of `query_lower` in `text` (case-insensitive) and append
/// to `results`.
fn find_matches_in_text(
    text: &str,
    query_lower: &str,
    page: usize,
    results: &mut Vec<SearchMatch>,
) {
    if query_lower.is_empty() {
        return;
    }

    let text_lower = text.to_lowercase();
    let query_len = query_lower.len();

    let mut start = 0;
    while let Some(pos) = text_lower[start..].find(query_lower) {
        let absolute_pos = start + pos;

        // Build a context snippet (up to 40 chars before and after).
        let ctx_start = text.floor_char_boundary(absolute_pos.saturating_sub(40));
        let ctx_end = text.ceil_char_boundary((absolute_pos + query_len + 40).min(text.len()));
        let context = text[ctx_start..ctx_end].trim().replace('\n', " ");

        results.push(SearchMatch {
            page,
            offset: absolute_pos,
            length: query_len,
            context,
        });

        start = absolute_pos + query_len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_matches_basic() {
        let mut results = Vec::new();
        find_matches_in_text("Hello World hello", "hello", 0, &mut results);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].page, 0);
        assert_eq!(results[0].offset, 0);
        assert_eq!(results[1].offset, 12);
    }

    #[test]
    fn test_find_matches_empty_query() {
        let mut results = Vec::new();
        find_matches_in_text("Hello World", "", 0, &mut results);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_find_matches_no_match() {
        let mut results = Vec::new();
        find_matches_in_text("Hello World", "xyz", 0, &mut results);
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_matches_case_insensitive() {
        let mut results = Vec::new();
        find_matches_in_text("Rust Programming in RUST", "rust", 0, &mut results);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_extract_text_from_nodes() {
        let nodes = vec![
            ContentNode::Heading {
                level: 1,
                text: "Chapter One".to_string(),
                style: Default::default(),
            },
            ContentNode::Paragraph(
                vec![
                    TextSpan {
                        text: "Hello ".to_string(),
                        bold: false,
                        italic: false,
                        monospace: false,
                        link: None,
                    },
                    TextSpan {
                        text: "world".to_string(),
                        bold: true,
                        italic: false,
                        monospace: false,
                        link: None,
                    },
                ],
                Default::default(),
            ),
        ];
        let text = extract_text_from_nodes(&nodes);
        assert!(text.contains("Chapter One"));
        assert!(text.contains("Hello world"));
    }

    #[test]
    fn test_context_snippet() {
        let mut results = Vec::new();
        let text = "This is a longer text with the word target somewhere in the middle of it";
        find_matches_in_text(text, "target", 0, &mut results);
        assert_eq!(results.len(), 1);
        assert!(results[0].context.contains("target"));
    }
}
