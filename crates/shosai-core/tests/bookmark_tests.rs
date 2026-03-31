use shosai_core::bookmarks::BookmarkStore;
use shosai_core::reading_state::ReadingStateStore;
use std::path::PathBuf;
use tempfile::TempDir;

async fn temp_store() -> (BookmarkStore, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("shosai.db");
    let rs = ReadingStateStore::open_at_async(&db_path).await.unwrap();
    let store = BookmarkStore::new(rs.pool().clone());
    (store, dir)
}

#[tokio::test]
async fn test_add_bookmark() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    let bm = store
        .add_async(&path, 5, Some("Chapter 3"), None, "yellow")
        .await
        .unwrap();

    assert_eq!(bm.page, 5);
    assert_eq!(bm.title.as_deref(), Some("Chapter 3"));
    assert!(bm.note.is_none());
    assert_eq!(bm.color, "yellow");
}

#[tokio::test]
async fn test_add_bookmark_with_note() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    let bm = store
        .add_async(&path, 10, None, Some("Important concept"), "blue")
        .await
        .unwrap();

    assert_eq!(bm.page, 10);
    assert_eq!(bm.note.as_deref(), Some("Important concept"));
    assert_eq!(bm.color, "blue");
}

#[tokio::test]
async fn test_list_for_file() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    store
        .add_async(&path, 1, Some("Page 1"), None, "yellow")
        .await
        .unwrap();
    store
        .add_async(&path, 10, Some("Page 10"), None, "yellow")
        .await
        .unwrap();
    store
        .add_async(&path, 5, Some("Page 5"), None, "yellow")
        .await
        .unwrap();

    let bookmarks = store.list_for_file_async(&path).await.unwrap();
    assert_eq!(bookmarks.len(), 3);
    // Should be sorted by page
    assert_eq!(bookmarks[0].page, 1);
    assert_eq!(bookmarks[1].page, 5);
    assert_eq!(bookmarks[2].page, 10);
}

#[tokio::test]
async fn test_list_only_returns_for_specified_file() {
    let (store, _dir) = temp_store().await;
    let path_a = PathBuf::from("/books/a.pdf");
    let path_b = PathBuf::from("/books/b.pdf");

    store
        .add_async(&path_a, 1, None, None, "yellow")
        .await
        .unwrap();
    store
        .add_async(&path_b, 2, None, None, "yellow")
        .await
        .unwrap();

    let bms_a = store.list_for_file_async(&path_a).await.unwrap();
    assert_eq!(bms_a.len(), 1);
    assert_eq!(bms_a[0].page, 1);
}

#[tokio::test]
async fn test_toggle_creates_and_removes() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    // First toggle: creates bookmark
    let result = store.toggle_async(&path, 5, Some("Page 5")).await.unwrap();
    assert!(result.is_some(), "should create bookmark");

    assert!(store.is_bookmarked_async(&path, 5).await);

    // Second toggle: removes bookmark
    let result = store.toggle_async(&path, 5, Some("Page 5")).await.unwrap();
    assert!(result.is_none(), "should remove bookmark");

    assert!(!store.is_bookmarked_async(&path, 5).await);
}

#[tokio::test]
async fn test_is_bookmarked() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    assert!(!store.is_bookmarked_async(&path, 5).await);

    store
        .add_async(&path, 5, None, None, "yellow")
        .await
        .unwrap();

    assert!(store.is_bookmarked_async(&path, 5).await);
    assert!(!store.is_bookmarked_async(&path, 6).await);
}

#[tokio::test]
async fn test_update_note() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    let bm = store
        .add_async(&path, 5, None, None, "yellow")
        .await
        .unwrap();

    store
        .update_note_async(bm.id, Some("Added a note later"))
        .await
        .unwrap();

    let bookmarks = store.list_for_file_async(&path).await.unwrap();
    assert_eq!(bookmarks[0].note.as_deref(), Some("Added a note later"));
}

#[tokio::test]
async fn test_update_title() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    let bm = store
        .add_async(&path, 5, Some("Old title"), None, "yellow")
        .await
        .unwrap();

    store
        .update_title_async(bm.id, Some("New title"))
        .await
        .unwrap();

    let bookmarks = store.list_for_file_async(&path).await.unwrap();
    assert_eq!(bookmarks[0].title.as_deref(), Some("New title"));
}

#[tokio::test]
async fn test_remove_bookmark() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    let bm = store
        .add_async(&path, 5, None, None, "yellow")
        .await
        .unwrap();

    store.remove_async(bm.id).await.unwrap();

    let bookmarks = store.list_for_file_async(&path).await.unwrap();
    assert!(bookmarks.is_empty());
}

#[tokio::test]
async fn test_remove_all_for_file() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    store
        .add_async(&path, 1, None, None, "yellow")
        .await
        .unwrap();
    store
        .add_async(&path, 5, None, None, "yellow")
        .await
        .unwrap();
    store
        .add_async(&path, 10, None, None, "yellow")
        .await
        .unwrap();

    store.remove_all_for_file_async(&path).await.unwrap();

    let bookmarks = store.list_for_file_async(&path).await.unwrap();
    assert!(bookmarks.is_empty());
}

#[tokio::test]
async fn test_multiple_bookmarks_on_same_page() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    // A page bookmark (no note) + a note on the same page
    store
        .add_async(&path, 5, Some("Page 5"), None, "yellow")
        .await
        .unwrap();
    store
        .add_async(&path, 5, None, Some("A note"), "blue")
        .await
        .unwrap();

    let bookmarks = store.list_for_file_async(&path).await.unwrap();
    assert_eq!(bookmarks.len(), 2);
}

#[tokio::test]
async fn test_export_markdown() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/test.pdf");

    store
        .add_async(&path, 0, Some("Introduction"), None, "yellow")
        .await
        .unwrap();
    store
        .add_async(
            &path,
            4,
            Some("Key Concept"),
            Some("This is very important."),
            "blue",
        )
        .await
        .unwrap();

    let md = store.export_markdown_async(&path).await.unwrap();

    assert!(md.contains("# Bookmarks: test.pdf"));
    assert!(md.contains("## Page 1: Introduction"));
    assert!(md.contains("## Page 5: Key Concept"));
    assert!(md.contains("This is very important."));
}

#[tokio::test]
async fn test_export_empty() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/empty.pdf");

    let md = store.export_markdown_async(&path).await.unwrap();
    assert!(md.contains("# Bookmarks: empty.pdf"));
    // No bookmark sections
    assert!(!md.contains("## Page"));
}
