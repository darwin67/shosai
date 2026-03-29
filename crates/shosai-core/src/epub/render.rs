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
    /// If set, this span is a link to the given URL/href.
    pub link: Option<String>,
}

/// Style annotations that can be applied to any block-level content node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeStyle {
    /// Text alignment override.
    pub text_align: Option<super::style::TextAlignment>,
    /// Font size multiplier override.
    pub font_size_multiplier: Option<f32>,
    /// Left margin in em.
    pub margin_left_em: Option<f32>,
}

/// A content node in the simplified document model.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentNode {
    /// A heading (level 1–6).
    Heading {
        level: u8,
        text: String,
        style: NodeStyle,
    },
    /// A paragraph with mixed inline formatting.
    Paragraph(Vec<TextSpan>, NodeStyle),
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
/// `styles` is the CSS class → style map for applying class-based styles.
pub fn parse_chapter_xhtml(
    xhtml: &str,
    base_path: &str,
    styles: &super::style::StyleMap,
) -> Vec<ContentNode> {
    let doc = match roxmltree::Document::parse(xhtml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    // Find <body> (or fall back to root).
    let body = doc
        .descendants()
        .find(|n| n.tag_name().name() == "body")
        .unwrap_or(doc.root());

    parse_block_children(body, base_path, styles)
}

/// Parse block-level children of an element.
fn parse_block_children(
    parent: roxmltree::Node,
    base_path: &str,
    styles: &super::style::StyleMap,
) -> Vec<ContentNode> {
    let mut nodes = Vec::new();

    for child in parent.children() {
        if !child.is_element() {
            if child.is_text() {
                let text = child.text().unwrap_or("").trim();
                if !text.is_empty() {
                    nodes.push(ContentNode::Paragraph(
                        vec![TextSpan {
                            text: text.to_string(),
                            bold: false,
                            italic: false,
                            monospace: false,
                            link: None,
                        }],
                        NodeStyle::default(),
                    ));
                }
            }
            continue;
        }

        // Look up CSS class-based style for this element.
        let css_style = lookup_element_style(&child, styles);

        // If `display: none`, skip entirely.
        if css_style.as_ref().is_some_and(|s| s.hidden == Some(true)) {
            continue;
        }

        // If the CSS says monospace + preserve-whitespace, treat as code block
        // regardless of the HTML tag (handles Calibre-generated classes).
        if css_style
            .as_ref()
            .is_some_and(|s| s.monospace == Some(true) && s.preserve_whitespace == Some(true))
        {
            let code = collect_text_content(&child);
            if !code.trim().is_empty() {
                nodes.push(ContentNode::CodeBlock {
                    code: code.trim().to_string(),
                    language: None,
                });
                continue;
            }
        }

        let node_style = css_to_node_style(&css_style);

        match child.tag_name().name() {
            "h1" => push_heading(&mut nodes, &child, 1, &node_style),
            "h2" => push_heading(&mut nodes, &child, 2, &node_style),
            "h3" => push_heading(&mut nodes, &child, 3, &node_style),
            "h4" => push_heading(&mut nodes, &child, 4, &node_style),
            "h5" => push_heading(&mut nodes, &child, 5, &node_style),
            "h6" => push_heading(&mut nodes, &child, 6, &node_style),

            "p" => {
                let spans = collect_inline_spans(&child, styles);
                if !spans.is_empty() {
                    nodes.push(ContentNode::Paragraph(spans, node_style));
                }
            }

            "blockquote" => {
                let inner = parse_block_children(child, base_path, styles);
                if !inner.is_empty() {
                    nodes.push(ContentNode::BlockQuote(inner));
                }
            }

            "ul" => {
                let items = parse_list_items(&child, styles);
                if !items.is_empty() {
                    nodes.push(ContentNode::UnorderedList(items));
                }
            }

            "ol" => {
                let items = parse_list_items(&child, styles);
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

            "div" | "section" | "article" | "main" | "aside" | "header" | "footer" | "figure"
            | "figcaption" => {
                nodes.extend(parse_block_children(child, base_path, styles));
            }

            _ => {
                let spans = collect_inline_spans(&child, styles);
                if !spans.is_empty() {
                    nodes.push(ContentNode::Paragraph(spans, node_style));
                }
            }
        }
    }

    nodes
}

/// Look up the CSS style for an element based on its class attribute.
fn lookup_element_style(
    element: &roxmltree::Node,
    styles: &super::style::StyleMap,
) -> Option<super::style::EpubStyle> {
    let class_attr = element.attribute("class")?;
    let mut merged = super::style::EpubStyle::default();
    let mut found = false;
    for cls in class_attr.split_whitespace() {
        if let Some(style) = styles.get(cls) {
            // Simple merge: later classes override earlier ones.
            macro_rules! merge {
                ($field:ident) => {
                    if style.$field.is_some() {
                        merged.$field = style.$field;
                    }
                };
            }
            merge!(bold);
            merge!(italic);
            merge!(monospace);
            merge!(font_size_multiplier);
            merge!(text_align);
            merge!(hidden);
            merge!(text_indent_em);
            merge!(margin_left_em);
            merge!(preserve_whitespace);
            found = true;
        }
    }
    found.then_some(merged)
}

/// Convert a CSS style to a NodeStyle.
fn css_to_node_style(css: &Option<super::style::EpubStyle>) -> NodeStyle {
    match css {
        Some(s) => NodeStyle {
            text_align: s.text_align,
            font_size_multiplier: s.font_size_multiplier,
            margin_left_em: s.margin_left_em,
        },
        None => NodeStyle::default(),
    }
}

/// Collect heading text content.
fn push_heading(
    nodes: &mut Vec<ContentNode>,
    element: &roxmltree::Node,
    level: u8,
    node_style: &NodeStyle,
) {
    let text = collect_text_content(element).trim().to_string();
    if !text.is_empty() {
        nodes.push(ContentNode::Heading {
            level,
            text,
            style: node_style.clone(),
        });
    }
}

/// Parse <li> items from a <ul> or <ol>.
fn parse_list_items(list: &roxmltree::Node, styles: &super::style::StyleMap) -> Vec<Vec<TextSpan>> {
    let mut items = Vec::new();
    for child in list.children() {
        if child.is_element() && child.tag_name().name() == "li" {
            let spans = collect_inline_spans(&child, styles);
            if !spans.is_empty() {
                items.push(spans);
            }
        }
    }
    items
}

/// Collect inline text spans with bold/italic formatting from an element.
fn collect_inline_spans(
    element: &roxmltree::Node,
    styles: &super::style::StyleMap,
) -> Vec<TextSpan> {
    let mut spans = Vec::new();
    // Check if the element itself has CSS-based styling.
    let css = lookup_element_style(element, styles);
    let bold = css.as_ref().is_some_and(|s| s.bold == Some(true));
    let italic = css.as_ref().is_some_and(|s| s.italic == Some(true));
    let mono = css.as_ref().is_some_and(|s| s.monospace == Some(true));
    collect_inline_spans_recursive(element, bold, italic, mono, None, styles, &mut spans);

    // Merge adjacent spans with the same formatting.
    merge_spans(&mut spans);
    spans
}

fn collect_inline_spans_recursive(
    node: &roxmltree::Node,
    bold: bool,
    italic: bool,
    monospace: bool,
    link: Option<&str>,
    styles: &super::style::StyleMap,
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
                    link: link.map(|s| s.to_string()),
                });
            }
        } else if child.is_element() {
            // Apply CSS class-based overrides for this inline element.
            let css = lookup_element_style(&child, styles);
            let css_bold = css.as_ref().is_some_and(|s| s.bold == Some(true));
            let css_italic = css.as_ref().is_some_and(|s| s.italic == Some(true));
            let css_mono = css.as_ref().is_some_and(|s| s.monospace == Some(true));

            match child.tag_name().name() {
                "a" => {
                    let href = child.attribute("href");
                    collect_inline_spans_recursive(
                        &child,
                        bold || css_bold,
                        italic || css_italic,
                        monospace || css_mono,
                        href,
                        styles,
                        spans,
                    );
                }
                tag => {
                    let (b, i, m) = match tag {
                        "b" | "strong" => (true, italic, monospace),
                        "i" | "em" | "cite" => (bold, true, monospace),
                        "bi" => (true, true, monospace),
                        "code" | "tt" | "samp" | "kbd" => (bold, italic, true),
                        _ => (bold, italic, monospace),
                    };
                    collect_inline_spans_recursive(
                        &child,
                        b || css_bold,
                        i || css_italic,
                        m || css_mono,
                        link,
                        styles,
                        spans,
                    );
                }
            }
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
            && spans[i].link == spans[i + 1].link
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans, _) => {
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 2);
        assert!(
            matches!(&nodes[0], ContentNode::Heading { level: 1, text, .. } if text == "Title")
        );
        assert!(
            matches!(&nodes[1], ContentNode::Heading { level: 2, text, .. } if text == "Subtitle")
        );
    }

    #[test]
    fn test_parse_bold_italic() {
        let xhtml =
            r#"<html><body><p>Normal <strong>bold</strong> and <em>italic</em></p></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans, _) => {
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 2);
        assert!(matches!(&nodes[0], ContentNode::UnorderedList(items) if items.len() == 2));
        assert!(matches!(&nodes[1], ContentNode::OrderedList(items) if items.len() == 2));
    }

    #[test]
    fn test_parse_blockquote() {
        let xhtml = r#"<html><body><blockquote><p>Quoted text</p></blockquote></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::BlockQuote(inner) => {
                assert_eq!(inner.len(), 1);
                assert!(matches!(&inner[0], ContentNode::Paragraph(_, _)));
            }
            other => panic!("expected BlockQuote, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_image() {
        let xhtml = r#"<html><body><img src="images/fig1.png" alt="Figure 1"/></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "OEBPS", &Default::default());
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], ContentNode::HorizontalRule));
    }

    #[test]
    fn test_parse_div_wrapper() {
        let xhtml = r#"<html><body><div><p>Inside div</p></div></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], ContentNode::Paragraph(_, _)));
    }

    #[test]
    fn test_parse_pre_code_block() {
        let xhtml = r#"<html><body><pre class="language-rust">fn main() {
    println!("hello");
}</pre></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
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
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans, _) => {
                let mono_span = spans.iter().find(|s| s.monospace);
                assert!(mono_span.is_some(), "should have a monospace span");
                assert_eq!(mono_span.unwrap().text, "println!");
            }
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_link() {
        let xhtml = r#"<html><body><p>Visit <a href="https://example.com">our site</a> today</p></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans, _) => {
                let link_span = spans.iter().find(|s| s.link.is_some());
                assert!(link_span.is_some(), "should have a link span");
                let link_span = link_span.unwrap();
                assert_eq!(link_span.text, "our site");
                assert_eq!(link_span.link.as_deref(), Some("https://example.com"));
            }
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_bold_link() {
        let xhtml =
            r#"<html><body><p><a href="url"><strong>bold link</strong></a></p></body></html>"#;
        let nodes = parse_chapter_xhtml(xhtml, "", &Default::default());
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            ContentNode::Paragraph(spans, _) => {
                let link_span = spans.iter().find(|s| s.link.is_some());
                assert!(link_span.is_some());
                let link_span = link_span.unwrap();
                assert!(link_span.bold, "link should be bold");
                assert_eq!(link_span.link.as_deref(), Some("url"));
            }
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }
}
