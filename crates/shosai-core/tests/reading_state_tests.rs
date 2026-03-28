use serial_test::serial;
use shosai_core::reading_state::{FileReadingState, ReadingStateStore};
use std::path::PathBuf;

#[test]
fn test_default_store_is_empty() {
    let store = ReadingStateStore::default();
    assert!(store.get(&PathBuf::from("/some/file.pdf")).is_none());
}

#[test]
fn test_set_and_get() {
    let mut store = ReadingStateStore::default();
    let path = PathBuf::from("/tmp/test-shosai-reading-state/test.pdf");

    store.set(&path, FileReadingState { page: 5, zoom: 1.5 });

    let state = store.get(&path);
    assert!(state.is_some(), "reading state should exist after set");
    let state = state.unwrap();
    assert_eq!(state.page, 5);
    assert!((state.zoom - 1.5).abs() < f32::EPSILON);
}

#[test]
#[serial]
fn test_save_and_load_roundtrip() {
    // Use a temp directory to avoid polluting the real config
    let tmpdir = std::env::temp_dir().join("shosai-test-reading-state");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir).unwrap();

    // SAFETY: tests using env vars are serialized via #[serial]
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &tmpdir);
    }

    let mut store = ReadingStateStore::default();
    let path = PathBuf::from("/fake/path/book.pdf");

    store.set(
        &path,
        FileReadingState {
            page: 42,
            zoom: 2.0,
        },
    );
    store.save().unwrap();

    // Load it back
    let loaded = ReadingStateStore::load().unwrap();
    let state = loaded.get(&path).expect("state should persist after load");
    assert_eq!(state.page, 42);
    assert!((state.zoom - 2.0).abs() < f32::EPSILON);

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmpdir);
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_load_nonexistent_returns_default() {
    let tmpdir = std::env::temp_dir().join("shosai-test-reading-state-empty");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir).unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &tmpdir);
    }

    let store = ReadingStateStore::load().unwrap();
    assert!(store.get(&PathBuf::from("/any/file.pdf")).is_none());

    let _ = std::fs::remove_dir_all(&tmpdir);
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
fn test_overwrite_state() {
    let mut store = ReadingStateStore::default();
    let path = PathBuf::from("/tmp/test-shosai-overwrite.pdf");

    store.set(&path, FileReadingState { page: 1, zoom: 1.0 });
    store.set(
        &path,
        FileReadingState {
            page: 10,
            zoom: 3.0,
        },
    );

    let state = store.get(&path).unwrap();
    assert_eq!(state.page, 10);
    assert!((state.zoom - 3.0).abs() < f32::EPSILON);
}
