use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pdfium_render::prelude::*;
use unicode_bidi::BidiClass;
use unicode_segmentation::UnicodeSegmentation;

use crate::document::{Document, DocumentMetadata, RenderedPage};

/// Maximum number of character endpoints retained for one selectable PDF page.
pub const PDF_SELECTION_MAX_ENDPOINTS: usize = 65_536;
/// Maximum owned hit-test geometry retained for one selectable PDF page.
pub const PDF_SELECTION_MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PDF_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_PDF_BITMAP_DIMENSION: u32 = 16_384;
pub const MAX_PDF_BITMAP_PIXELS: u64 = 40_000_000;
pub const MAX_PDF_PAGE_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PDF_METADATA_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfSelectionRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl PdfSelectionRect {
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfSelectionEndpoint {
    /// PDFium character whose glyph bounds own this half-zone.
    pub underlying_character: usize,
    /// Caret boundary represented by this half-zone.
    pub character: usize,
    pub page_x: f32,
    pub page_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfSelectionCaret {
    pub character: usize,
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfSelectionLine {
    pub carets: Vec<PdfSelectionCaret>,
}

#[derive(Clone, Debug)]
struct PdfSelectionZone {
    bounds: PdfSelectionRect,
    page_bounds: (f32, f32, f32, f32),
    character: usize,
    endpoint: PdfSelectionEndpoint,
}

#[derive(Clone, Copy, Debug)]
struct PdfCharacterGeometry {
    bounds: PdfSelectionRect,
    page_bounds: (f32, f32, f32, f32),
    character: usize,
    page_x: f32,
    page_y: f32,
    orientation: Option<(bool, bool)>,
    direction: Option<PdfTextDirection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PdfTextDirection {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Debug)]
struct PdfSelectionRow {
    bounds: PdfSelectionRect,
    zones: Vec<PdfSelectionZone>,
}

/// PDFium-independent hit-test data for one rendered page.
#[derive(Clone, Debug)]
pub struct PdfSelectionSnapshot {
    bitmap_width: u32,
    bitmap_height: u32,
    text: String,
    rows: Vec<PdfSelectionRow>,
}

impl PdfSelectionSnapshot {
    pub fn bitmap_size(&self) -> (u32, u32) {
        (self.bitmap_width, self.bitmap_height)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn hit_test(&self, bitmap_x: f32, bitmap_y: f32) -> Option<PdfSelectionEndpoint> {
        self.rows
            .iter()
            .filter(|row| row.bounds.contains(bitmap_x, bitmap_y))
            .find_map(|row| {
                row.zones
                    .iter()
                    .find(|zone| zone.bounds.contains(bitmap_x, bitmap_y))
                    .map(|zone| zone.endpoint)
            })
    }

    pub fn bitmap_bounds(&self, character: usize) -> Option<PdfSelectionRect> {
        self.rows
            .iter()
            .flat_map(|row| &row.zones)
            .find(|zone| zone.endpoint.character == character)
            .map(|zone| zone.bounds)
    }

    pub fn bitmap_bounds_at(&self, endpoint: usize) -> Option<PdfSelectionRect> {
        self.rows
            .iter()
            .flat_map(|row| &row.zones)
            .nth(endpoint)
            .map(|zone| zone.bounds)
    }

    pub fn endpoint_count(&self) -> usize {
        self.rows.iter().map(|row| row.zones.len()).sum()
    }

    /// Return owned endpoint geometry that can be retained by a frontend and
    /// hit-tested without re-entering PDFium.
    pub fn endpoints(&self) -> Vec<(PdfSelectionRect, PdfSelectionEndpoint)> {
        self.rows
            .iter()
            .flat_map(|row| &row.zones)
            .map(|zone| (zone.bounds, zone.endpoint))
            .collect()
    }

    /// Visual lines and their retained caret positions, as classified by the
    /// PDF extractor. Consumers must not reconstruct lines from glyph bounds.
    pub fn visual_lines(&self) -> Vec<PdfSelectionLine> {
        self.rows
            .iter()
            .map(|row| {
                let mut carets = row
                    .zones
                    .iter()
                    .map(|zone| PdfSelectionCaret {
                        character: zone.endpoint.character,
                        x: (zone.bounds.left + zone.bounds.right) / 2.0,
                        top: row.bounds.top,
                        bottom: row.bounds.bottom,
                    })
                    .collect::<Vec<_>>();
                carets.sort_by(|left, right| left.x.total_cmp(&right.x));
                carets.dedup_by(|left, right| left.character == right.character);
                PdfSelectionLine { carets }
            })
            .collect()
    }

    /// Return PDF page-coordinate rectangles for a durable character range.
    pub fn page_rectangles(&self, start: usize, end: usize) -> Vec<(usize, (f32, f32, f32, f32))> {
        let mut rectangles = self
            .rows
            .iter()
            .flat_map(|row| &row.zones)
            .filter(|zone| start <= zone.character && zone.character < end)
            .map(|zone| (zone.character, zone.page_bounds))
            .collect::<Vec<_>>();
        // Each PDFium character owns two hit zones. Keep one durable rectangle
        // per original character regardless of grapheme-snapped caret offsets.
        rectangles.dedup_by_key(|(character, _)| *character);
        rectangles
    }

    pub fn retained_bytes(&self) -> usize {
        self.text.capacity()
            + self.rows.capacity() * std::mem::size_of::<PdfSelectionRow>()
            + self
                .rows
                .iter()
                .map(|row| row.zones.capacity() * std::mem::size_of::<PdfSelectionZone>())
                .sum::<usize>()
    }
}

/// Create a short-lived Pdfium instance.
///
/// `pdfium-render`'s `thread_safe` feature serializes all PDFium access behind a
/// global mutex. The lock is acquired on `FPDF_InitLibrary` (when a `Pdfium` is
/// created) and released on `FPDF_DestroyLibrary` (when it is dropped). Creating
/// a `Pdfium`, doing work, and dropping it promptly is the intended usage pattern
/// — it keeps the lock held only as long as needed and allows other threads to
/// proceed in between.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct PdfBackendUnavailable(pub(crate) String);

pub(crate) fn is_backend_unavailable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<PdfBackendUnavailable>().is_some()
}

fn create_pdfium() -> Result<Pdfium> {
    let library = std::env::current_exe()
        .ok()
        .and_then(|executable| bundled_pdfium_path(&executable))
        .filter(|path| path.is_file())
        .or_else(configured_pdfium_path);

    let bindings = match library {
        Some(path) => Pdfium::bind_to_library(&path)
            .with_context(|| format!("failed to load PDFium library at {}", path.display())),
        None => Pdfium::bind_to_system_library().context("failed to load PDFium system library"),
    }
    .map_err(|error| {
        PdfBackendUnavailable(format!(
            "{error}. Install a Shosai package containing PDFium, or ensure \
             pdfium-binaries is available through the system library path"
        ))
    })?;

    Ok(Pdfium::new(bindings))
}

fn configured_pdfium_path() -> Option<PathBuf> {
    std::env::var_os("SHOSAI_PDFIUM_LIBRARY")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn bundled_pdfium_path(executable: &Path) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;

    #[cfg(target_os = "macos")]
    {
        let contents_dir = executable_dir.parent()?;
        Some(contents_dir.join("Frameworks/libpdfium.dylib"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let adjacent_library = executable_dir.join("lib/libpdfium.so");
        if adjacent_library.is_file() {
            return Some(adjacent_library);
        }
        let package_dir = executable_dir.parent()?;
        Some(package_dir.join("lib/libpdfium.so"))
    }
}

#[cfg(test)]
mod tests {
    use crate::document::Document;

    use super::{
        BoundedPageTextError, PdfDoc, PdfSelectionEndpoint, PdfSelectionRect, PdfSelectionZone,
        bundled_pdfium_path, grapheme_boundary_for_character, grapheme_ranges, pdf_selection_rows,
        read_pdf_file_with_limit, validate_pdf_bitmap_size, validate_pdf_preflight,
        validate_pdf_selection_endpoint_count,
    };
    use std::cell::Cell;
    use std::fs::File;
    use std::path::{Path, PathBuf};

    fn selectable_pdf(text: &str) -> Vec<u8> {
        selectable_pdf_content(&format!("BT /F1 24 Tf 1 0 0 1 130 120 Tm ({text}) Tj ET"))
    }

    fn selectable_pdf_content(content: &str) -> Vec<u8> {
        selectable_pdf_content_with_rotation(content, 90)
    }

    fn selectable_pdf_content_with_rotation(content: &str, rotation: u16) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /CropBox [100 50 300 200] /Rotate {rotation} /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            ),
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

    #[test]
    fn render_honors_preexisting_cancellation() {
        let document = PdfDoc::from_bytes(selectable_pdf("cancel")).unwrap();

        let error = document
            .render_page_with_highlights_cancellable(0, 1.0, &[], &|| true)
            .unwrap_err();

        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn bounded_file_read_uses_open_descriptor_after_path_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.pdf");
        let replacement = directory.path().join("replacement.pdf");
        std::fs::write(&path, b"original").unwrap();
        std::fs::write(&replacement, vec![b'x'; 32]).unwrap();

        let file = File::open(&path).unwrap();
        std::fs::rename(&replacement, &path).unwrap();

        let data = read_pdf_file_with_limit(file, &path, 8).unwrap();
        assert_eq!(data, b"original");
        assert_eq!(std::fs::read(&path).unwrap(), vec![b'x'; 32]);
    }

    #[test]
    fn fixed_snapshot_rejects_growth_and_truncation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.pdf");
        std::fs::write(&path, b"12345").unwrap();

        assert!(super::read_pdf_snapshot(File::open(&path).unwrap(), &path, 4, None).is_err());
        assert!(super::read_pdf_snapshot(File::open(&path).unwrap(), &path, 6, None).is_err());
    }

    #[test]
    fn bundled_pdfium_is_resolved_relative_to_executable() {
        #[cfg(target_os = "macos")]
        let expected =
            PathBuf::from("/Applications/Shosai.app/Contents/Frameworks/libpdfium.dylib");
        #[cfg(target_os = "macos")]
        let executable = Path::new("/Applications/Shosai.app/Contents/MacOS/Shosai");

        #[cfg(not(target_os = "macos"))]
        let expected = PathBuf::from("/opt/shosai/lib/libpdfium.so");
        #[cfg(not(target_os = "macos"))]
        let executable = Path::new("/opt/shosai/bin/shosai");

        assert_eq!(bundled_pdfium_path(executable), Some(expected));

        #[cfg(not(target_os = "macos"))]
        {
            let directory = tempfile::tempdir().unwrap();
            let library_dir = directory.path().join("lib");
            std::fs::create_dir(&library_dir).unwrap();
            let expected = library_dir.join("libpdfium.so");
            std::fs::write(&expected, []).unwrap();
            let executable = directory.path().join("shosai_flutter");

            assert_eq!(bundled_pdfium_path(&executable), Some(expected));
        }
    }

    #[test]
    fn owned_snapshot_emits_a_stable_endpoint_on_a_cropped_rotated_page() {
        let document = PdfDoc::from_bytes(selectable_pdf("TARGET")).unwrap();
        let snapshot = document.selection_snapshot(0, 1.0).unwrap();
        let bounds = snapshot.bitmap_bounds(0).unwrap();
        let endpoint = snapshot
            .hit_test(
                (bounds.left + bounds.right) / 2.0,
                (bounds.top + bounds.bottom) / 2.0,
            )
            .unwrap();

        assert_eq!(snapshot.bitmap_size(), (150, 200));
        assert_eq!(snapshot.endpoint_count(), 12);
        assert_eq!(endpoint.character, 0);
        assert!((130.0..150.0).contains(&endpoint.page_x));
        assert!((110.0..140.0).contains(&endpoint.page_y));
    }

    #[test]
    fn selection_snapshot_exposes_both_half_open_caret_boundaries() {
        let document = PdfDoc::from_bytes(selectable_pdf("AB")).unwrap();
        let snapshot = document.selection_snapshot(0, 1.0).unwrap();
        let endpoints = snapshot.endpoints();

        assert!(
            endpoints
                .iter()
                .any(|(_, endpoint)| endpoint.character == 0)
        );
        assert!(
            endpoints
                .iter()
                .any(|(_, endpoint)| endpoint.character == 1)
        );
        assert!(
            endpoints
                .iter()
                .any(|(_, endpoint)| endpoint.character == 2)
        );
        assert_eq!(snapshot.page_rectangles(0, 1).len(), 1);
        assert_eq!(snapshot.page_rectangles(1, 2).len(), 1);
        assert_eq!(snapshot.page_rectangles(2, 1), Vec::new());
    }

    #[test]
    fn decomposed_accent_has_no_interior_caret_boundary() {
        let ranges = grapheme_ranges("e\u{301}x");
        assert_eq!(ranges, vec![0..2, 2..3]);
        assert_eq!(grapheme_boundary_for_character(&ranges, 1, false), Some(0));
        assert_eq!(grapheme_boundary_for_character(&ranges, 1, true), Some(2));
    }

    #[test]
    fn overlapping_mixed_height_pdf_glyphs_share_a_visual_line() {
        let zone = |character, top, bottom| PdfSelectionZone {
            bounds: PdfSelectionRect {
                left: character as f32,
                top,
                right: character as f32 + 1.0,
                bottom,
            },
            page_bounds: (0.0, 0.0, 1.0, 1.0),
            character,
            endpoint: PdfSelectionEndpoint {
                underlying_character: character,
                character,
                page_x: 0.0,
                page_y: 0.0,
            },
        };
        let rows = pdf_selection_rows(vec![zone(0, 8.0, 12.0), zone(1, 9.0, 30.0)]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].zones.len(), 2);
    }

    #[test]
    fn selection_half_zones_keep_their_underlying_character_across_whitespace() {
        let document = PdfDoc::from_bytes(selectable_pdf("A B")).unwrap();
        let snapshot = document.selection_snapshot(0, 1.0).unwrap();
        let a: Vec<_> = snapshot
            .endpoints()
            .into_iter()
            .filter(|(_, endpoint)| endpoint.underlying_character == 0)
            .collect();

        assert_eq!(snapshot.text(), "A B");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].1.character, 0);
        assert_eq!(a[1].1.character, 1);
        assert_eq!(snapshot.page_rectangles(0, 1).len(), 1);
        assert_eq!(snapshot.page_rectangles(1, 2).len(), 1);
    }

    #[test]
    fn final_character_direction_uses_only_its_own_pdf_line() {
        let document = PdfDoc::from_bytes(selectable_pdf_content(
            "BT /F1 24 Tf 1 0 0 1 130 130 Tm (AB) Tj 1 0 0 1 110 80 Tm (CD) Tj ET",
        ))
        .unwrap();
        let snapshot = document.selection_snapshot(0, 1.0).unwrap();
        let indexes: Vec<_> = snapshot
            .text()
            .chars()
            .enumerate()
            .filter_map(|(index, character)| "ABCD".contains(character).then_some(index))
            .collect();
        assert_eq!(indexes.len(), 4);

        let endpoints = snapshot.endpoints();
        for (first, final_character) in [(indexes[0], indexes[1]), (indexes[2], indexes[3])] {
            let direction = |character| {
                let halves: Vec<_> = endpoints
                    .iter()
                    .filter(|(_, endpoint)| endpoint.underlying_character == character)
                    .collect();
                assert_eq!(halves.len(), 2);
                let before = halves
                    .iter()
                    .find(|(_, endpoint)| endpoint.character == character)
                    .unwrap()
                    .0;
                let after = halves
                    .iter()
                    .find(|(_, endpoint)| endpoint.character == character + 1)
                    .unwrap()
                    .0;
                (
                    after.left + after.right - before.left - before.right,
                    after.top + after.bottom - before.top - before.bottom,
                )
            };
            let expected = direction(first);
            let actual = direction(final_character);
            assert!(expected.0 * actual.0 + expected.1 * actual.1 > 0.0);
        }
    }

    #[test]
    fn vertical_text_matrix_preserves_first_and_final_caret_identity() {
        let document = PdfDoc::from_bytes(selectable_pdf_content(
            "BT /F1 24 Tf 0 1 -1 0 180 80 Tm (ABCD) Tj ET",
        ))
        .unwrap();
        let snapshot = document.selection_snapshot(0, 1.0).unwrap();
        let indexes: Vec<_> = snapshot
            .text()
            .chars()
            .enumerate()
            .filter_map(|(index, character)| "ABCD".contains(character).then_some(index))
            .collect();
        assert_eq!(indexes.len(), 4);

        let endpoints = snapshot.endpoints();
        for (character, leading, trailing) in [
            (indexes[0], indexes[0], indexes[0] + 1),
            (*indexes.last().unwrap(), indexes[3], indexes[3] + 1),
        ] {
            let halves: Vec<_> = endpoints
                .iter()
                .filter(|(_, endpoint)| endpoint.underlying_character == character)
                .collect();
            assert_eq!(halves.len(), 2);
            assert_eq!(halves[0].1.character, leading);
            assert_eq!(halves[1].1.character, trailing);
        }

        let direction = |character| {
            let halves: Vec<_> = endpoints
                .iter()
                .filter(|(_, endpoint)| endpoint.underlying_character == character)
                .collect();
            let leading = halves
                .iter()
                .find(|(_, endpoint)| endpoint.character == character)
                .unwrap()
                .0;
            let trailing = halves
                .iter()
                .find(|(_, endpoint)| endpoint.character == character + 1)
                .unwrap()
                .0;
            (
                trailing.left + trailing.right - leading.left - leading.right,
                trailing.top + trailing.bottom - leading.top - leading.bottom,
            )
        };
        let first = direction(indexes[0]);
        let final_character = direction(indexes[3]);
        assert!(first.0 * final_character.0 + first.1 * final_character.1 > 0.0);
    }

    #[test]
    fn isolated_glyph_uses_its_transformed_baseline_for_caret_halves() {
        let centers = |matrix: &str, font_size, page_rotation| {
            let document = PdfDoc::from_bytes(selectable_pdf_content_with_rotation(
                &format!("BT /F1 {font_size} Tf {matrix} 180 80 Tm (A) Tj ET"),
                page_rotation,
            ))
            .unwrap();
            let snapshot = document.selection_snapshot(0, 1.0).unwrap();
            let character = snapshot.text().find('A').unwrap();
            let halves: Vec<_> = snapshot
                .endpoints()
                .into_iter()
                .filter(|(_, endpoint)| endpoint.underlying_character == character)
                .collect();
            assert_eq!(halves.len(), 2);
            let center = |rect: super::PdfSelectionRect| {
                (
                    (rect.left + rect.right) / 2.0,
                    (rect.top + rect.bottom) / 2.0,
                )
            };
            let leading = halves
                .iter()
                .find(|(_, endpoint)| endpoint.character == character)
                .map(|(rect, _)| center(*rect))
                .unwrap();
            let trailing = halves
                .iter()
                .find(|(_, endpoint)| endpoint.character == character + 1)
                .map(|(rect, _)| center(*rect))
                .unwrap();
            (leading, trailing)
        };

        let (leading, trailing) = centers("0 1 -1 0", 24, 0);
        assert!(leading.1 > trailing.1, "90° baseline points up in pixels");
        let (leading, trailing) = centers("0 -1 1 0", 24, 0);
        assert!(
            leading.1 < trailing.1,
            "270° baseline points down in pixels"
        );
        let (leading, trailing) = centers("1 0 0 1", 24, 90);
        assert!(
            leading.1 < trailing.1,
            "page rotation transforms a horizontal baseline into pixel-space vertical"
        );
        let (leading, trailing) = centers("0 .01 -.01 0", 2400, 0);
        assert!(
            leading.1 > trailing.1,
            "a subpixel text-space baseline still retains its physical direction"
        );
    }

    #[test]
    fn logical_run_progression_overrides_upright_and_rotated_matrix_direction() {
        let direction = |content: &str, vertical| {
            let document =
                PdfDoc::from_bytes(selectable_pdf_content_with_rotation(content, 0)).unwrap();
            let snapshot = document.selection_snapshot(0, 1.0).unwrap();
            let character = snapshot.text().find('A').unwrap();
            let endpoints = snapshot.endpoints();
            let center = |caret| {
                let rect = endpoints
                    .iter()
                    .find(|(_, endpoint)| {
                        endpoint.underlying_character == character && endpoint.character == caret
                    })
                    .unwrap()
                    .0;
                if vertical {
                    (rect.top + rect.bottom) / 2.0
                } else {
                    (rect.left + rect.right) / 2.0
                }
            };
            (center(character), center(character + 1))
        };

        let (leading, trailing) =
            direction("BT /F1 24 Tf -30 Tc 1 0 0 1 180 80 Tm (ABC) Tj ET", false);
        assert!(leading > trailing, "logical progression moves left");
        let (leading, trailing) =
            direction("BT /F1 24 Tf -30 Tc 0 1 -1 0 180 120 Tm (ABC) Tj ET", true);
        assert!(leading < trailing, "rotated logical progression moves down");
    }

    #[test]
    fn expanded_ligature_subdivisions_reach_the_full_caret_range() {
        let bounds = super::PdfSelectionRect {
            left: 0.0,
            top: 0.0,
            right: 40.0,
            bottom: 10.0,
        };
        let geometry = |character| super::PdfCharacterGeometry {
            bounds,
            page_bounds: (0.0, 0.0, 10.0, 10.0),
            character,
            page_x: 5.0,
            page_y: 5.0,
            orientation: Some((false, true)),
            direction: Some(super::PdfTextDirection::LeftToRight),
        };
        let glyph = [geometry(0), geometry(1)];
        assert!(super::same_pdf_glyph(glyph[0], glyph[1]));
        let mut zones = Vec::new();
        super::append_pdf_glyph_zones(&mut zones, &glyph, false, true);
        let snapshot = super::PdfSelectionSnapshot {
            bitmap_width: 40,
            bitmap_height: 10,
            text: "fi".into(),
            rows: super::pdf_selection_rows(zones),
        };

        assert_eq!(snapshot.hit_test(5.0, 5.0).unwrap().character, 0);
        assert_eq!(snapshot.hit_test(35.0, 5.0).unwrap().character, 2);
    }

    #[test]
    fn adjacent_bidi_runs_do_not_share_neighbor_direction() {
        fn rotate_geometry(mut item: super::PdfCharacterGeometry) -> super::PdfCharacterGeometry {
            item.bounds = super::PdfSelectionRect {
                left: 0.0,
                top: item.page_x - 4.0,
                right: 10.0,
                bottom: item.page_x + 4.0,
            };
            item.page_y = item.page_x;
            item.page_x = 5.0;
            item.orientation = item.orientation.map(|(_, positive)| (true, positive));
            item
        }

        let geometry = |character, center, direction| super::PdfCharacterGeometry {
            bounds: super::PdfSelectionRect {
                left: center - 4.0,
                top: 0.0,
                right: center + 4.0,
                bottom: 10.0,
            },
            page_bounds: (center - 4.0, 0.0, center + 4.0, 10.0),
            character,
            page_x: center,
            page_y: 5.0,
            orientation: Some((
                false,
                direction != Some(super::PdfTextDirection::RightToLeft),
            )),
            direction,
        };
        let rtl = super::PdfTextDirection::RightToLeft;
        let ltr = super::PdfTextDirection::LeftToRight;
        let rtl_previous = geometry(1, 40.0, Some(rtl));
        let rtl_final = geometry(2, 30.0, Some(rtl));
        let ltr_first = geometry(3, 60.0, Some(ltr));
        let ltr_next = geometry(4, 70.0, Some(ltr));
        let neutral = geometry(3, 60.0, None);

        assert_eq!(
            super::pdf_character_direction(
                Some(rtl_previous),
                rtl_final,
                rtl_final,
                Some(ltr_first),
            ),
            (false, false)
        );
        assert_eq!(
            super::pdf_character_direction(Some(rtl_final), ltr_first, ltr_first, Some(ltr_next),),
            (false, true)
        );
        assert_eq!(
            super::pdf_character_direction(Some(rtl_previous), rtl_final, rtl_final, Some(neutral),),
            (false, false)
        );

        assert_eq!(super::pdf_text_direction('١'), Some(ltr));
        assert_eq!(
            super::pdf_logical_orientation((false, true), super::pdf_text_direction('١')),
            (false, true)
        );
        assert_eq!(
            super::pdf_logical_orientation((true, false), super::pdf_text_direction('١')),
            (true, false)
        );

        let mut neutral_run = [
            geometry(10, 40.0, Some(rtl)),
            geometry(11, 30.0, None),
            geometry(12, 20.0, Some(rtl)),
        ];
        super::resolve_pdf_neutral_directions(&mut neutral_run);
        assert_eq!(neutral_run[1].direction, Some(rtl));
        assert_eq!(
            super::pdf_character_direction(
                Some(neutral_run[0]),
                neutral_run[1],
                neutral_run[1],
                Some(neutral_run[2]),
            ),
            (false, false)
        );
        let rotated_neutral_run = neutral_run.map(rotate_geometry);
        assert_eq!(
            super::pdf_character_direction(
                Some(rotated_neutral_run[0]),
                rotated_neutral_run[1],
                rotated_neutral_run[1],
                Some(rotated_neutral_run[2]),
            ),
            (true, false)
        );

        let mut neutral_with_edge_gap = [
            geometry(20, 40.0, Some(rtl)),
            geometry(21, 30.0, None),
            geometry(23, 20.0, Some(rtl)),
        ];
        super::resolve_pdf_neutral_directions(&mut neutral_with_edge_gap);
        assert_eq!(neutral_with_edge_gap[1].direction, None);
        assert_eq!(
            super::pdf_character_direction(
                Some(neutral_with_edge_gap[0]),
                neutral_with_edge_gap[0],
                neutral_with_edge_gap[0],
                Some(neutral_with_edge_gap[1]),
            ),
            (false, false)
        );

        let mut neutral_with_inner_gap = [
            geometry(30, 40.0, Some(rtl)),
            geometry(31, 30.0, None),
            geometry(33, 20.0, None),
            geometry(34, 10.0, Some(rtl)),
        ];
        super::resolve_pdf_neutral_directions(&mut neutral_with_inner_gap);
        assert_eq!(neutral_with_inner_gap[1].direction, None);
        assert_eq!(neutral_with_inner_gap[2].direction, None);

        let mut displaced_neutral_run = [
            geometry(40, 40.0, Some(rtl)),
            geometry(41, 30.0, None),
            geometry(42, 20.0, Some(rtl)),
        ];
        displaced_neutral_run[1].bounds.top = 20.0;
        displaced_neutral_run[1].bounds.bottom = 30.0;
        super::resolve_pdf_neutral_directions(&mut displaced_neutral_run);
        assert_eq!(
            super::pdf_character_direction(
                Some(displaced_neutral_run[0]),
                displaced_neutral_run[1],
                displaced_neutral_run[1],
                Some(displaced_neutral_run[2]),
            ),
            (false, false)
        );
        let mut rotated_displaced_neutral_run = displaced_neutral_run.map(rotate_geometry);
        rotated_displaced_neutral_run[1].bounds.left = 20.0;
        rotated_displaced_neutral_run[1].bounds.right = 30.0;
        assert_eq!(
            super::pdf_character_direction(
                Some(rotated_displaced_neutral_run[0]),
                rotated_displaced_neutral_run[1],
                rotated_displaced_neutral_run[1],
                Some(rotated_displaced_neutral_run[2]),
            ),
            (true, false)
        );

        let rtl_previous = rotate_geometry(rtl_previous);
        let rtl_final = rotate_geometry(rtl_final);
        let ltr_first = rotate_geometry(ltr_first);
        let ltr_next = rotate_geometry(ltr_next);
        let neutral = rotate_geometry(neutral);
        assert_eq!(
            super::pdf_character_direction(
                Some(rtl_previous),
                rtl_final,
                rtl_final,
                Some(ltr_first),
            ),
            (true, false)
        );
        assert_eq!(
            super::pdf_character_direction(Some(rtl_final), ltr_first, ltr_first, Some(ltr_next),),
            (true, true)
        );
        assert_eq!(
            super::pdf_character_direction(Some(rtl_previous), rtl_final, rtl_final, Some(neutral),),
            (true, false)
        );
    }

    #[test]
    fn direction_construction_cancels_many_single_character_rows() {
        let mut content = "BT /F1 1 Tf ".to_owned();
        for row in 0..200 {
            content.push_str(&format!("1 0 0 1 110 {} Tm (A) Tj ", 51 + row));
        }
        content.push_str("ET");
        let document = PdfDoc::from_bytes(selectable_pdf_content(&content)).unwrap();
        let character_count = document.page_text(0).unwrap().chars().count();
        let calls = Cell::new(0);

        let error = document
            .selection_snapshot_cancellable(0, 1.0, &|| {
                calls.set(calls.get() + 1);
                calls.get() > character_count + 2
            })
            .unwrap_err();

        assert_eq!(calls.get(), character_count + 3);
        assert_eq!(error.to_string(), "import cancelled");
    }

    #[test]
    fn actual_emitted_endpoint_limit_is_a_typed_resource_error() {
        let line = "A".repeat(164);
        let mut content = "BT /F1 .5 Tf ".to_owned();
        for row in 0..200 {
            content.push_str(&format!(
                "1 0 0 1 110 {} Tm ({line}) Tj ",
                50.25 + row as f32 * 0.5
            ));
        }
        content.push_str("ET");
        let document = PdfDoc::from_bytes(selectable_pdf_content(&content)).unwrap();

        let error = document.selection_snapshot(0, 1.0).unwrap_err();

        assert!(
            error
                .downcast_ref::<crate::application::ResourceLimitError>()
                .is_some()
        );
    }

    #[test]
    fn selection_snapshot_honors_cooperative_cancellation() {
        let document = PdfDoc::from_bytes(selectable_pdf("AB")).unwrap();

        let error = document
            .selection_snapshot_cancellable(0, 1.0, &|| true)
            .unwrap_err();

        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn selection_snapshot_rejects_excess_pdfium_endpoints_without_truncating() {
        let error = validate_pdf_selection_endpoint_count(65_537).unwrap_err();

        assert!(error.to_string().contains("65536-endpoint"));
    }

    #[test]
    fn selection_snapshot_rejects_invalid_scales_before_pdfium_work() {
        let document = PdfDoc::from_bytes(selectable_pdf("TARGET")).unwrap();

        assert!(document.selection_snapshot(0, f32::NAN).is_err());
        assert!(document.selection_snapshot(0, 0.0).is_err());
        assert!(document.selection_snapshot(0, -1.0).is_err());
        assert!(validate_pdf_bitmap_size(300.0, 200.0, 100_000.0).is_err());
        assert!(document.render_page(0, 100_000.0).is_err());
    }

    #[test]
    fn pdf_input_is_rejected_before_parsing_when_it_exceeds_the_limit() {
        let error = PdfDoc::from_bytes_with_limit(vec![0; 5], 4).unwrap_err();

        assert!(error.to_string().contains("4-byte input limit"));
        assert!(
            error
                .downcast_ref::<crate::application::ResourceLimitError>()
                .is_some()
        );
    }

    #[test]
    fn bounded_page_text_stops_during_character_extraction() {
        let document = PdfDoc::from_bytes(selectable_pdf("TARGET")).unwrap();
        let checks = Cell::new(0);

        let result = document.page_text_bounded(0, usize::MAX, || {
            checks.set(checks.get() + 1);
            checks.get() > 3
        });

        assert!(matches!(result, Err(BoundedPageTextError::Cancelled)));
        assert_eq!(checks.get(), 4);
    }

    #[test]
    fn bounded_page_text_rejects_text_that_cannot_fit() {
        let document = PdfDoc::from_bytes(selectable_pdf("TARGET")).unwrap();

        assert!(matches!(
            document.page_text_bounded(0, 1, || false),
            Err(BoundedPageTextError::Limit { actual }) if actual > 1
        ));
    }

    #[test]
    fn retained_charge_includes_input_allocation_capacity() {
        let bytes = selectable_pdf("TARGET");
        let logical_len = bytes.len();
        let mut overallocated = Vec::with_capacity(logical_len * 4);
        overallocated.extend_from_slice(&bytes);
        let capacity = overallocated.capacity();

        let document = PdfDoc::from_bytes(overallocated).unwrap();

        assert!(document.retained_byte_len().unwrap() >= capacity);
    }

    #[test]
    fn native_page_count_and_metadata_lengths_are_preflighted() {
        assert!(validate_pdf_preflight(u16::MAX as i32, &[1024]).is_ok());
        assert!(validate_pdf_preflight(u16::MAX as i32 + 1, &[0]).is_err());
        assert!(validate_pdf_preflight(1, &[super::MAX_PDF_METADATA_BYTES + 1]).is_err());
    }
}

/// A PDF document backed by pdfium-render.
#[derive(Debug)]
pub struct PdfDoc {
    page_count: usize,
    page_sizes: Vec<(f32, f32)>,
    metadata: DocumentMetadata,
    /// Raw PDF bytes, kept for re-opening during render calls.
    data: Vec<u8>,
    _admission: Option<crate::document_admission::DocumentAdmission>,
}

impl PdfDoc {
    pub(crate) fn retained_byte_len(&self) -> Option<usize> {
        let metadata = [
            &self.metadata.title,
            &self.metadata.author,
            &self.metadata.subject,
            &self.metadata.creator,
        ]
        .into_iter()
        .flatten()
        .try_fold(0_usize, |total, value| total.checked_add(value.capacity()))?;
        self.data
            .capacity()
            .checked_add(
                self.page_sizes
                    .capacity()
                    .checked_mul(std::mem::size_of::<(f32, f32)>())?,
            )?
            .checked_add(metadata)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
    }

    pub(crate) fn selection_admission_byte_len(&self, index: usize, scale: f32) -> Result<usize> {
        let (width, height) = self
            .page_sizes
            .get(index)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("page index {index} out of range"))?;
        let (width, height) = validate_pdf_bitmap_size(width, height, scale)?;
        (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| bytes.checked_add(PDF_SELECTION_MAX_RETAINED_BYTES))
            .and_then(|bytes| bytes.checked_add(MAX_PDF_PAGE_TEXT_BYTES))
            .context("PDF selection admission overflowed")
    }

    /// Open a PDF file from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limit(path, MAX_PDF_INPUT_BYTES)
    }

    pub fn open_with_limit(path: impl AsRef<Path>, max_input_bytes: u64) -> Result<Self> {
        Self::open_with_limit_inner(path.as_ref(), max_input_bytes, None)
    }

    pub(crate) fn open_with_limit_cancellable(
        path: &Path,
        max_input_bytes: u64,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        Self::open_with_limit_inner(path, max_input_bytes, Some(is_cancelled))
    }

    fn open_with_limit_inner(
        path: &Path,
        max_input_bytes: u64,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let expected = file.metadata()?.len();
        if expected > max_input_bytes {
            crate::resource_limit!("PDF exceeds the {max_input_bytes}-byte input limit");
        }
        let retained_ceiling = crate::document_admission::pdf_retained_ceiling(
            usize::try_from(expected).unwrap_or(usize::MAX),
        )
        .context("PDF retained-memory admission overflowed")?;
        let admission =
            crate::document_admission::ProvisionalDocumentAdmission::acquire(retained_ceiling)?;
        let data = read_pdf_snapshot(file, path, expected, is_cancelled)?;
        check_cancelled(is_cancelled)?;
        let document =
            Self::from_bytes_with_limit_admitted(data, max_input_bytes, is_cancelled, admission)?;
        check_cancelled(is_cancelled)?;
        Ok(document)
    }

    /// Open a PDF from raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        Self::from_bytes_with_limit(data, MAX_PDF_INPUT_BYTES)
    }

    pub(crate) fn from_bytes_admitted_cancellable(
        data: Vec<u8>,
        admission: crate::document_admission::ProvisionalDocumentAdmission,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self> {
        Self::from_bytes_with_limit_admitted(data, MAX_PDF_INPUT_BYTES, is_cancelled, admission)
    }

    pub fn from_bytes_with_limit(data: Vec<u8>, max_input_bytes: u64) -> Result<Self> {
        Self::from_bytes_with_limit_inner(data, max_input_bytes, None)
    }

    fn from_bytes_with_limit_inner(
        data: Vec<u8>,
        max_input_bytes: u64,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self> {
        let retained_ceiling = crate::document_admission::pdf_retained_ceiling(data.capacity())
            .context("PDF retained-memory admission overflowed")?;
        let admission =
            crate::document_admission::ProvisionalDocumentAdmission::acquire(retained_ceiling)?;
        Self::from_bytes_with_limit_admitted(data, max_input_bytes, is_cancelled, admission)
    }

    fn from_bytes_with_limit_admitted(
        data: Vec<u8>,
        max_input_bytes: u64,
        is_cancelled: Option<&dyn Fn() -> bool>,
        admission: crate::document_admission::ProvisionalDocumentAdmission,
    ) -> Result<Self> {
        if u64::try_from(data.len()).unwrap_or(u64::MAX) > max_input_bytes {
            crate::resource_limit!("PDF exceeds the {max_input_bytes}-byte input limit");
        }
        check_cancelled(is_cancelled)?;
        let pdfium = create_pdfium()?;
        check_cancelled(is_cancelled)?;
        preflight_pdf(pdfium.bindings(), &data)?;
        check_cancelled(is_cancelled)?;
        let document = pdfium
            .load_pdf_from_byte_slice(&data, None)
            .map_err(|e| anyhow::anyhow!("failed to load PDF: {e}"))?;
        check_cancelled(is_cancelled)?;

        let page_count = document.pages().len() as usize;

        let mut page_sizes = Vec::with_capacity(page_count);
        for i in 0..page_count {
            check_cancelled(is_cancelled)?;
            let page = document
                .pages()
                .get(i as u16)
                .map_err(|e| anyhow::anyhow!("failed to get page {i}: {e}"))?;
            let w = page.width().value;
            let h = page.height().value;
            page_sizes.push((w, h));
        }

        check_cancelled(is_cancelled)?;
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
        let metadata_bytes = [
            &metadata.title,
            &metadata.author,
            &metadata.subject,
            &metadata.creator,
        ]
        .into_iter()
        .flatten()
        .try_fold(0_usize, |total, value| total.checked_add(value.len()))
        .filter(|total| *total <= MAX_PDF_METADATA_BYTES);
        if metadata_bytes.is_none() {
            crate::resource_limit!("PDF metadata exceeds retained byte limit");
        }

        // Explicitly drop document and pdfium before moving `data` into the struct.
        // This releases the borrow on `data` and the global PDFium mutex lock.
        drop(document);
        drop(pdfium);

        let mut parsed = Self {
            page_count,
            page_sizes,
            metadata,
            data,
            _admission: None,
        };
        let retained_bytes = parsed
            .retained_byte_len()
            .context("PDF retained-memory charge overflowed")?;
        parsed._admission = Some(admission.finish(retained_bytes)?);
        Ok(parsed)
    }
}

#[cfg(test)]
fn read_pdf_file_with_limit(
    file: std::fs::File,
    path: &Path,
    max_input_bytes: u64,
) -> Result<Vec<u8>> {
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.len() > max_input_bytes {
        crate::resource_limit!("PDF exceeds the {max_input_bytes}-byte input limit");
    }
    read_pdf_snapshot(file, path, metadata.len(), None)
}
fn read_pdf_snapshot(
    mut file: std::fs::File,
    path: &Path,
    expected: u64,
    is_cancelled: Option<&dyn Fn() -> bool>,
) -> Result<Vec<u8>> {
    let capacity = usize::try_from(expected).context("PDF size cannot be represented")?;
    let mut data = Vec::with_capacity(capacity);
    let mut buffer = [0_u8; 64 * 1024];
    while data.len() < capacity {
        check_cancelled(is_cancelled)?;
        let chunk = (capacity - data.len()).min(buffer.len());
        let read = file
            .read(&mut buffer[..chunk])
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    let mut extra = [0; 1];
    let grew = file.read(&mut extra)? != 0;
    if data.len() != capacity || grew {
        anyhow::bail!("PDF changed while reading");
    }
    Ok(data)
}

fn check_cancelled(is_cancelled: Option<&dyn Fn() -> bool>) -> Result<()> {
    if is_cancelled.is_some_and(|is_cancelled| is_cancelled()) {
        anyhow::bail!("import cancelled");
    }
    Ok(())
}

fn preflight_pdf(bindings: &dyn PdfiumLibraryBindings, data: &[u8]) -> Result<()> {
    let document = bindings.FPDF_LoadMemDocument64(data, None);
    if document.is_null() {
        anyhow::bail!("failed to load PDF for resource preflight");
    }
    struct DocumentGuard<'a> {
        bindings: &'a dyn PdfiumLibraryBindings,
        document: FPDF_DOCUMENT,
    }
    impl Drop for DocumentGuard<'_> {
        fn drop(&mut self) {
            self.bindings.FPDF_CloseDocument(self.document);
        }
    }
    let guard = DocumentGuard { bindings, document };
    let metadata_lengths = [
        "Title",
        "Author",
        "Subject",
        "Keywords",
        "Creator",
        "Producer",
        "CreationDate",
        "ModificationDate",
    ]
    .map(|tag| bindings.FPDF_GetMetaText(document, tag, std::ptr::null_mut(), 0) as usize);
    validate_pdf_preflight(bindings.FPDF_GetPageCount(document), &metadata_lengths)?;
    drop(guard);
    Ok(())
}

