use shosai_core::cbz::CbzDoc;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_open_cbz() {
    let doc = CbzDoc::open(fixture_path("sample.cbz"));
    assert!(doc.is_ok(), "failed to open CBZ: {:?}", doc.err());
}

#[test]
fn test_page_count() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    // 3 image files (page1.png, page2.png, page10.png)
    assert_eq!(doc.page_count(), 3);
}

#[test]
fn test_natural_sort_order() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    // Natural sort: page1 < page2 < page10
    // Verify by rendering — page1 is red (255,0,0)
    let page0 = doc.render_page(0, 1.0).unwrap();
    // First pixel should be red
    assert!(page0.pixels[0] > 200, "first page should be red (R)");
    assert!(page0.pixels[1] < 50, "first page should be red (G)");

    // page10 is blue (0,0,255) and should be last (index 2)
    let page2 = doc.render_page(2, 1.0).unwrap();
    assert!(page2.pixels[0] < 50, "third page should be blue (R)");
    assert!(page2.pixels[2] > 200, "third page should be blue (B)");
}

#[test]
fn test_render_page() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    let page = doc.render_page(0, 1.0).unwrap();

    assert_eq!(page.width, 100);
    assert_eq!(page.height, 150);
    // RGBA: width * height * 4
    assert_eq!(page.pixels.len(), (100 * 150 * 4) as usize);
}

#[test]
fn test_render_page_scaled() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    let page_1x = doc.render_page(0, 1.0).unwrap();
    let page_2x = doc.render_page(0, 2.0).unwrap();

    assert!(page_2x.width > page_1x.width);
    assert!(page_2x.height > page_1x.height);
}

#[test]
fn test_page_size() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    let (w, h) = doc.page_size(0).unwrap();
    assert!((w - 100.0).abs() < 1.0);
    assert!((h - 150.0).abs() < 1.0);
}

#[test]
fn test_render_page_out_of_range() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    assert!(doc.render_page(99, 1.0).is_err());
}

#[test]
fn test_metadata_title() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    let meta = doc.metadata();
    assert_eq!(meta.title.as_deref(), Some("sample"));
}

#[test]
fn test_from_bytes() {
    let data = std::fs::read(fixture_path("sample.cbz")).unwrap();
    let doc = CbzDoc::from_bytes(data);
    assert!(doc.is_ok());
    assert_eq!(doc.unwrap().page_count(), 3);
}

#[test]
fn test_page_image_bytes() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    let bytes = doc.page_image_bytes(0).unwrap();
    // Should be valid PNG data
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn test_skips_non_image_files() {
    let doc = CbzDoc::open(fixture_path("sample.cbz")).unwrap();
    // ComicInfo.xml and __MACOSX/.DS_Store should be ignored
    assert_eq!(doc.page_count(), 3);
}

#[test]
fn test_open_nonexistent() {
    assert!(CbzDoc::open("/nonexistent/file.cbz").is_err());
}
