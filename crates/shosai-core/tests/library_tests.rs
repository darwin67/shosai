use shosai_core::library::{BookFormat, Library};
use shosai_core::reading_state::ReadingStateStore;
use std::path::PathBuf;
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

async fn temp_library() -> (Library, ReadingStateStore, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("shosai.db");
    let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
    let library = Library::new(store.pool().clone());
    (library, store, dir)
}

#[tokio::test]
async fn test_import_pdf() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.pdf")).await.unwrap();
    assert_eq!(book.format, BookFormat::Pdf);
    assert!(!book.title.is_empty());
}

#[tokio::test]
async fn test_import_epub() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.epub")).await.unwrap();
    assert_eq!(book.format, BookFormat::Epub);
    assert_eq!(book.title, "Sample Book");
    assert_eq!(book.author.as_deref(), Some("Test Author"));
}

#[tokio::test]
async fn test_import_cbz() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.cbz")).await.unwrap();
    assert_eq!(book.format, BookFormat::Cbz);
    assert_eq!(book.title, "sample");
}

#[tokio::test]
async fn test_import_duplicate_returns_existing() {
    let (lib, _, _dir) = temp_library().await;
    let book1 = lib.import_file(&fixture_path("sample.pdf")).await.unwrap();
    let book2 = lib.import_file(&fixture_path("sample.pdf")).await.unwrap();
    assert_eq!(book1.id, book2.id);
}

#[tokio::test]
async fn test_list_all() {
    let (lib, _, _dir) = temp_library().await;
    lib.import_file(&fixture_path("sample.pdf")).await.unwrap();
    lib.import_file(&fixture_path("sample.epub")).await.unwrap();
    lib.import_file(&fixture_path("sample.cbz")).await.unwrap();

    let books = lib.list_all().await.unwrap();
    assert_eq!(books.len(), 3);
}

#[tokio::test]
async fn test_search_by_title() {
    let (lib, _, _dir) = temp_library().await;
    lib.import_file(&fixture_path("sample.epub")).await.unwrap();
    lib.import_file(&fixture_path("sample.pdf")).await.unwrap();

    let results = lib.search("Sample Book").await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Sample Book");
}

#[tokio::test]
async fn test_search_by_author() {
    let (lib, _, _dir) = temp_library().await;
    lib.import_file(&fixture_path("sample.epub")).await.unwrap();

    let results = lib.search("Test Author").await.unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_filter_by_format() {
    let (lib, _, _dir) = temp_library().await;
    lib.import_file(&fixture_path("sample.pdf")).await.unwrap();
    lib.import_file(&fixture_path("sample.epub")).await.unwrap();
    lib.import_file(&fixture_path("sample.cbz")).await.unwrap();

    let pdfs = lib.filter_by_format(BookFormat::Pdf).await.unwrap();
    assert_eq!(pdfs.len(), 1);
    assert_eq!(pdfs[0].format, BookFormat::Pdf);

    let epubs = lib.filter_by_format(BookFormat::Epub).await.unwrap();
    assert_eq!(epubs.len(), 1);
}

#[tokio::test]
async fn test_update_progress() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.pdf")).await.unwrap();
    assert!((book.progress - 0.0).abs() < f64::EPSILON);

    lib.update_progress(book.id, 0.5).await.unwrap();

    let books = lib.list_all().await.unwrap();
    let updated = books.iter().find(|b| b.id == book.id).unwrap();
    assert!((updated.progress - 0.5).abs() < f64::EPSILON);
    assert!(updated.last_read.is_some());
}

#[tokio::test]
async fn test_remove() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.pdf")).await.unwrap();

    lib.remove(book.id).await.unwrap();

    let books = lib.list_all().await.unwrap();
    assert!(books.is_empty());
}

#[tokio::test]
async fn test_cover_extracted_for_epub() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.epub")).await.unwrap();
    // Our sample EPUB has a cover image
    assert!(book.cover.is_some(), "EPUB should have a cover");
    let cover = book.cover.unwrap();
    // Should be valid PNG
    assert!(cover.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[tokio::test]
async fn test_cover_extracted_for_cbz() {
    let (lib, _, _dir) = temp_library().await;
    let book = lib.import_file(&fixture_path("sample.cbz")).await.unwrap();
    assert!(book.cover.is_some(), "CBZ should have a cover");
    let cover = book.cover.unwrap();
    assert!(cover.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[tokio::test]
async fn test_import_unsupported_format() {
    let (lib, _, dir) = temp_library().await;
    let txt_path = dir.path().join("test.txt");
    std::fs::write(&txt_path, "hello").unwrap();
    let result = lib.import_file(&txt_path).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_import_directory() {
    let (lib, _, dir) = temp_library().await;

    // Create a directory with some fixtures copied in.
    let import_dir = dir.path().join("imports");
    std::fs::create_dir_all(&import_dir).unwrap();
    std::fs::copy(fixture_path("sample.pdf"), import_dir.join("book.pdf")).unwrap();
    std::fs::copy(fixture_path("sample.epub"), import_dir.join("book.epub")).unwrap();
    // Also a non-book file that should be skipped.
    std::fs::write(import_dir.join("notes.txt"), "some notes").unwrap();

    let books = lib.import_directory(&import_dir).await.unwrap();
    assert_eq!(books.len(), 2);
}