fn validate_pdf_preflight(page_count: i32, metadata_lengths: &[usize]) -> Result<()> {
    if !(0..=u16::MAX as i32).contains(&page_count) {
        crate::resource_limit!("PDF page count is outside the supported range");
    }
    let metadata_bytes = metadata_lengths
        .iter()
        .try_fold(0_usize, |total, bytes| total.checked_add(*bytes))
        .filter(|total| *total <= MAX_PDF_METADATA_BYTES);
    if metadata_bytes.is_none() {
        crate::resource_limit!("PDF metadata exceeds retained byte limit");
    }
    Ok(())
}

impl PdfDoc {
    /// Extract all text from a single page.
    pub fn page_text(&self, index: usize) -> Result<String> {
        self.page_text_bounded(index, MAX_PDF_PAGE_TEXT_BYTES, || false)
            .map_err(|error| match error {
                BoundedPageTextError::Cancelled => anyhow::anyhow!("PDF text extraction cancelled"),
                BoundedPageTextError::Limit { .. } => anyhow::anyhow!(
                    "PDF page text exceeds the {MAX_PDF_PAGE_TEXT_BYTES}-byte limit"
                ),
                BoundedPageTextError::Document(error) => error,
            })
    }

    pub(crate) fn page_text_bounded(
        &self,
        index: usize,
        max_bytes: usize,
        is_cancelled: impl Fn() -> bool,
    ) -> std::result::Result<String, BoundedPageTextError> {
        if index >= self.page_count {
            return Err(BoundedPageTextError::Document(anyhow::anyhow!(
                "page index {index} out of range (total: {})",
                self.page_count
            )));
        }
        if is_cancelled() {
            return Err(BoundedPageTextError::Cancelled);
        }

        let pdfium = create_pdfium().map_err(BoundedPageTextError::Document)?;
        let document = pdfium
            .load_pdf_from_byte_slice(&self.data, None)
            .map_err(|error| {
                BoundedPageTextError::Document(anyhow::anyhow!(
                    "failed to load PDF for text extraction: {error}"
                ))
            })?;
        let page = document.pages().get(index as u16).map_err(|error| {
            BoundedPageTextError::Document(anyhow::anyhow!("failed to get page {index}: {error}"))
        })?;
        let text = page.text().map_err(|error| {
            BoundedPageTextError::Document(anyhow::anyhow!(
                "failed to load text for page {index}: {error}"
            ))
        })?;

        // Each PDFium character produces at least one UTF-8 byte. Rejecting on
        // the character count avoids allocating PdfPageTextChars for a page
        // that cannot fit in the caller's remaining text budget.
        let character_count = usize::try_from(text.len()).unwrap_or(usize::MAX);
        if character_count > max_bytes {
            return Err(BoundedPageTextError::Limit {
                actual: character_count,
            });
        }

        searchable_page_text_bounded(&page, &text, max_bytes, is_cancelled)
    }

