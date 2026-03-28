use serial_test::serial;
use shosai_core::document::Document;
use shosai_core::pdf::PdfDoc;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
#[serial]
fn test_open_pdf() {
    let doc = PdfDoc::open(fixture_path("sample.pdf"));
    assert!(doc.is_ok(), "failed to open PDF: {:?}", doc.err());
}

#[test]
#[serial]
fn test_page_count() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();
    assert_eq!(doc.page_count(), 2);
}

#[test]
#[serial]
fn test_page_size() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();

    let (w, h) = doc.page_size(0).unwrap();
    // US Letter: 612 x 792 points
    assert!((w - 612.0).abs() < 1.0, "unexpected width: {w}");
    assert!((h - 792.0).abs() < 1.0, "unexpected height: {h}");

    // Second page should have same dimensions
    let (w2, h2) = doc.page_size(1).unwrap();
    assert!((w2 - 612.0).abs() < 1.0);
    assert!((h2 - 792.0).abs() < 1.0);
}

#[test]
#[serial]
fn test_page_size_out_of_range() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();
    assert!(doc.page_size(99).is_err());
}

#[test]
#[serial]
fn test_render_page() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();
    let page = doc.render_page(0, 1.0).unwrap();

    // Page should have non-zero dimensions
    assert!(page.width > 0, "rendered page width is 0");
    assert!(page.height > 0, "rendered page height is 0");

    // RGBA pixels: width * height * 4 bytes
    assert_eq!(
        page.pixels.len(),
        (page.width * page.height * 4) as usize,
        "pixel buffer size mismatch"
    );
}

#[test]
#[serial]
fn test_render_page_scaled() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();

    let page_1x = doc.render_page(0, 1.0).unwrap();
    let page_2x = doc.render_page(0, 2.0).unwrap();

    // At 2x scale, dimensions should be roughly double
    assert!(
        page_2x.width > page_1x.width,
        "2x width {} should be > 1x width {}",
        page_2x.width,
        page_1x.width
    );
    assert!(
        page_2x.height > page_1x.height,
        "2x height {} should be > 1x height {}",
        page_2x.height,
        page_1x.height
    );
}

#[test]
#[serial]
fn test_render_page_out_of_range() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();
    assert!(doc.render_page(99, 1.0).is_err());
}

#[test]
#[serial]
fn test_metadata() {
    let doc = PdfDoc::open(fixture_path("sample.pdf")).unwrap();
    let _meta = doc.metadata();
    // Our minimal test PDF doesn't have metadata, but the call shouldn't panic
}

#[test]
#[serial]
fn test_from_bytes() {
    let data = std::fs::read(fixture_path("sample.pdf")).unwrap();
    let doc = PdfDoc::from_bytes(data);
    assert!(doc.is_ok(), "from_bytes failed: {:?}", doc.err());
    assert_eq!(doc.unwrap().page_count(), 2);
}

#[test]
fn test_open_nonexistent_file() {
    let result = PdfDoc::open("/nonexistent/path/to/file.pdf");
    assert!(result.is_err());
}
