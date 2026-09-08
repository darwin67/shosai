//! Executable Phase 0 proofs for RFD 6. Production integration follows in Phases 2 and 3.

use std::ops::Range;

use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, renderer, widget::Tree};
use iced::{ContentFit, Element, Event, Length, Point, Rectangle, Size, mouse, widget::image};
use shosai_core::pdf::{PdfSelectionEndpoint, PdfSelectionSnapshot};

const MAX_ENDPOINTS_PER_SURFACE: usize = 65_536;
const MAX_RETAINED_BYTES_PER_DOCUMENT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct EndpointZone {
    bounds: Rectangle,
    scalar: usize,
}

#[derive(Clone, Debug)]
struct EndpointRow {
    bounds: Rectangle,
    zones: Vec<EndpointZone>,
}

#[derive(Clone, Debug)]
struct SelectableTextSurface {
    rows: Vec<EndpointRow>,
}

impl SelectableTextSurface {
    fn hit_test(&self, point: Point) -> Option<usize> {
        self.rows
            .iter()
            .filter(|row| row.bounds.contains(point))
            .find_map(|row| {
                row.zones
                    .iter()
                    .find(|zone| zone.bounds.contains(point))
                    .map(|zone| zone.scalar)
            })
    }

    fn endpoint_count(&self) -> usize {
        self.rows.iter().map(|row| row.zones.len()).sum()
    }

    fn retained_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<EndpointRow>()
            + self
                .rows
                .iter()
                .map(|row| row.zones.capacity() * std::mem::size_of::<EndpointZone>())
                .sum::<usize>()
    }
}

#[derive(Default)]
struct SelectionGeometryBudget {
    retained_bytes: usize,
}

impl SelectionGeometryBudget {
    fn admit(&mut self, endpoint_count: usize, retained_bytes: usize) -> bool {
        if endpoint_count > MAX_ENDPOINTS_PER_SURFACE {
            return false;
        }
        let Some(total) = self.retained_bytes.checked_add(retained_bytes) else {
            return false;
        };
        if total > MAX_RETAINED_BYTES_PER_DOCUMENT {
            return false;
        }
        self.retained_bytes = total;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LogicalEndpoint {
    spine: usize,
    scalar: usize,
}

fn logical_range(anchor: LogicalEndpoint, focus: LogicalEndpoint) -> Option<Range<usize>> {
    (anchor.spine == focus.spine)
        .then(|| anchor.scalar.min(focus.scalar)..anchor.scalar.max(focus.scalar))
}

fn image_content_bounds(widget: Rectangle, bitmap: (u32, u32)) -> Rectangle {
    let fitted =
        ContentFit::Contain.fit(Size::new(bitmap.0 as f32, bitmap.1 as f32), widget.size());
    Rectangle::new(
        Point::new(
            widget.center_x() - fitted.width / 2.0,
            widget.center_y() - fitted.height / 2.0,
        ),
        fitted,
    )
}

fn widget_to_bitmap(point: Point, widget: Rectangle, bitmap: (u32, u32)) -> Option<Point> {
    let content = image_content_bounds(widget, bitmap);
    content.contains(point).then(|| {
        Point::new(
            (point.x - content.x) * bitmap.0 as f32 / content.width,
            (point.y - content.y) * bitmap.1 as f32 / content.height,
        )
    })
}

/// Test-only Iced widget proving that pointer handling consumes only an owned PDF snapshot.
struct SelectablePdfPage<'a, Message> {
    snapshot: &'a PdfSelectionSnapshot,
    raster: image::Handle,
    on_endpoint: fn(PdfSelectionEndpoint) -> Message,
}

impl<Message> SelectablePdfPage<'_, Message> {
    fn endpoint_at(&self, bounds: Rectangle, position: Point) -> Option<PdfSelectionEndpoint> {
        let bitmap = widget_to_bitmap(position, bounds, self.snapshot.bitmap_size())?;
        self.snapshot.hit_test(bitmap.x, bitmap.y)
    }
}

impl<Message, Renderer> Widget<Message, iced::Theme, Renderer> for SelectablePdfPage<'_, Message>
where
    Renderer: iced::advanced::image::Renderer<Handle = image::Handle>,
{
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        renderer.draw_image(
            iced::advanced::image::Image::new(self.raster.clone()),
            image_content_bounds(layout.bounds(), self.snapshot.bitmap_size()),
            *viewport,
        );
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && let Some(position) = cursor.position_over(layout.bounds())
            && let Some(endpoint) = self.endpoint_at(layout.bounds(), position)
        {
            shell.publish((self.on_endpoint)(endpoint));
            shell.capture_event();
        }
    }
}

