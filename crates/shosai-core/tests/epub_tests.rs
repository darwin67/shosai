use shosai_core::epub::EpubDoc;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_open_epub() {
    let doc = EpubDoc::open(fixture_path("sample.epub"));
    assert!(doc.is_ok(), "failed to open EPUB: {:?}", doc.err());
}

#[test]
fn test_chapter_count() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    assert_eq!(doc.chapter_count(), 2);
}

#[test]
fn test_metadata_title() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    let meta = doc.metadata();
    assert_eq!(meta.title.as_deref(), Some("Sample Book"));
}

#[test]
fn test_metadata_author() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    let meta = doc.metadata();
    assert_eq!(meta.author.as_deref(), Some("Test Author"));
}

#[test]
fn test_epub_metadata_full() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    let meta = &doc.content.metadata;
    assert_eq!(meta.language.as_deref(), Some("en"));
    assert_eq!(meta.publisher.as_deref(), Some("Shosai Press"));
    assert_eq!(
        meta.description.as_deref(),
        Some("A test EPUB for unit testing.")
    );
    assert_eq!(meta.cover_image_id.as_deref(), Some("cover-img"));
}

#[test]
fn test_chapter_content_loaded() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();

    let ch1 = doc.chapter(0).expect("chapter 0 should exist");
    assert!(
        ch1.content.contains("first"),
        "chapter 1 should contain 'first'"
    );
    assert!(
        ch1.content.contains("<strong>"),
        "chapter 1 should contain <strong> tags"
    );

    let ch2 = doc.chapter(1).expect("chapter 1 should exist");
    assert!(
        ch2.content.contains("Getting Started"),
        "chapter 2 should contain 'Getting Started'"
    );
}

#[test]
fn test_chapter_titles_from_toc() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();

    let ch1 = doc.chapter(0).unwrap();
    assert_eq!(
        ch1.title.as_deref(),
        Some("Chapter 1: Introduction"),
        "chapter 1 title should come from TOC"
    );

    let ch2 = doc.chapter(1).unwrap();
    assert_eq!(
        ch2.title.as_deref(),
        Some("Chapter 2: Getting Started"),
        "chapter 2 title should come from TOC"
    );
}

#[test]
fn test_toc_entries() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    let toc = doc.toc();

    assert_eq!(toc.len(), 2, "should have 2 TOC entries");
    assert_eq!(toc[0].title, "Chapter 1: Introduction");
    assert_eq!(toc[1].title, "Chapter 2: Getting Started");
}

#[test]
fn test_resources_loaded() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();

    // CSS should be loaded as a resource
    let css = doc.resource("OEBPS/style.css");
    assert!(css.is_some(), "CSS resource should be loaded");
    let css_text = std::str::from_utf8(css.unwrap()).unwrap();
    assert!(
        css_text.contains("font-family"),
        "CSS should contain styles"
    );

    // Cover image should be loaded
    let cover = doc.resource("OEBPS/images/cover.png");
    assert!(cover.is_some(), "cover image should be loaded");
    assert!(
        cover.unwrap().starts_with(&[0x89, b'P', b'N', b'G']),
        "cover should be a valid PNG"
    );
}

#[test]
fn test_manifest_entries() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    let manifest = &doc.content.manifest;

    assert!(manifest.contains_key("ch1"), "manifest should contain ch1");
    assert!(manifest.contains_key("ch2"), "manifest should contain ch2");
    assert!(
        manifest.contains_key("style"),
        "manifest should contain style"
    );
    assert!(
        manifest.contains_key("cover-img"),
        "manifest should contain cover-img"
    );
}

#[test]
fn test_chapter_out_of_range() {
    let doc = EpubDoc::open(fixture_path("sample.epub")).unwrap();
    assert!(doc.chapter(99).is_none());
}

#[test]
fn test_from_bytes() {
    let data = std::fs::read(fixture_path("sample.epub")).unwrap();
    let doc = EpubDoc::from_bytes(data);
    assert!(doc.is_ok(), "from_bytes failed: {:?}", doc.err());
    assert_eq!(doc.unwrap().chapter_count(), 2);
}

#[test]
fn test_open_nonexistent_file() {
    let result = EpubDoc::open("/nonexistent/path/to/file.epub");
    assert!(result.is_err());
}
