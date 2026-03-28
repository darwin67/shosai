use shosai_core::reading_state::{FileReadingState, ReadingStateStore};
use std::path::PathBuf;

/// Create a store backed by a temporary SQLite database file.
async fn temp_store(name: &str) -> ReadingStateStore {
    let dir = std::env::temp_dir().join("shosai-test-db");
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join(format!("{name}.db"));
    let _ = std::fs::remove_file(&db_path);
    ReadingStateStore::open_at_async(&db_path).await.unwrap()
}

#[tokio::test]
async fn test_empty_store_returns_none() {
    let store = temp_store("empty").await;
    assert!(
        store
            .get_async(&PathBuf::from("/some/file.pdf"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn test_set_and_get() {
    let store = temp_store("set_get").await;
    let path = PathBuf::from("/tmp/test-shosai/test.pdf");

    store
        .set_async(&path, &FileReadingState { page: 5, zoom: 1.5 })
        .await
        .unwrap();

    let state = store.get_async(&path).await;
    assert!(state.is_some(), "reading state should exist after set");
    let state = state.unwrap();
    assert_eq!(state.page, 5);
    assert!((state.zoom - 1.5).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_persistence_across_opens() {
    let dir = std::env::temp_dir().join("shosai-test-db");
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("persist.db");
    let _ = std::fs::remove_file(&db_path);
    let path = PathBuf::from("/fake/path/book.pdf");

    // Write with one store instance.
    {
        let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
        store
            .set_async(
                &path,
                &FileReadingState {
                    page: 42,
                    zoom: 2.0,
                },
            )
            .await
            .unwrap();
    }

    // Open a new store instance and verify data persisted.
    {
        let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
        let state = store
            .get_async(&path)
            .await
            .expect("state should persist across opens");
        assert_eq!(state.page, 42);
        assert!((state.zoom - 2.0).abs() < f32::EPSILON);
    }
}

#[tokio::test]
async fn test_overwrite_state() {
    let store = temp_store("overwrite").await;
    let path = PathBuf::from("/tmp/test-shosai-overwrite.pdf");

    store
        .set_async(&path, &FileReadingState { page: 1, zoom: 1.0 })
        .await
        .unwrap();
    store
        .set_async(
            &path,
            &FileReadingState {
                page: 10,
                zoom: 3.0,
            },
        )
        .await
        .unwrap();

    let state = store.get_async(&path).await.unwrap();
    assert_eq!(state.page, 10);
    assert!((state.zoom - 3.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_multiple_files() {
    let store = temp_store("multi").await;
    let path_a = PathBuf::from("/books/a.pdf");
    let path_b = PathBuf::from("/books/b.pdf");

    store
        .set_async(&path_a, &FileReadingState { page: 1, zoom: 1.0 })
        .await
        .unwrap();
    store
        .set_async(
            &path_b,
            &FileReadingState {
                page: 99,
                zoom: 2.5,
            },
        )
        .await
        .unwrap();

    let a = store.get_async(&path_a).await.unwrap();
    assert_eq!(a.page, 1);

    let b = store.get_async(&path_b).await.unwrap();
    assert_eq!(b.page, 99);
    assert!((b.zoom - 2.5).abs() < f32::EPSILON);
}