#[allow(dead_code)]
fn selectable_pdf_page<'a, Message: 'a>(
    snapshot: &'a PdfSelectionSnapshot,
    raster: image::Handle,
    on_endpoint: fn(PdfSelectionEndpoint) -> Message,
) -> Element<'a, Message> {
    Element::new(SelectablePdfPage {
        snapshot,
        raster,
        on_endpoint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_text::{
        Attrs, Buffer, Color as CosmicColor, Family, FontSystem, LineIter, Metrics, Shaping,
        SwashCache, Wrap,
    };
    use iced::advanced::text::{
        Alignment, LineHeight, Paragraph as _, Shaping as IcedShaping, Span, Text,
        Wrapping as IcedWrapping,
    };
    use iced::{Font, Pixels, alignment};
    use shosai_core::document::Document;
    use shosai_core::epub::render::{ContentNode, TextSpan};
    use shosai_core::pdf::PdfDoc;
    use unicode_normalization::UnicodeNormalization;
    use unicode_segmentation::UnicodeSegmentation;

    type Paragraph = iced_tiny_skia::graphics::text::Paragraph;

    struct PreparedNativeText {
        fonts: FontSystem,
        buffer: Buffer,
        surface: SelectableTextSurface,
    }

    impl PreparedNativeText {
        fn rasterized_pixels(&mut self) -> usize {
            let mut pixels = 0;
            self.buffer.draw(
                &mut self.fonts,
                &mut SwashCache::new(),
                CosmicColor::rgb(0, 0, 0),
                |_, _, width, height, _| pixels += width as usize * height as usize,
            );
            pixels
        }
    }

    fn paragraph(spans: &[Span<'_, (), Font>], width: f32) -> Paragraph {
        Paragraph::with_spans(Text {
            content: spans,
            bounds: Size::new(width, 500.0),
            size: Pixels(18.0),
            line_height: LineHeight::default(),
            font: Font::default(),
            align_x: Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping: IcedShaping::Advanced,
            wrapping: IcedWrapping::WordOrGlyph,
        })
    }

    fn source_line_ranges(text: &str) -> Vec<Range<usize>> {
        LineIter::new(text).map(|(range, _)| range).collect()
    }

    fn quote_v1_whitespace(character: char) -> bool {
        matches!(
            character,
            '\u{0009}'..='\u{000d}'
                | '\u{0020}'
                | '\u{0085}'
                | '\u{00a0}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
        )
    }

    fn normalize_quote_v1(value: &str) -> String {
        let without_soft_hyphens = value.replace("\r\n", "\n").replace('\r', "\n");
        let normalized = without_soft_hyphens
            .chars()
            .filter(|character| *character != '\u{00ad}')
            .collect::<String>()
            .nfc()
            .collect::<String>();
        let mut result = String::new();
        let mut whitespace = false;
        for character in normalized.chars() {
            if quote_v1_whitespace(character) {
                whitespace = !result.is_empty();
            } else {
                if whitespace {
                    result.push(' ');
                    whitespace = false;
                }
                result.push(character);
            }
        }
        result
    }

    fn quote_v1_context(value: &str, prefix: bool) -> String {
        const LIMIT: usize = 32;
        let normalized = normalize_quote_v1(value);
        let graphemes = normalized.graphemes(true).collect::<Vec<_>>();
        let selected = if prefix {
            let mut scalars = 0;
            let start = graphemes
                .iter()
                .rposition(|grapheme| {
                    let next = scalars + grapheme.chars().count();
                    if next <= LIMIT {
                        scalars = next;
                        false
                    } else {
                        true
                    }
                })
                .map_or(0, |index| index + 1);
            &graphemes[start..]
        } else {
            let mut scalars = 0;
            let end = graphemes
                .iter()
                .position(|grapheme| {
                    let next = scalars + grapheme.chars().count();
                    if next <= LIMIT {
                        scalars = next;
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(graphemes.len());
            &graphemes[..end]
        };
        selected.concat()
    }

    fn paragraph_scalar_at(paragraph: &Paragraph, text: &str, point: Point) -> Option<usize> {
        let cursor = paragraph.buffer().hit(point.x, point.y)?;
        let line = source_line_ranges(text).get(cursor.line)?.clone();
        let byte = line.start.checked_add(cursor.index)?;
        if byte > line.end || !text.is_char_boundary(byte) {
            return None;
        }
        let scalar = text[..byte].chars().count();
        text.grapheme_indices(true)
            .map(|(byte, _)| text[..byte].chars().count())
            .chain(std::iter::once(text.chars().count()))
            .any(|boundary| boundary == scalar)
            .then_some(scalar)
    }

    fn prepare_selectable_native_text(text: &str, width: f32) -> Option<PreparedNativeText> {
        let mut fonts = crate::epub::text_shaping::font_system();
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(18.0, 24.0));
        buffer.set_size(&mut fonts, Some(width), None);
        buffer.set_wrap(&mut fonts, Wrap::WordOrGlyph);
        buffer.set_text(
            &mut fonts,
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let line_ranges = source_line_ranges(text);
        let mut rows = Vec::new();
        for run in buffer.layout_runs() {
            let line = &line_ranges[run.line_i];
            let line_text = &text[line.clone()];
            let mut zones = Vec::new();
            for glyph in run.glyphs {
                let cluster = &line_text[glyph.start..glyph.end];
                let graphemes = cluster.grapheme_indices(true).collect::<Vec<_>>();
                let grapheme_width = glyph.w / graphemes.len().max(1) as f32;
                for (position, (byte, grapheme)) in graphemes.iter().enumerate() {
                    let left = glyph.x + position as f32 * grapheme_width;
                    let before_byte = line.start + glyph.start + byte;
                    let after_byte = before_byte + grapheme.len();
                    let before = text[..before_byte].chars().count();
                    let after = text[..after_byte].chars().count();
                    let (left_endpoint, right_endpoint) = if glyph.level.is_rtl() {
                        (after, before)
                    } else {
                        (before, after)
                    };
                    let top = run.line_top + glyph.y;
                    let height = glyph.line_height_opt.unwrap_or(run.line_height);
                    zones.push(EndpointZone {
                        bounds: Rectangle::new(
                            Point::new(left, top),
                            Size::new(grapheme_width / 2.0, height),
                        ),
                        scalar: left_endpoint,
                    });
                    zones.push(EndpointZone {
                        bounds: Rectangle::new(
                            Point::new(left + grapheme_width / 2.0, top),
                            Size::new(grapheme_width / 2.0, height),
                        ),
                        scalar: right_endpoint,
                    });
                }
            }
            if let (Some(first), Some(last)) = (zones.first(), zones.last()) {
                rows.push(EndpointRow {
                    bounds: Rectangle::new(
                        Point::new(0.0, run.line_top),
                        Size::new(
                            run.line_w.max(last.bounds.x + last.bounds.width),
                            run.line_height.max(first.bounds.height),
                        ),
                    ),
                    zones,
                });
            }
        }
        let surface = SelectableTextSurface { rows };
        (surface.endpoint_count() <= MAX_ENDPOINTS_PER_SURFACE).then_some(PreparedNativeText {
            fonts,
            buffer,
            surface,
        })
    }

    fn pdf_with_content(content: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /CropBox [100 50 300 200] /Rotate 90 /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len() + 1
            ),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn generated_pdf() -> Vec<u8> {
        pdf_with_content("BT /F1 24 Tf 1 0 0 1 130 120 Tm (TARGET) Tj ET")
    }

    fn dense_pdf() -> Vec<u8> {
        let mut content = String::new();
        for line in 0..80 {
            content.push_str(&format!(
                "BT /F1 1 Tf 1 0 0 1 105 {} Tm ({}) Tj ET\n",
                51.0 + line as f32 * 0.9,
                "A".repeat(180)
            ));
        }
        pdf_with_content(&content)
    }

    fn text_span(text: impl Into<String>) -> TextSpan {
        TextSpan {
            text: text.into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }
    }

    fn content_text(node: &ContentNode) -> (&[TextSpan], String) {
        let ContentNode::Paragraph(spans, _) = node else {
            panic!("selection fixture must remain paragraph text");
        };
        (spans, spans.iter().map(|span| span.text.as_str()).collect())
    }

    #[test]
    fn selectable_pdf_widget_composes_letterboxing_raster_and_pdfium_coordinates() {
        let document = PdfDoc::from_bytes(generated_pdf()).unwrap();
        let snapshot = document.selection_snapshot(0, 2.0).unwrap();
        let rendered = document.render_page(0, 2.0).unwrap();
        assert_eq!(snapshot.bitmap_size(), (rendered.width, rendered.height));
        let raster = image::Handle::from_rgba(rendered.width, rendered.height, rendered.pixels);
        let target = snapshot.bitmap_bounds(0).unwrap();
        let bitmap_center = Point::new(
            (target.left + target.right) / 2.0,
            (target.top + target.bottom) / 2.0,
        );
        let widget_bounds = Rectangle::new(Point::new(40.0, 20.0), Size::new(500.0, 500.0));
        let content = image_content_bounds(widget_bounds, snapshot.bitmap_size());
        let widget_point = Point::new(
            content.x + bitmap_center.x * content.width / snapshot.bitmap_size().0 as f32,
            content.y + bitmap_center.y * content.height / snapshot.bitmap_size().1 as f32,
        );
        let mut widget = SelectablePdfPage {
            snapshot: &snapshot,
            raster,
            on_endpoint: std::convert::identity,
        };

        assert_eq!(snapshot.bitmap_size(), (300, 400));
        assert!(content.x > widget_bounds.x, "portrait page must letterbox");
        assert!(
            widget
                .endpoint_at(
                    widget_bounds,
                    Point::new(widget_bounds.x + 1.0, widget_bounds.y + 1.0)
                )
                .is_none(),
            "letterbox presses must not emit endpoints"
        );

        let node = layout::Node::new(widget_bounds.size()).move_to(widget_bounds.position());
        let layout = Layout::new(&node);
        let mut tree = Tree::empty();
        let mut clipboard = iced::advanced::clipboard::Null;
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);
        let renderer = ();
        Widget::<PdfSelectionEndpoint, iced::Theme, ()>::update(
            &mut widget,
            &mut tree,
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            layout,
            mouse::Cursor::Available(widget_point),
            &renderer,
            &mut clipboard,
            &mut shell,
            &widget_bounds,
        );
        assert!(shell.is_event_captured());
        drop(shell);
        let endpoint = messages[0];
        assert_eq!(endpoint.character, 0);
        assert!((130.0..150.0).contains(&endpoint.page_x));
        assert!((110.0..140.0).contains(&endpoint.page_y));

        let mut renderer = ();
        Widget::<PdfSelectionEndpoint, iced::Theme, ()>::draw(
            &widget,
            &tree,
            &mut renderer,
            &iced::Theme::Light,
            &renderer::Style::default(),
            layout,
            mouse::Cursor::Unavailable,
            &widget_bounds,
        );
    }

    #[test]
    fn quote_v1_golden_vectors_pin_normalization_and_context_direction() {
        assert_eq!(normalize_quote_v1("Cafe\u{301}"), "Café");
        assert_eq!(normalize_quote_v1(" a\r\n\t b\u{a0}c "), "a b c");
        assert_eq!(normalize_quote_v1("co\u{ad}operate"), "cooperate");
        assert_eq!(normalize_quote_v1("Case—A-B! ﬁ"), "Case—A-B! ﬁ");
        assert_ne!(normalize_quote_v1("Résumé"), normalize_quote_v1("résumé"));

        let asymmetric = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        assert_eq!(
            quote_v1_context(asymmetric, true),
            "456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"
        );
        assert_eq!(
            quote_v1_context(asymmetric, false),
            "0123456789ABCDEFGHIJKLMNOPQRSTUV"
        );
        assert_eq!(
            quote_v1_context(&format!("{}e\u{301}", "x".repeat(31)), true),
            format!("{}é", "x".repeat(31)),
            "NFC must run before the scalar context bound"
        );
        let emoji = "👩\u{200d}🔬";
        assert_eq!(
            quote_v1_context(&format!("{emoji}{}", "x".repeat(31)), true),
            "x".repeat(31),
            "prefix must omit an EGC crossed by its scalar boundary"
        );
        assert_eq!(
            quote_v1_context(&format!("{}{emoji}", "x".repeat(31)), false),
            "x".repeat(31),
            "suffix must omit an EGC crossed by its scalar boundary"
        );
    }

    #[test]
    fn iced_rich_text_converts_multiline_utf8_hits_to_chapter_scalars() {
        let text = "Aé\n👩\u{200d}🔬 סוף";
        let spans = [Span::<(), Font>::new("Aé\n"), Span::new("👩\u{200d}🔬 סוף")];
        let paragraph = paragraph(&spans, 300.0);
        let second_span = paragraph.span_bounds(1);
        let point = second_span.last().unwrap().center();
        let scalar = paragraph_scalar_at(&paragraph, text, point).unwrap();

        assert!(scalar >= "Aé\n".chars().count());
        assert_ne!(scalar, paragraph.hit_test(point).unwrap().cursor());
        assert!(
            text.grapheme_indices(true)
                .map(|(byte, _)| text[..byte].chars().count())
                .chain(std::iter::once(text.chars().count()))
                .any(|boundary| boundary == scalar)
        );
    }

    #[test]
    fn native_hit_zones_emit_grapheme_boundaries_for_ligatures_emoji_and_bidi() {
        let text = "office e\u{301} 👩\u{200d}🔬 שלום";
        let mut prepared = prepare_selectable_native_text(text, 500.0).unwrap();
        assert!(prepared.rasterized_pixels() > 0);
        let surface = &prepared.surface;
        let boundaries = text
            .grapheme_indices(true)
            .map(|(byte, _)| text[..byte].chars().count())
            .chain(std::iter::once(text.chars().count()))
            .collect::<Vec<_>>();

        assert!(!surface.rows.is_empty());
        let zone_scalars = surface
            .rows
            .iter()
            .flat_map(|row| &row.zones)
            .map(|zone| zone.scalar)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            boundaries
                .iter()
                .all(|boundary| zone_scalars.contains(boundary)),
            "every grapheme boundary must have a pointer hit zone"
        );
        assert!(
            surface
                .rows
                .iter()
                .flat_map(|row| &row.zones)
                .all(|zone| boundaries.contains(&zone.scalar))
        );
        assert!(
            surface
                .rows
                .iter()
                .flat_map(|row| &row.zones)
                .filter_map(|zone| surface.hit_test(zone.bounds.center()))
                .all(|scalar| boundaries.contains(&scalar))
        );
        let hebrew_start = text.find('ש').unwrap();
        let hebrew_scalar = text[..hebrew_start].chars().count();
        let hebrew_zones = surface
            .rows
            .iter()
            .flat_map(|row| &row.zones)
            .filter(|zone| zone.scalar >= hebrew_scalar)
            .collect::<Vec<_>>();
        assert!(
            hebrew_zones
                .windows(2)
                .any(|pair| pair[0].bounds.x < pair[1].bounds.x && pair[0].scalar > pair[1].scalar),
            "RTL visual movement must retain decreasing logical endpoints"
        );
    }

    #[test]
    fn epub_selection_rebases_real_paginated_fragment_hits_to_chapter_offsets() {
        let nodes = vec![
            ContentNode::Paragraph(
                vec![text_span("Start é "), text_span("first span ".repeat(35))],
                Default::default(),
            ),
            ContentNode::Paragraph(
                vec![text_span("Second block 👩\u{200d}🔬 "), text_span("ending")],
                Default::default(),
            ),
        ];
        let chapter_text = shosai_core::search::extract_text_from_nodes(&nodes);
        let pages = crate::epub::paginate_epub_chapter(
            &nodes,
            None,
            16.0,
            1.6,
            crate::epub::LayoutSize::new(220.0, 130.0),
        );
        assert!(pages.len() > 1);
        let first = pages.first().unwrap().first().unwrap();
        let last = pages.last().unwrap().last().unwrap();
        let first_node_scalars = content_text(&nodes[0]).1.chars().count();
        assert_eq!(first.text_offset, 0);
        assert_eq!(last.text_offset, first_node_scalars + 1);
        assert!(
            !std::ptr::eq(first, last),
            "endpoints must occupy different page fragments"
        );
        let (first_spans, first_text) = content_text(&first.node);
        let (last_spans, last_text) = content_text(&last.node);
        let first_iced = first_spans
            .iter()
            .map(|span| Span::<(), Font>::new(span.text.as_str()))
            .collect::<Vec<_>>();
        let last_iced = last_spans
            .iter()
            .map(|span| Span::<(), Font>::new(span.text.as_str()))
            .collect::<Vec<_>>();
        let first_paragraph = paragraph(&first_iced, 220.0);
        let last_paragraph = paragraph(&last_iced, 220.0);
        let first_local =
            paragraph_scalar_at(&first_paragraph, &first_text, Point::new(0.0, 0.0)).unwrap();
        let last_local = paragraph_scalar_at(
            &last_paragraph,
            &last_text,
            Point::new(last_paragraph.bounds().width - 1.0, 0.0),
        )
        .unwrap();
        assert_eq!(first_local, 0);
        assert_eq!(last_text, "Second block 👩\u{200d}🔬 ending");
        assert_eq!(last_local, last_text.chars().count());
        let start = LogicalEndpoint {
            spine: 3,
            scalar: first.text_offset + first_local,
        };
        let end = LogicalEndpoint {
            spine: 3,
            scalar: last.text_offset + last_local,
        };
        let range = logical_range(start, end).unwrap();

        assert!(range.start < range.end);
        assert!(range.end <= chapter_text.chars().count());
        assert_eq!(range, 0..chapter_text.chars().count() - 1);
        assert_eq!(logical_range(end, start), Some(range.clone()));
        assert_eq!(
            logical_range(
                start,
                LogicalEndpoint {
                    spine: 4,
                    scalar: end.scalar,
                }
            ),
            None
        );
    }

    #[test]
    fn actual_selectable_surfaces_obey_document_geometry_budget() {
        let prepared =
            prepare_selectable_native_text(&"selection ".repeat(3_000), 1_000.0).unwrap();
        let surface = &prepared.surface;
        let pdf = PdfDoc::from_bytes(dense_pdf()).unwrap();
        let pdf = pdf.selection_snapshot(0, 1.0).unwrap();
        let mut budget = SelectionGeometryBudget::default();

        assert!(surface.endpoint_count() <= MAX_ENDPOINTS_PER_SURFACE);
        assert!(surface.retained_bytes() < MAX_RETAINED_BYTES_PER_DOCUMENT);
        assert!(
            pdf.endpoint_count() > 10_000,
            "PDF fixture must remain dense"
        );
        assert!(pdf.retained_bytes() < MAX_RETAINED_BYTES_PER_DOCUMENT);
        assert!(
            prepare_selectable_native_text(&"selection ".repeat(4_000), 1_000.0).is_none(),
            "surface construction must reject rather than truncate excess endpoints"
        );
        assert!(budget.admit(surface.endpoint_count(), surface.retained_bytes()));
        assert!(budget.admit(pdf.endpoint_count(), pdf.retained_bytes()));
        while budget.admit(pdf.endpoint_count(), pdf.retained_bytes()) {}
        assert!(budget.retained_bytes <= MAX_RETAINED_BYTES_PER_DOCUMENT);
        assert!(!budget.admit(pdf.endpoint_count(), pdf.retained_bytes()));
    }

    #[test]
    #[ignore = "manual latency measurement; wall-clock results are not a portable unit assertion"]
    fn measure_actual_selection_hot_path() {
        const SAMPLES: usize = 10_000;
        let prepared =
            prepare_selectable_native_text(&"selection ".repeat(3_000), 1_000.0).unwrap();
        let surface = &prepared.surface;
        let zones = surface
            .rows
            .iter()
            .flat_map(|row| &row.zones)
            .collect::<Vec<_>>();
        let mut samples = (0..SAMPLES)
            .map(|sample| {
                let index = sample * (zones.len() - 1) / (SAMPLES - 1);
                let started = std::time::Instant::now();
                assert_eq!(
                    surface.hit_test(zones[index].bounds.center()),
                    Some(zones[index].scalar)
                );
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        eprintln!(
            "RFD 6 actual geometry: endpoints={}, retained={} bytes, p50={:?}, p95={:?}, max={:?}",
            surface.endpoint_count(),
            surface.retained_bytes(),
            samples[SAMPLES / 2],
            samples[SAMPLES * 95 / 100],
            samples[SAMPLES - 1]
        );

        let pdf = PdfDoc::from_bytes(dense_pdf()).unwrap();
        let pdf = pdf.selection_snapshot(0, 1.0).unwrap();
        let pdf_bounds = (0..pdf.endpoint_count())
            .map(|endpoint| pdf.bitmap_bounds_at(endpoint).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pdf_bounds.len(), pdf.endpoint_count());
        let mut pdf_samples = (0..SAMPLES)
            .map(|sample| {
                let index = sample * (pdf_bounds.len() - 1) / (SAMPLES - 1);
                let bounds = pdf_bounds[index];
                let started = std::time::Instant::now();
                assert!(
                    pdf.hit_test(
                        (bounds.left + bounds.right) / 2.0,
                        (bounds.top + bounds.bottom) / 2.0,
                    )
                    .is_some()
                );
                started.elapsed()
            })
            .collect::<Vec<_>>();
        pdf_samples.sort_unstable();
        eprintln!(
            "RFD 6 PDF geometry: endpoints={}, retained={} bytes, p50={:?}, p95={:?}, max={:?}",
            pdf.endpoint_count(),
            pdf.retained_bytes(),
            pdf_samples[SAMPLES / 2],
            pdf_samples[SAMPLES * 95 / 100],
            pdf_samples[SAMPLES - 1]
        );
    }
}
