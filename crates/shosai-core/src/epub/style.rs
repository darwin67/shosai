//! Tier 1 CSS style extraction for EPUB rendering.
//!
//! Parses CSS stylesheets from EPUB resources and builds a map from class
//! names to a simplified [`EpubStyle`] that the renderer can apply. Only
//! simple `.class` selectors are matched; combinators and pseudo-classes are
//! ignored.

use std::collections::HashMap;

use lightningcss::properties::Property;
use lightningcss::properties::display::{Display, DisplayKeyword};
use lightningcss::properties::font::{
    AbsoluteFontWeight, FontFamily, FontSize, FontStyle as CssFontStyle, FontWeight,
};
use lightningcss::properties::text::{TextAlign, WhiteSpace};
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use lightningcss::traits::ToCss;
use lightningcss::values::length::LengthPercentageOrAuto;
use lightningcss::values::length::LengthValue;
use lightningcss::values::percentage::DimensionPercentage;

/// Simplified style extracted from CSS.
///
/// Each field is `Option` — `None` means the property wasn't set by this rule,
/// so the default or inherited value applies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EpubStyle {
    /// `font-weight: bold`
    pub bold: Option<bool>,
    /// `font-style: italic`
    pub italic: Option<bool>,
    /// `font-family` contains a monospace family
    pub monospace: Option<bool>,
    /// Font size as a multiplier relative to the base size (e.g. 1.5 = 150%).
    pub font_size_multiplier: Option<f32>,
    /// `text-align`
    pub text_align: Option<TextAlignment>,
    /// `display: none`
    pub hidden: Option<bool>,
    /// `text-indent` in em.
    pub text_indent_em: Option<f32>,
    /// `margin-left` in em.
    pub margin_left_em: Option<f32>,
    /// `white-space: pre` or `pre-wrap`
    pub preserve_whitespace: Option<bool>,
}

/// Simplified text alignment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
    Justify,
}

/// A map from CSS class name to extracted style.
pub type StyleMap = HashMap<String, EpubStyle>;

/// Parse all CSS resources from an EPUB and build a class → style map.
///
/// `css_sources` is an iterator of `(path, css_text)` pairs.
pub fn parse_epub_styles<'a>(
    css_sources: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> StyleMap {
    let mut map = StyleMap::new();

    for (_path, css_text) in css_sources {
        let Ok(sheet) = StyleSheet::parse(css_text, ParserOptions::default()) else {
            continue;
        };

        extract_rules(&sheet.rules.0, &mut map);
    }

    map
}

/// Recursively extract style rules.
fn extract_rules<'i>(rules: &[CssRule<'i>], map: &mut StyleMap) {
    for rule in rules {
        match rule {
            CssRule::Style(style_rule) => {
                let style = extract_style(&style_rule.declarations);
                if style == EpubStyle::default() {
                    continue;
                }

                // Extract class names from selectors.
                // Serialize the full selector and extract .class patterns.
                for selector in style_rule.selectors.0.iter() {
                    let selector_str = selector
                        .to_css_string(Default::default())
                        .unwrap_or_default();
                    for class_name in extract_class_names_from_str(&selector_str) {
                        let entry = map.entry(class_name).or_default();
                        merge_style(entry, &style);
                    }
                }
            }
            CssRule::Media(media) => extract_rules(&media.rules.0, map),
            CssRule::Supports(supports) => extract_rules(&supports.rules.0, map),
            _ => {}
        }
    }
}