    /// Extracts a bounded owned hit-test snapshot for a page rendered at `scale`.
    ///
    /// This performs PDFium work once. Calling methods on the returned snapshot
    /// performs no PDFium, file, or database access.
    pub fn selection_snapshot(&self, index: usize, scale: f32) -> Result<PdfSelectionSnapshot> {
        self.selection_snapshot_cancellable(index, scale, &|| false)
    }

    pub fn selection_snapshot_cancellable(
        &self,
        index: usize,
        scale: f32,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<PdfSelectionSnapshot> {
        check_cancelled(Some(is_cancelled))?;
        if !scale.is_finite() || scale <= 0.0 {
            anyhow::bail!("page scale must be finite and positive");
        }
        if index >= self.page_count {
            anyhow::bail!(
                "page index {index} out of range (total: {})",
                self.page_count
            );
        }
        let pdfium = create_pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(&self.data, None)
            .map_err(|e| anyhow::anyhow!("failed to load PDF for selection extraction: {e}"))?;
        let page = document
            .pages()
            .get(index as u16)
            .map_err(|e| anyhow::anyhow!("failed to get page {index}: {e}"))?;
        let text = page
            .text()
            .map_err(|e| anyhow::anyhow!("failed to load text for page {index}: {e}"))?;
        check_cancelled(Some(is_cancelled))?;
        // PdfPageTextChars eagerly allocates an index for every character, so
        // reject from PDFium's cheap count before constructing it.
        let character_count = usize::try_from(text.len()).unwrap_or(usize::MAX);
        validate_pdf_selection_endpoint_count(character_count.saturating_mul(2))?;
        let chars = text.chars();

        let (pt_w, pt_h) = self.page_sizes[index];
        let (pixel_w, pixel_h) = validate_pdf_bitmap_size(pt_w, pt_h, scale)?;
        let config = PdfRenderConfig::new()
            .set_target_width(pixel_w)
            .set_maximum_height(pixel_h)
            .use_lcd_text_rendering(true);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| anyhow::anyhow!("failed to render PDF selection page {index}: {e}"))?;
        let bitmap_width = bitmap.width() as u32;
        let bitmap_height = bitmap.height() as u32;
        let mut character_zones = Vec::with_capacity(chars.len());
        for character in chars.iter() {
            check_cancelled(Some(is_cancelled))?;
            let Ok(page_bounds) = character.loose_bounds() else {
                continue;
            };
            let Some((left, top, right, bottom)) = rect_to_pixels(&page, page_bounds, &config)
            else {
                continue;
            };
            let bounds = PdfSelectionRect {
                left: left.max(0) as f32,
                top: top.max(0) as f32,
                right: right.min(bitmap_width as i32) as f32,
                bottom: bottom.min(bitmap_height as i32) as f32,
            };
            if bounds.left >= bounds.right || bounds.top >= bounds.bottom {
                continue;
            }
            let page_x = (page_bounds.left().value + page_bounds.right().value) / 2.0;
            let page_y = (page_bounds.bottom().value + page_bounds.top().value) / 2.0;
            let direction = character.unicode_char().and_then(pdf_text_direction);
            let orientation = character
                .matrix()
                .ok()
                .and_then(|matrix| {
                    let baseline_length = matrix.a().hypot(matrix.b());
                    if !baseline_length.is_finite() || baseline_length <= f32::EPSILON {
                        return None;
                    }
                    // Pdfium exposes integer device coordinates. Extend the baseline
                    // probe so a valid subpixel text matrix cannot quantize to zero.
                    let probe_scale = 1024.0 / baseline_length;
                    let origin = page
                        .points_to_pixels(PdfPoints::new(page_x), PdfPoints::new(page_y), &config)
                        .ok()?;
                    let baseline = page
                        .points_to_pixels(
                            PdfPoints::new(page_x + matrix.a() * probe_scale),
                            PdfPoints::new(page_y + matrix.b() * probe_scale),
                            &config,
                        )
                        .ok()?;
                    let dx = baseline.0 - origin.0;
                    let dy = baseline.1 - origin.1;
                    (dx != 0 || dy != 0).then(|| {
                        let vertical = dy.abs() > dx.abs();
                        (vertical, if vertical { dy > 0 } else { dx > 0 })
                    })
                })
                .map(|orientation| pdf_logical_orientation(orientation, direction));
            character_zones.push(PdfCharacterGeometry {
                bounds,
                page_bounds: (
                    page_bounds.left().value,
                    page_bounds.bottom().value,
                    page_bounds.right().value,
                    page_bounds.top().value,
                ),
                character: character.index(),
                page_x,
                page_y,
                orientation,
                direction,
            });
        }
        resolve_pdf_neutral_directions(&mut character_zones);
        let mut zones = Vec::with_capacity(character_zones.len().saturating_mul(2));
        let mut glyphs = Vec::new();
        let mut start = 0;
        while start < character_zones.len() {
            check_cancelled(Some(is_cancelled))?;
            let mut end = start + 1;
            while end < character_zones.len()
                && character_zones[end].character
                    == character_zones[end - 1].character.saturating_add(1)
                && same_pdf_glyph(character_zones[start], character_zones[end])
            {
                end += 1;
            }
            glyphs.push(start..end);
            start = end;
        }
        for (position, glyph) in glyphs.iter().enumerate() {
            check_cancelled(Some(is_cancelled))?;
            let first = character_zones[glyph.start];
            let last = character_zones[glyph.end - 1];
            let previous = position
                .checked_sub(1)
                .map(|previous| character_zones[glyphs[previous].end - 1]);
            let next = glyphs
                .get(position + 1)
                .map(|next| character_zones[next.start]);
            let (vertical, forward_positive) = pdf_character_direction(previous, first, last, next);
            append_pdf_glyph_zones(
                &mut zones,
                &character_zones[glyph.clone()],
                vertical,
                forward_positive,
            );
        }
        validate_pdf_selection_endpoint_count(zones.len())?;
        let owned_text =
            searchable_page_text_bounded(&page, &text, MAX_PDF_PAGE_TEXT_BYTES, is_cancelled)
                .map_err(|error| match error {
                    BoundedPageTextError::Cancelled => anyhow::anyhow!("PDF selection cancelled"),
                    BoundedPageTextError::Limit { actual } => anyhow::anyhow!(
                        "PDF selection text ({actual} bytes) exceeds its byte limit"
                    ),
                    BoundedPageTextError::Document(error) => error,
                })?;
        // PDFium indexes remain untouched for annotation ranges, while every
        // interactive half-zone is snapped to an extended grapheme boundary.
        let graphemes = grapheme_ranges(&owned_text);
        for zone in &mut zones {
            if let Some(boundary) = grapheme_boundary_for_character(
                &graphemes,
                zone.character,
                zone.endpoint.character != zone.character,
            ) {
                zone.endpoint.character = boundary;
            }
        }
        let snapshot = PdfSelectionSnapshot {
            bitmap_width,
            bitmap_height,
            text: owned_text,
            rows: pdf_selection_rows(zones),
        };
        if snapshot.retained_bytes() > PDF_SELECTION_MAX_RETAINED_BYTES {
            anyhow::bail!(
                "PDF page exceeds the {PDF_SELECTION_MAX_RETAINED_BYTES}-byte selection geometry ceiling"
            );
        }
        Ok(snapshot)
    }

