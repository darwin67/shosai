use shosai_core::reading_state::{FileReadingState, ReadingStateStore};
use std::path::PathBuf;
use tempfile::TempDir;

/// Each test gets its own temporary directory (and therefore its own database).
/// The directory is cleaned up automatically when `TempDir` is dropped.
async fn temp_store() -> (ReadingStateStore, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("shosai.db");
    let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
    (store, dir)
}

// ---------------------------------------------------------------------------
// Basic CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_nonexistent_returns_none() {
    let (store, _dir) = temp_store().await;
    assert!(
        store
            .get_async(&PathBuf::from("/no/such/file.pdf"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn test_set_then_get() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/rust.pdf");

    store
        .set_async(&path, &FileReadingState { page: 5, zoom: 1.5 })
        .await
        .unwrap();

    let state = store
        .get_async(&path)
        .await
        .expect("should exist after set");
    assert_eq!(state.page, 5);
    assert!((state.zoom - 1.5).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_upsert_overwrites() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/overwrite.pdf");

    store
        .set_async(&path, &FileReadingState { page: 1, zoom: 1.0 })
        .await
        .unwrap();
    store
        .set_async(
            &path,
            &FileReadingState {
                page: 42,
                zoom: 3.0,
            },
        )
        .await
        .unwrap();

    let state = store.get_async(&path).await.unwrap();
    assert_eq!(state.page, 42);
    assert!((state.zoom - 3.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_multiple_files_independent() {
    let (store, _dir) = temp_store().await;
    let a = PathBuf::from("/books/a.pdf");
    let b = PathBuf::from("/books/b.pdf");

    store
        .set_async(&a, &FileReadingState { page: 1, zoom: 1.0 })
        .await
        .unwrap();
    store
        .set_async(
            &b,
            &FileReadingState {
                page: 99,
                zoom: 2.5,
            },
        )
        .await
        .unwrap();

    let sa = store.get_async(&a).await.unwrap();
    assert_eq!(sa.page, 1);
    assert!((sa.zoom - 1.0).abs() < f32::EPSILON);

    let sb = store.get_async(&b).await.unwrap();
    assert_eq!(sb.page, 99);
    assert!((sb.zoom - 2.5).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_updating_one_file_does_not_affect_another() {
    let (store, _dir) = temp_store().await;
    let a = PathBuf::from("/books/a.pdf");
    let b = PathBuf::from("/books/b.pdf");

    store
        .set_async(
            &a,
            &FileReadingState {
                page: 10,
                zoom: 1.0,
            },
        )
        .await
        .unwrap();
    store
        .set_async(
            &b,
            &FileReadingState {
                page: 20,
                zoom: 2.0,
            },
        )
        .await
        .unwrap();

    // Update only b
    store
        .set_async(
            &b,
            &FileReadingState {
                page: 50,
                zoom: 4.0,
            },
        )
        .await
        .unwrap();

    // a should be untouched
    let sa = store.get_async(&a).await.unwrap();
    assert_eq!(sa.page, 10);

    let sb = store.get_async(&b).await.unwrap();
    assert_eq!(sb.page, 50);
    assert!((sb.zoom - 4.0).abs() < f32::EPSILON);
}

// ---------------------------------------------------------------------------
// Persistence across store instances
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_data_persists_across_opens() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("shosai.db");
    let path = PathBuf::from("/books/persist.pdf");

    // Write with first instance
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

    // Read with a fresh instance
    {
        let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
        let state = store.get_async(&path).await.expect("should persist");
        assert_eq!(state.page, 42);
        assert!((state.zoom - 2.0).abs() < f32::EPSILON);
    }
}

#[tokio::test]
async fn test_preferences_persist_across_opens() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("shosai.db");

    {
        let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
        store
            .set_pref_int_async("library.cards_per_row", 6)
            .await
            .unwrap();
    }

    {
        let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
        let value = store.get_pref_int_async("library.cards_per_row").await;
        assert_eq!(value, Some(6));
    }
}

#[tokio::test]
async fn test_migrations_are_idempotent() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("shosai.db");
    let path = PathBuf::from("/books/idempotent.pdf");

    // Open and write
    let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
    store
        .set_async(
            &path,
            &FileReadingState {
                page: 7,
                zoom: 1.25,
            },
        )
        .await
        .unwrap();
    drop(store);

    // Open again — migrations run a second time but should not destroy data
    let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
    let state = store
        .get_async(&path)
        .await
        .expect("data should survive re-migration");
    assert_eq!(state.page, 7);
    assert!((state.zoom - 1.25).abs() < f32::EPSILON);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_page_zero_and_default_zoom() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/start.pdf");

    store
        .set_async(&path, &FileReadingState { page: 0, zoom: 1.0 })
        .await
        .unwrap();

    let state = store.get_async(&path).await.unwrap();
    assert_eq!(state.page, 0);
    assert!((state.zoom - 1.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_large_page_number() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/big.pdf");

    store
        .set_async(
            &path,
            &FileReadingState {
                page: 999_999,
                zoom: 5.0,
            },
        )
        .await
        .unwrap();

    let state = store.get_async(&path).await.unwrap();
    assert_eq!(state.page, 999_999);
    assert!((state.zoom - 5.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_small_zoom_value() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/books/tiny.pdf");

    store
        .set_async(
            &path,
            &FileReadingState {
                page: 0,
                zoom: 0.25,
            },
        )
        .await
        .unwrap();

    let state = store.get_async(&path).await.unwrap();
    assert!((state.zoom - 0.25).abs() < f32::EPSILON);
}

#[tokio::test]
async fn test_path_with_spaces_and_unicode() {
    let (store, _dir) = temp_store().await;
    let path = PathBuf::from("/my books/日本語の本 (copy).pdf");

    store
        .set_async(&path, &FileReadingState { page: 3, zoom: 1.5 })
        .await
        .unwrap();

    let state = store.get_async(&path).await.unwrap();
    assert_eq!(state.page, 3);
}

#[tokio::test]
async fn test_open_creates_parent_directories() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("nested").join("dirs").join("shosai.db");

    // Should create nested/dirs/ automatically
    let store = ReadingStateStore::open_at_async(&db_path).await.unwrap();
    store
        .set_async(
            &PathBuf::from("/test.pdf"),
            &FileReadingState { page: 1, zoom: 1.0 },
        )
        .await
        .unwrap();

    assert!(db_path.exists());
}