/// Extract relevant CSS properties from a declaration block into an EpubStyle.
fn extract_style(declarations: &lightningcss::declaration::DeclarationBlock) -> EpubStyle {
    let mut style = EpubStyle::default();

    for prop in declarations
        .declarations
        .iter()
        .chain(declarations.important_declarations.iter())
    {
        match prop {
            Property::FontWeight(fw) => {
                style.bold = Some(is_bold(fw));
            }
            Property::FontStyle(fs) => {
                style.italic = Some(matches!(
                    fs,
                    CssFontStyle::Italic | CssFontStyle::Oblique(_)
                ));
            }
            Property::FontFamily(families) => {
                style.monospace = Some(has_monospace_family(families));
            }
            Property::FontSize(size) => {
                style.font_size_multiplier = font_size_to_multiplier(size);
            }
            Property::TextAlign(ta) => {
                style.text_align = Some(match ta {
                    TextAlign::Left => TextAlignment::Left,
                    TextAlign::Center => TextAlignment::Center,
                    TextAlign::Right => TextAlignment::Right,
                    TextAlign::Justify => TextAlignment::Justify,
                    _ => TextAlignment::Left,
                });
            }
            Property::Display(display) => {
                style.hidden = Some(matches!(display, Display::Keyword(DisplayKeyword::None)));
            }
            Property::TextIndent(indent) => {
                style.text_indent_em = dim_percentage_to_em(&indent.value);
            }
            Property::MarginLeft(LengthPercentageOrAuto::LengthPercentage(lp)) => {
                style.margin_left_em = dim_percentage_to_em(lp);
            }
            Property::WhiteSpace(ws) => {
                style.preserve_whitespace = Some(matches!(
                    ws,
                    WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
                ));
            }
            _ => {}
        }
    }

    style
}

fn is_bold(fw: &FontWeight) -> bool {
    match fw {
        FontWeight::Absolute(AbsoluteFontWeight::Weight(w)) => *w >= 600.0,
        FontWeight::Absolute(AbsoluteFontWeight::Bold) => true,
        FontWeight::Bolder => true,
        _ => false,
    }
}

fn has_monospace_family(families: &[FontFamily]) -> bool {
    families.iter().any(|f| {
        match f {
            FontFamily::Generic(g) => {
                use lightningcss::properties::font::GenericFontFamily;
                matches!(
                    g,
                    GenericFontFamily::Monospace | GenericFontFamily::UIMonospace
                )
            }
            FontFamily::FamilyName(name) => {
                // FamilyName.0 is private, use ToCss to get the string.
                let s = name.to_css_string(Default::default()).unwrap_or_default();
                let lower = s.to_lowercase();
                lower.contains("mono")
                    || lower.contains("courier")
                    || lower.contains("consolas")
                    || lower == "\"menlo\""
                    || lower == "menlo"
            }
        }
    })
}

fn font_size_to_multiplier(size: &FontSize) -> Option<f32> {
    match size {
        FontSize::Length(lp) => dim_percentage_to_multiplier(lp),
        _ => None,
    }
}

fn dim_percentage_to_em(lp: &DimensionPercentage<LengthValue>) -> Option<f32> {
    match lp {
        DimensionPercentage::Dimension(lv) => length_value_to_em(lv),
        DimensionPercentage::Percentage(p) => Some(p.0),
        _ => None,
    }
}

fn dim_percentage_to_multiplier(lp: &DimensionPercentage<LengthValue>) -> Option<f32> {
    match lp {
        DimensionPercentage::Dimension(lv) => length_value_to_multiplier(lv),
        DimensionPercentage::Percentage(p) => Some(p.0),
        _ => None,
    }
}

fn length_value_to_em(lv: &LengthValue) -> Option<f32> {
    match *lv {
        LengthValue::Em(v) => Some(v),
        LengthValue::Rem(v) => Some(v),
        LengthValue::Px(v) => Some(v / 16.0),
        LengthValue::Pt(v) => Some(v / 12.0),
        _ => None,
    }
}

fn length_value_to_multiplier(lv: &LengthValue) -> Option<f32> {
    match *lv {
        LengthValue::Em(v) => Some(v),
        LengthValue::Rem(v) => Some(v),
        LengthValue::Px(v) => Some(v / 16.0),
        LengthValue::Pt(v) => Some(v / 12.0),
        _ => None,
    }
}

/// Extract class names from a serialized selector string.
///
/// Finds `.classname` patterns, handling compound selectors like `p.foo.bar`.
fn extract_class_names_from_str(selector: &str) -> Vec<String> {
    let mut classes = Vec::new();
    for part in selector.split([' ', '>', '+', '~', ',']) {
        let part = part.trim();
        for segment in part.split('.').skip(1) {
            // segment may contain further selectors like "foo:hover" or "foo[attr]"
            let class = segment.split([':', '[', '#']).next().unwrap_or("").trim();
            if !class.is_empty() {
                classes.push(class.to_string());
            }
        }
    }
    classes
}