    /// Render a page and tint the text ranges used by in-document search.
    ///
    /// Each tuple contains a character offset, character count, and whether
    /// the range is the currently selected search result.
    pub fn render_page_with_highlights(
        &self,
        index: usize,
        scale: f32,
        highlights: &[(usize, usize, bool)],
    ) -> Result<RenderedPage> {
        self.render_page_impl(index, scale, highlights, None)
    }

    #[doc(hidden)]
    pub fn render_page_with_highlights_cancellable(
        &self,
        index: usize,
        scale: f32,
        highlights: &[(usize, usize, bool)],
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<RenderedPage> {
        self.render_page_impl(index, scale, highlights, Some(is_cancelled))
    }

    pub fn rendered_byte_len(&self, index: usize, scale: f32) -> Result<usize> {
        let (width, height) = self.render_dimensions(index, scale)?;
        (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("PDF raster byte size overflow")
    }

    /// Temporary native bitmap bytes retained while producing the owned RGBA output.
    pub fn render_transient_byte_len(&self, index: usize, scale: f32) -> Result<usize> {
        self.rendered_byte_len(index, scale)
    }

    fn render_dimensions(&self, index: usize, scale: f32) -> Result<(i32, i32)> {
        let &(width, height) = self.page_sizes.get(index).with_context(|| {
            format!(
                "page index {index} out of range (total: {})",
                self.page_count
            )
        })?;
        validate_pdf_bitmap_size(width, height, scale)
    }

    fn render_page_impl(
        &self,
        index: usize,
        scale: f32,
        highlights: &[(usize, usize, bool)],
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<RenderedPage> {
        check_cancelled(is_cancelled)?;
        let (pixel_w, pixel_h) = self.render_dimensions(index, scale)?;

        let pdfium = create_pdfium()?;
        check_cancelled(is_cancelled)?;
        let document = pdfium
            .load_pdf_from_byte_slice(&self.data, None)
            .map_err(|e| anyhow::anyhow!("failed to load PDF for rendering: {e}"))?;
        let page = document
            .pages()
            .get(index as u16)
            .map_err(|e| anyhow::anyhow!("failed to get page {index}: {e}"))?;

        let config = PdfRenderConfig::new()
            .set_target_width(pixel_w)
            .set_maximum_height(pixel_h)
            .use_lcd_text_rendering(true);
        check_cancelled(is_cancelled)?;
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| anyhow::anyhow!("failed to render page {index}: {e}"))?;
        check_cancelled(is_cancelled)?;

        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;
        let mut pixels = bitmap.as_rgba_bytes();

        if !highlights.is_empty()
            && let Ok(text) = page.text()
        {
            let chars = text.chars();

            for &(offset, length, current) in highlights {
                check_cancelled(is_cancelled)?;
                let end = offset.saturating_add(length).min(chars.len());
                for char_index in offset..end {
                    check_cancelled(is_cancelled)?;
                    let Ok(character) = chars.get(char_index) else {
                        continue;
                    };
                    let Ok(bounds) = character.loose_bounds() else {
                        continue;
                    };
                    if let Some(bounds) = rect_to_pixels(&page, bounds, &config) {
                        tint_rectangle(&mut pixels, width, height, bounds, current);
                    }
                }
            }
        }

        check_cancelled(is_cancelled)?;
        Ok(RenderedPage {
            width,
            height,
            pixels: bytes::Bytes::from(pixels),
        })
    }
}

fn grapheme_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut scalar = 0;
    text.graphemes(true)
        .map(|grapheme| {
            let start = scalar;
            scalar += grapheme.chars().count();
            start..scalar
        })
        .collect()
}

fn grapheme_boundary_for_character(
    graphemes: &[std::ops::Range<usize>],
    character: usize,
    trailing: bool,
) -> Option<usize> {
    graphemes
        .iter()
        .find(|range| range.start <= character && character < range.end)
        .map(|range| if trailing { range.end } else { range.start })
}

fn selection_neighbor_direction(
    current: PdfSelectionRect,
    next: PdfSelectionRect,
) -> Option<(bool, bool)> {
    let dx = next.left + next.right - current.left - current.right;
    let dy = next.top + next.bottom - current.top - current.bottom;
    let vertical = dy.abs() > dx.abs();
    let shares_run_axis = if vertical {
        next.right > current.left && next.left < current.right
    } else {
        next.bottom > current.top && next.top < current.bottom
    };
    if !shares_run_axis || (dx == 0.0 && dy == 0.0) {
        return None;
    }
    Some((vertical, if vertical { dy > 0.0 } else { dx > 0.0 }))
}

fn pdf_text_direction(character: char) -> Option<PdfTextDirection> {
    match unicode_bidi::bidi_class(character) {
        BidiClass::L | BidiClass::EN | BidiClass::AN => Some(PdfTextDirection::LeftToRight),
        BidiClass::R | BidiClass::AL => Some(PdfTextDirection::RightToLeft),
        _ => None,
    }
}

fn same_pdf_direction(left: Option<PdfTextDirection>, right: Option<PdfTextDirection>) -> bool {
    left == right
}

fn resolve_pdf_neutral_directions(characters: &mut [PdfCharacterGeometry]) {
    let mut start = 0;
    while start < characters.len() {
        if characters[start].direction.is_some() {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < characters.len() && characters[end].direction.is_none() {
            end += 1;
        }
        let previous = start.checked_sub(1).map(|index| characters[index]);
        let next = characters.get(end).copied();
        let contiguous = characters[start..end]
            .windows(2)
            .all(|pair| pair[1].character == pair[0].character.saturating_add(1));
        let direction = previous.and_then(|character| character.direction);
        if contiguous
            && previous.is_some_and(|character| {
                characters[start].character == character.character.saturating_add(1)
            })
            && next.is_some_and(|character| {
                character.character == characters[end - 1].character.saturating_add(1)
            })
            && direction.is_some()
            && direction == next.and_then(|character| character.direction)
        {
            for character in &mut characters[start..end] {
                character.direction = direction;
                character.orientation = character
                    .orientation
                    .map(|orientation| pdf_logical_orientation(orientation, direction));
            }
        }
        start = end;
    }
}

fn pdf_logical_orientation(
    (vertical, forward_positive): (bool, bool),
    direction: Option<PdfTextDirection>,
) -> (bool, bool) {
    (
        vertical,
        if direction == Some(PdfTextDirection::RightToLeft) {
            !forward_positive
        } else {
            forward_positive
        },
    )
}

fn pdf_character_direction(
    previous: Option<PdfCharacterGeometry>,
    first: PdfCharacterGeometry,
    last: PdfCharacterGeometry,
    next: Option<PdfCharacterGeometry>,
) -> (bool, bool) {
    let next_direction = next.and_then(|next| {
        (next.character == last.character.saturating_add(1)
            && same_pdf_direction(last.direction, next.direction))
        .then(|| selection_neighbor_direction(first.bounds, next.bounds))
        .flatten()
    });
    let previous_direction = previous.and_then(|previous| {
        (first.character == previous.character.saturating_add(1)
            && same_pdf_direction(previous.direction, first.direction))
        .then(|| selection_neighbor_direction(previous.bounds, first.bounds))
        .flatten()
    });
    next_direction
        .or(previous_direction)
        .or(first.orientation)
        .unwrap_or((false, true))
}

fn same_pdf_glyph(left: PdfCharacterGeometry, right: PdfCharacterGeometry) -> bool {
    left.bounds == right.bounds
        && left.page_bounds == right.page_bounds
        && left.page_x == right.page_x
        && left.page_y == right.page_y
}

fn selection_subdivision(
    bounds: PdfSelectionRect,
    vertical: bool,
    index: usize,
    count: usize,
) -> PdfSelectionRect {
    debug_assert!(index < count && count > 0);
    let start = index as f32 / count as f32;
    let end = (index + 1) as f32 / count as f32;
    if vertical {
        let height = bounds.bottom - bounds.top;
        PdfSelectionRect {
            top: bounds.top + height * start,
            bottom: bounds.top + height * end,
            ..bounds
        }
    } else {
        let width = bounds.right - bounds.left;
        PdfSelectionRect {
            left: bounds.left + width * start,
            right: bounds.left + width * end,
            ..bounds
        }
    }
}

fn append_pdf_glyph_zones(
    zones: &mut Vec<PdfSelectionZone>,
    glyph: &[PdfCharacterGeometry],
    vertical: bool,
    forward_positive: bool,
) {
    let bounds = glyph[0].bounds;
    let half_count = glyph.len().saturating_mul(2);
    for (glyph_position, geometry) in glyph.iter().enumerate() {
        for (caret, logical_half) in [
            (geometry.character, glyph_position * 2),
            (geometry.character.saturating_add(1), glyph_position * 2 + 1),
        ] {
            let physical_half = if forward_positive {
                logical_half
            } else {
                half_count - logical_half - 1
            };
            zones.push(PdfSelectionZone {
                bounds: selection_subdivision(bounds, vertical, physical_half, half_count),
                page_bounds: geometry.page_bounds,
                character: geometry.character,
                endpoint: PdfSelectionEndpoint {
                    underlying_character: geometry.character,
                    character: caret,
                    page_x: geometry.page_x,
                    page_y: geometry.page_y,
                },
            });
        }
    }
}

#[derive(Debug)]
pub(crate) enum BoundedPageTextError {
    Cancelled,
    Limit { actual: usize },
    Document(anyhow::Error),
}

fn validate_pdf_selection_endpoint_count(count: usize) -> Result<()> {
    if count > PDF_SELECTION_MAX_ENDPOINTS {
        crate::resource_limit!(
            "PDF page exceeds the {PDF_SELECTION_MAX_ENDPOINTS}-endpoint selection ceiling"
        );
    }
    Ok(())
}

fn validate_pdf_bitmap_size(width: f32, height: f32, scale: f32) -> Result<(i32, i32)> {
    let width = (f64::from(width) * f64::from(scale)).ceil();
    let height = (f64::from(height) * f64::from(scale)).ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > f64::from(MAX_PDF_BITMAP_DIMENSION)
        || height > f64::from(MAX_PDF_BITMAP_DIMENSION)
        || width * height > MAX_PDF_BITMAP_PIXELS as f64
    {
        crate::resource_limit!("PDF bitmap exceeds decoded image limits");
    }
    Ok((width as i32, height as i32))
}

fn pdf_selection_rows(mut zones: Vec<PdfSelectionZone>) -> Vec<PdfSelectionRow> {
    zones.sort_by(|left, right| {
        left.bounds
            .top
            .total_cmp(&right.bounds.top)
            .then_with(|| left.bounds.left.total_cmp(&right.bounds.left))
    });
    let mut rows: Vec<PdfSelectionRow> = Vec::new();
    for zone in zones {
        if let Some(row) = rows.last_mut().filter(|row| {
            zone.bounds.bottom > row.bounds.top && zone.bounds.top < row.bounds.bottom
        }) {
            row.bounds.left = row.bounds.left.min(zone.bounds.left);
            row.bounds.top = row.bounds.top.min(zone.bounds.top);
            row.bounds.right = row.bounds.right.max(zone.bounds.right);
            row.bounds.bottom = row.bounds.bottom.max(zone.bounds.bottom);
            row.zones.push(zone);
        } else {
            rows.push(PdfSelectionRow {
                bounds: zone.bounds,
                zones: vec![zone],
            });
        }
    }
    rows
}

fn searchable_page_text_bounded(
    page: &PdfPage<'_>,
    text: &PdfPageText<'_>,
    max_bytes: usize,
    is_cancelled: impl Fn() -> bool,
) -> std::result::Result<String, BoundedPageTextError> {
    let page_bounds = page
        .boundaries()
        .bounding()
        .ok()
        .map(|boundary| boundary.bounds);
    let chars = text.chars();
    let mut result = String::with_capacity(chars.len().min(max_bytes));

    for character in chars.iter() {
        if is_cancelled() {
            return Err(BoundedPageTextError::Cancelled);
        }
        // Generated whitespace and line breaks must remain to preserve PDFium
        // character indexes even though they often have no visible bounds.
        let visible = character.is_generated().unwrap_or(false)
            || character.loose_bounds().is_ok_and(|bounds| {
                page_bounds.is_some_and(|page_bounds| bounds.does_overlap(&page_bounds))
            });
        let character = if visible {
            character
                .unicode_char()
                .filter(|character| *character != '\0')
                .unwrap_or('\u{FFFD}')
        } else {
            '\u{FFFD}'
        };
        let actual = result.len().saturating_add(character.len_utf8());
        if actual > max_bytes {
            return Err(BoundedPageTextError::Limit { actual });
        }
        result.push(character);
    }

    Ok(result)
}

fn rect_to_pixels(
    page: &PdfPage<'_>,
    bounds: PdfRect,
    config: &PdfRenderConfig,
) -> Option<(i32, i32, i32, i32)> {
    let corners = [
        page.points_to_pixels(bounds.left(), bounds.bottom(), config)
            .ok()?,
        page.points_to_pixels(bounds.left(), bounds.top(), config)
            .ok()?,
        page.points_to_pixels(bounds.right(), bounds.bottom(), config)
            .ok()?,
        page.points_to_pixels(bounds.right(), bounds.top(), config)
            .ok()?,
    ];
    Some((
        corners.iter().map(|(x, _)| *x).min()?,
        corners.iter().map(|(_, y)| *y).min()?,
        corners.iter().map(|(x, _)| *x).max()?,
        corners.iter().map(|(_, y)| *y).max()?,
    ))
}

fn tint_rectangle(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    bounds: (i32, i32, i32, i32),
    current: bool,
) {
    let (left, top, right, bottom) = bounds;
    let left = left.clamp(0, width as i32) as u32;
    let right = right.clamp(0, width as i32) as u32;
    let top = top.clamp(0, height as i32) as u32;
    let bottom = bottom.clamp(0, height as i32) as u32;
    let color = if current {
        [255_u16, 160_u16, 60_u16]
    } else {
        [255_u16, 225_u16, 70_u16]
    };
    let alpha = if current { 120_u16 } else { 95_u16 };

    for y in top..bottom {
        for x in left..right {
            let pixel = ((y * width + x) * 4) as usize;
            for channel in 0..3 {
                pixels[pixel + channel] = ((pixels[pixel + channel] as u16 * (255 - alpha)
                    + color[channel] * alpha)
                    / 255) as u8;
            }
        }
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
        self.render_page_impl(index, scale, &[], None)
    }

    fn metadata(&self) -> DocumentMetadata {
        self.metadata.clone()
    }
}
