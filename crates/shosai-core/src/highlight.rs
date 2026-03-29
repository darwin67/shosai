//! Syntax highlighting for code blocks using syntect.
//!
//! Provides highlighted code as a list of colored spans that the GUI layer
//! can render with appropriate colors.

use syntect::highlighting::{Color, ThemeSet};
use syntect::parsing::SyntaxSet;

/// A single highlighted span of code.
#[derive(Debug, Clone)]
pub struct HighlightSpan {
    pub text: String,
    pub color: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
}

/// Highlight a code string, returning a list of spans per line.
///
/// If the language is not recognized, returns `None` and the caller
/// should fall back to plain monospace rendering.
pub fn highlight_code(
    code: &str,
    language: Option<&str>,
    theme_name: &str,
) -> Option<Vec<Vec<HighlightSpan>>> {
    let ss = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();

    let theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.get("base16-ocean.dark"))?;

    let syntax = language
        .and_then(|lang| {
            ss.find_syntax_by_token(lang)
                .or_else(|| ss.find_syntax_by_name(lang))
        })
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut highlighter = syntect::easy::HighlightLines::new(syntax, theme);

    let mut result = Vec::new();
    for line in syntect::util::LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, &ss).ok()?;
        let spans: Vec<HighlightSpan> = ranges
            .into_iter()
            .map(|(style, text)| {
                let Color { r, g, b, .. } = style.foreground;
                HighlightSpan {
                    text: text.to_string(),
                    color: (r, g, b),
                    bold: style
                        .font_style
                        .contains(syntect::highlighting::FontStyle::BOLD),
                    italic: style
                        .font_style
                        .contains(syntect::highlighting::FontStyle::ITALIC),
                }
            })
            .collect();
        result.push(spans);
    }

    Some(result)
}

/// Get the background color for a given theme.
pub fn theme_background(theme_name: &str) -> Option<(u8, u8, u8)> {
    let ts = ThemeSet::load_defaults();
    let theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.get("base16-ocean.dark"))?;
    let bg = theme.settings.background?;
    Some((bg.r, bg.g, bg.b))
}

/// Return the appropriate syntect theme name for a reader theme.
pub fn syntect_theme_for_reader(dark: bool) -> &'static str {
    if dark {
        "base16-ocean.dark"
    } else {
        "base16-ocean.light"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_known_language() {
        let code = "fn main() {\n    println!(\"hello\");\n}";
        let result = highlight_code(code, Some("rust"), "base16-ocean.dark");
        assert!(result.is_some(), "should highlight Rust code");
        let lines = result.unwrap();
        assert_eq!(lines.len(), 3, "should have 3 lines");
        // First line should have spans
        assert!(!lines[0].is_empty(), "first line should have spans");
    }

    #[test]
    fn test_highlight_unknown_language_fallback() {
        let code = "some random text";
        let result = highlight_code(code, Some("nonexistent_lang_xyz"), "base16-ocean.dark");
        // Should still produce output (falls back to plain text syntax)
        assert!(result.is_some());
    }

    #[test]
    fn test_highlight_no_language() {
        let code = "plain text here";
        let result = highlight_code(code, None, "base16-ocean.dark");
        assert!(result.is_some());
    }

    #[test]
    fn test_theme_background() {
        let bg = theme_background("base16-ocean.dark");
        assert!(bg.is_some(), "should have a background color");
    }
}