/// Merge `source` into `target`, overriding only the fields that `source` sets.
fn merge_style(target: &mut EpubStyle, source: &EpubStyle) {
    macro_rules! merge_field {
        ($field:ident) => {
            if source.$field.is_some() {
                target.$field = source.$field;
            }
        };
    }
    merge_field!(bold);
    merge_field!(italic);
    merge_field!(monospace);
    merge_field!(font_size_multiplier);
    merge_field!(text_align);
    merge_field!(hidden);
    merge_field!(text_indent_em);
    merge_field!(margin_left_em);
    merge_field!(preserve_whitespace);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bold() {
        let css = r#".bold-text { font-weight: bold; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        let style = map.get("bold-text").expect("should find bold-text class");
        assert_eq!(style.bold, Some(true));
    }

    #[test]
    fn test_parse_italic() {
        let css = r#".em { font-style: italic; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(map.get("em").unwrap().italic, Some(true));
    }

    #[test]
    fn test_parse_monospace() {
        let css = r#".code { font-family: "Liberation Mono", monospace; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(map.get("code").unwrap().monospace, Some(true));
    }

    #[test]
    fn test_parse_font_size_em() {
        let css = r#".big { font-size: 2em; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        let m = map.get("big").unwrap().font_size_multiplier.unwrap();
        assert!((m - 2.0).abs() < 0.01, "expected 2.0, got {m}");
    }

    #[test]
    fn test_parse_font_size_percent() {
        let css = r#".small { font-size: 75%; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        let m = map.get("small").unwrap().font_size_multiplier.unwrap();
        assert!((m - 0.75).abs() < 0.01, "expected 0.75, got {m}");
    }

    #[test]
    fn test_parse_text_align_center() {
        let css = r#".centered { text-align: center; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(
            map.get("centered").unwrap().text_align,
            Some(TextAlignment::Center)
        );
    }

    #[test]
    fn test_parse_display_none() {
        let css = r#".hidden { display: none; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(map.get("hidden").unwrap().hidden, Some(true));
    }

    #[test]
    fn test_parse_display_block_not_hidden() {
        let css = r#".visible { display: block; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(map.get("visible").unwrap().hidden, Some(false));
    }

    #[test]
    fn test_parse_whitespace_pre() {
        let css = r#".pre { white-space: pre-wrap; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(map.get("pre").unwrap().preserve_whitespace, Some(true));
    }

    #[test]
    fn test_parse_multiple_classes() {
        let css = r#"
            .a { font-weight: bold; }
            .b { font-style: italic; }
        "#;
        let map = parse_epub_styles([("style.css", css)]);
        assert_eq!(map.get("a").unwrap().bold, Some(true));
        assert_eq!(map.get("b").unwrap().italic, Some(true));
    }

    #[test]
    fn test_merge_rules_for_same_class() {
        let css = r#"
            .foo { font-weight: bold; }
            .foo { font-style: italic; }
        "#;
        let map = parse_epub_styles([("style.css", css)]);
        let style = map.get("foo").unwrap();
        assert_eq!(style.bold, Some(true));
        assert_eq!(style.italic, Some(true));
    }

    #[test]
    fn test_compound_selector_extracts_class() {
        let css = r#"p.indent { text-indent: 1.5em; }"#;
        let map = parse_epub_styles([("style.css", css)]);
        let style = map.get("indent").unwrap();
        assert!((style.text_indent_em.unwrap() - 1.5).abs() < 0.01);
    }

    #[test]
    fn test_calibre_code_class() {
        let css = r#".calibre14 {
            display: block;
            font-family: "Liberation Mono", monospace;
            font-size: 0.77778em;
            white-space: pre-wrap;
        }"#;
        let map = parse_epub_styles([("style.css", css)]);
        let style = map.get("calibre14").unwrap();
        assert_eq!(style.monospace, Some(true));
        assert_eq!(style.preserve_whitespace, Some(true));
        assert!(style.font_size_multiplier.is_some());
    }

    #[test]
    fn test_malformed_css_skipped() {
        let css = r#"this is not { valid css @@@ "#;
        let map = parse_epub_styles([("bad.css", css)]);
        let _ = map;
    }

    #[test]
    fn test_multiple_css_files() {
        let css1 = r#".a { font-weight: bold; }"#;
        let css2 = r#".b { font-style: italic; }"#;
        let map = parse_epub_styles([("a.css", css1), ("b.css", css2)]);
        assert!(map.contains_key("a"));
        assert!(map.contains_key("b"));
    }
}
