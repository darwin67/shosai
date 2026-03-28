//! Simplified XHTML → content model renderer for EPUB chapters.
//!
//! Parses EPUB chapter XHTML into a flat list of [`ContentNode`] values that
//! the GUI layer can map to native widgets. Complex CSS is intentionally
//! ignored; only structural HTML elements are interpreted.

/// A styled span of inline text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
}

/// A content node in the simplified document model.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentNode {
    /// A heading (level 1–6).
    Heading { level: u8, text: String },
    /// A paragraph with mixed inline formatting.
    Paragraph(Vec<TextSpan>),
    /// A block quote (contains paragraphs).
    BlockQuote(Vec<ContentNode>),
    /// An unordered list.
    UnorderedList(Vec<Vec<TextSpan>>),
    /// An ordered list.
    OrderedList(Vec<Vec<TextSpan>>),
    /// An image reference.
    Image {
        /// Path to the image within the EPUB archive.
        src: String,
        alt: String,
    },
    /// A code block (`<pre>`, `<code>` block-level, or `<pre><code>`).
    CodeBlock {
        /// The raw code text.
        code: String,
        /// Optional language hint from class (e.g. "language-rust", "python").
        language: Option<String>,
    },
    /// Inline code (`<code>` inside a paragraph).
    InlineCode(String),
    /// A horizontal rule / thematic break.
    HorizontalRule,
}

/// Parse chapter XHTML into a list of content nodes.
///
/// `base_path` is the directory of the chapter within the EPUB archive,
/// used to resolve relative image `src` attributes.
pub fn parse_chapter_xhtml(xhtml: &str, base_path: &str) -> Vec<ContentNode> {
    let doc = match roxmltree::Document::parse(xhtml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    // Find <body> (or fall back to root).
    let body = doc
        .descendants()
        .find(|n| n.tag_name().name() == "body")
        .unwrap_or(doc.root());

    parse_block_children(body, base_path)
}

/// Parse block-level children of an element.
fn parse_block_children(parent: roxmltree::Node, base_path: &str) -> Vec<ContentNode> {
    let mut nodes = Vec::new();

    for child in parent.children() {
        if !child.is_element() {
            // Bare text at block level → treat as paragraph.
            if child.is_text() {
                let text = child.text().unwrap_or("").trim();
                if !text.is_empty() {
                    nodes.push(ContentNode::Paragraph(vec![TextSpan {
                        text: text.to_string(),
                        bold: false,
                        italic: false,
                        monospace: false,
                    }]));
                }
            }
            continue;
        }

        match child.tag_name().name() {
            "h1" => push_heading(&mut nodes, &child, 1),
            "h2" => push_heading(&mut nodes, &child, 2),
            "h3" => push_heading(&mut nodes, &child, 3),
            "h4" => push_heading(&mut nodes, &child, 4),
            "h5" => push_heading(&mut nodes, &child, 5),
            "h6" => push_heading(&mut nodes, &child, 6),

            "p" => {
                let spans = collect_inline_spans(&child);
                if !spans.is_empty() {
                    nodes.push(ContentNode::Paragraph(spans));
                }
            }

            "blockquote" => {
                let inner = parse_block_children(child, base_path);
                if !inner.is_empty() {
                    nodes.push(ContentNode::BlockQuote(inner));
                }
            }

            "ul" => {
                let items = parse_list_items(&child);
                if !items.is_empty() {
                    nodes.push(ContentNode::UnorderedList(items));
                }
            }

            "ol" => {
                let items = parse_list_items(&child);
                if !items.is_empty() {
                    nodes.push(ContentNode::OrderedList(items));
                }
            }

            "pre" => {
                let language = extract_language_hint(&child);
                let code = collect_text_content(&child);
                if !code.trim().is_empty() {
                    nodes.push(ContentNode::CodeBlock {
                        code: code.trim().to_string(),
                        language,
                    });
                }
            }

            "code" => {
                // Block-level <code> (not inside <p>) — treat as code block.
                let language = extract_language_hint(&child);
                let code = collect_text_content(&child);
                if !code.trim().is_empty() {
                    nodes.push(ContentNode::CodeBlock {
                        code: code.trim().to_string(),
                        language,
                    });
                }
            }

            "img" => {
                if let Some(src) = child.attribute("src") {
                    let alt = child.attribute("alt").unwrap_or("").to_string();
                    nodes.push(ContentNode::Image {
                        src: resolve_relative(base_path, src),
                        alt,
                    });
                }
            }

            "hr" => {
                nodes.push(ContentNode::HorizontalRule);
            }

            // Wrapper elements: recurse into them.
            "div" | "section" | "article" | "main" | "aside" | "header" | "footer" | "figure"
            | "figcaption" => {
                nodes.extend(parse_block_children(child, base_path));
            }

            // Anything else: try to extract text content as a paragraph.
            _ => {
                let spans = collect_inline_spans(&child);
                if !spans.is_empty() {
                    nodes.push(ContentNode::Paragraph(spans));
                }
            }
        }
    }

    nodes
}

/// Collect heading text content.
fn push_heading(nodes: &mut Vec<ContentNode>, element: &roxmltree::Node, level: u8) {
    let text = collect_text_content(element).trim().to_string();
    if !text.is_empty() {
        nodes.push(ContentNode::Heading { level, text });
    }
}

/// Parse <li> items from a <ul> or <ol>.
fn parse_list_items(list: &roxmltree::Node) -> Vec<Vec<TextSpan>> {
    let mut items = Vec::new();
    for child in list.children() {
        if child.is_element() && child.tag_name().name() == "li" {
            let spans = collect_inline_spans(&child);
            if !spans.is_empty() {
                items.push(spans);
            }
        }
    }
    items
}

/// Collect inline text spans with bold/italic formatting from an element.
fn collect_inline_spans(element: &roxmltree::Node) -> Vec<TextSpan> {
    let mut spans = Vec::new();
    collect_inline_spans_recursive(element, false, false, false, &mut spans);

    // Merge adjacent spans with the same formatting.
    merge_spans(&mut spans);
    spans
}

fn collect_inline_spans_recursive(
    node: &roxmltree::Node,
    bold: bool,
    italic: bool,
    monospace: bool,
    spans: &mut Vec<TextSpan>,
) {
    for child in node.children() {
        if child.is_text() {
            let text = child.text().unwrap_or("");
            if !text.is_empty() {
                spans.push(TextSpan {
                    text: text.to_string(),
                    bold,
                    italic,
                    monospace,
                });
            }
        } else if child.is_element() {
            let (b, i, m) = match child.tag_name().name() {
                "b" | "strong" => (true, italic, monospace),
                "i" | "em" | "cite" => (bold, true, monospace),
                "bi" => (true, true, monospace),
                "code" | "tt" | "samp" | "kbd" => (bold, italic, true),
                _ => (bold, italic, monospace),
            };
            collect_inline_spans_recursive(&child, b, i, m, spans);
        }
    }
}

/// Merge adjacent spans that have the same formatting.
fn merge_spans(spans: &mut Vec<TextSpan>) {
    let mut i = 0;
    while i + 1 < spans.len() {
        if spans[i].bold == spans[i + 1].bold
            && spans[i].italic == spans[i + 1].italic
            && spans[i].monospace == spans[i + 1].monospace
        {
            let next_text = spans[i + 1].text.clone();
            spans[i].text.push_str(&next_text);
            spans.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

/// Recursively collect all text content from an element.
fn collect_text_content(node: &roxmltree::Node) -> String {
    let mut text = String::new();
    for child in node.children() {
        if child.is_text() {
            text.push_str(child.text().unwrap_or(""));
        } else if child.is_element() {
            text.push_str(&collect_text_content(&child));
        }
    }
    text
}

/// Extract a language hint from a `class` attribute.
///
/// Looks for patterns like `language-rust`, `lang-python`, `code-erlang`,
/// `sourceCode erlang`, or bare language names in the class of the element
/// or its first `<code>` child.
fn extract_language_hint(node: &roxmltree::Node) -> Option<String> {
    // Check the node itself and its first <code> child.
    let classes = [
        node.attribute("class"),
        node.children()
            .find(|c| c.is_element() && c.tag_name().name() == "code")
            .and_then(|c| c.attribute("class")),
    ];

    for class_attr in classes.into_iter().flatten() {
        for cls in class_attr.split_whitespace() {
            let lang = cls
                .strip_prefix("language-")
                .or_else(|| cls.strip_prefix("lang-"))
                .or_else(|| cls.strip_prefix("code-"))
                .or_else(|| cls.strip_prefix("sourceCode"));

            if let Some(l) = lang
                && !l.is_empty()
            {
                return Some(l.to_lowercase());
            }
        }
    }

    None
}

/// Resolve a relative path against a base directory.
fn resolve_relative(base: &str, href: &str) -> String {
    if href.starts_with('/') || href.contains("://") {
        return href.to_string();
    }
    if base.is_empty() {
        return href.to_string();
    }
    format!("{base}/{href}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_paragraph() {
        let xhtml = r#"<html><body><p>Hello world</p></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans) => {
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].text, "Hello world");
                assert!(!spans[0].bold);
                assert!(!spans[0].italic);
            }
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_heading() {
        let xhtml = r#"<html><body><h1>Title</h1><h2>Subtitle</h2></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 2);
        assert!(matches!(&nodes[0], ContentNode::Heading { level: 1, text } if text == "Title"));
        assert!(matches!(&nodes[1], ContentNode::Heading { level: 2, text } if text == "Subtitle"));
    }

    #[test]
    fn test_parse_bold_italic() {
        let xhtml =
            r#"<html><body><p>Normal <strong>bold</strong> and <em>italic</em></p></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans) => {
                // "Normal " (plain), "bold" (bold), " and " (plain), "italic" (italic)
                assert!(spans.len() >= 3, "expected at least 3 spans: {spans:?}");
                let bold_span = spans.iter().find(|s| s.bold);
                assert!(bold_span.is_some(), "should have a bold span");
                assert_eq!(bold_span.unwrap().text, "bold");

                let italic_span = spans.iter().find(|s| s.italic);
                assert!(italic_span.is_some(), "should have an italic span");
                assert_eq!(italic_span.unwrap().text, "italic");
            }
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_lists() {
        let xhtml = r#"<html><body>
            <ul><li>One</li><li>Two</li></ul>
            <ol><li>First</li><li>Second</li></ol>
        </body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 2);
        assert!(matches!(&nodes[0], ContentNode::UnorderedList(items) if items.len() == 2));
        assert!(matches!(&nodes[1], ContentNode::OrderedList(items) if items.len() == 2));
    }

    #[test]
    fn test_parse_blockquote() {
        let xhtml = r#"<html><body><blockquote><p>Quoted text</p></blockquote></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::BlockQuote(inner) => {
                assert_eq!(inner.len(), 1);
                assert!(matches!(&inner[0], ContentNode::Paragraph(_)));
            }
            other => panic!("expected BlockQuote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_image() {
        let xhtml = r#"<html><body><img src="images/fig1.png" alt="Figure 1"/></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "OEBPS");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Image { src, alt } => {
                assert_eq!(src, "OEBPS/images/fig1.png");
                assert_eq!(alt, "Figure 1");
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_hr() {
        let xhtml = r#"<html><body><hr/></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], ContentNode::HorizontalRule));
    }

    #[test]
    fn test_parse_div_wrapper() {
        let xhtml = r#"<html><body><div><p>Inside div</p></div></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], ContentNode::Paragraph(_)));
    }

    #[test]
    fn test_parse_pre_code_block() {
        let xhtml = r#"<html><body><pre class="language-rust">fn main() {
    println!("hello");
}</pre></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::CodeBlock { code, language } => {
                assert!(code.contains("fn main()"), "code should contain fn main()");
                assert_eq!(language.as_deref(), Some("rust"));
            }
            other => panic!("expected CodeBlock, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_pre_without_language() {
        let xhtml = r#"<html><body><pre>some plain text</pre></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::CodeBlock { code, language } => {
                assert_eq!(code, "some plain text");
                assert!(language.is_none());
            }
            other => panic!("expected CodeBlock, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_pre_code_nested() {
        let xhtml =
            r#"<html><body><pre><code class="lang-python">print("hi")</code></pre></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::CodeBlock { code, language } => {
                assert!(code.contains("print"));
                assert_eq!(language.as_deref(), Some("python"));
            }
            other => panic!("expected CodeBlock, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_inline_code() {
        let xhtml = r#"<html><body><p>Use <code>println!</code> to print</p></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans) => {
                let mono_span = spans.iter().find(|s| s.monospace);
                assert!(mono_span.is_some(), "should have a monospace span");
                assert_eq!(mono_span.unwrap().text, "println!");
            }
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }
}
