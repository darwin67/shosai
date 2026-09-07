use shosai_core::annotations::{
    AnnotationId, AnnotationStore, AnnotationTarget, DocumentFingerprint, EpubAnchor,
    HighlightColor, ImportProvenance, MAX_ANNOTATION_BODY_SCALARS, MAX_EPUB_RESOURCE_PATH_BYTES,
    MAX_FINGERPRINT_ALGORITHM_BYTES, MAX_FINGERPRINT_BYTES, MAX_LOCAL_PATH_BYTES,
    MAX_PDF_RECTANGLES, MAX_PROVENANCE_ID_BYTES, MAX_PROVENANCE_SYSTEM_BYTES,
    MAX_QUOTE_CONTEXT_INPUT_SCALARS, MAX_QUOTE_SCALARS, NewAnnotation, PageRect, PdfAnchor,
    QuoteSelector, normalize_quote_v1, scalar_range_to_utf16,
};
use shosai_core::reading_state::ReadingStateStore;
use tempfile::TempDir;

async fn temp_store() -> (AnnotationStore, sqlx::SqlitePool, TempDir) {
    let dir = TempDir::new().unwrap();
    let state = ReadingStateStore::open_at_async(&dir.path().join("shosai.db"))
        .await
        .unwrap();
    let pool = state.pool().clone();
    (AnnotationStore::new(pool.clone()), pool, dir)
}

fn fingerprint() -> DocumentFingerprint {
    DocumentFingerprint::new("sha256", 1, vec![0xab; 32]).unwrap()
}

fn epub_annotation(book_id: Option<i64>) -> NewAnnotation {
    NewAnnotation {
        id: AnnotationId::new(),
        book_id,
        local_path: Some("/books/example.epub".into()),
        fingerprint: fingerprint(),
        quote: Some(QuoteSelector::new("Cafe\u{301}", "before ", " after").unwrap()),
        target: AnnotationTarget::Epub(EpubAnchor::new(2, "EPUB/chapter.xhtml", 10, 15).unwrap()),
        color: HighlightColor::Yellow,
        body: None,
        provenance: None,
    }
}

#[test]
fn quote_v1_golden_vectors_pin_normalization_and_context_direction() {
    assert_eq!(normalize_quote_v1("Cafe\u{301}"), "Café");
    assert_eq!(normalize_quote_v1(" a\r\n\t b\u{a0}c "), "a b c");
    assert_eq!(normalize_quote_v1("co\u{ad}operate"), "cooperate");
    assert_eq!(normalize_quote_v1("Case—A-B! ﬁ"), "Case—A-B! ﬁ");
    assert_ne!(normalize_quote_v1("Résumé"), normalize_quote_v1("résumé"));

    let selector = QuoteSelector::new(
        "selected",
        "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    )
    .unwrap();
    assert_eq!(selector.prefix, "456789ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    assert_eq!(selector.suffix, "0123456789ABCDEFGHIJKLMNOPQRSTUV");

    let selector = QuoteSelector::new("selected", &format!("{}é", "x".repeat(31)), "").unwrap();
    assert_eq!(selector.prefix, format!("{}é", "x".repeat(31)));

    let selector = QuoteSelector::new(
        "selected",
        &format!("👩‍🔬{}", "x".repeat(31)),
        &format!("{}👩‍🔬", "x".repeat(31)),
    )
    .unwrap();
    assert_eq!(selector.prefix, "x".repeat(31));
    assert_eq!(selector.suffix, "x".repeat(31));

    let selector = QuoteSelector::new(
        "selected",
        &format!("{} {}", "discarded", "p".repeat(31)),
        &format!("{} {}", "s".repeat(31), "discarded"),
    )
    .unwrap();
    assert_eq!(selector.prefix, "p".repeat(31));
    assert_eq!(selector.suffix, "s".repeat(31));
}

#[test]
fn scalar_offsets_convert_explicitly_to_utf16_units() {
    assert_eq!(scalar_range_to_utf16("A😀é", 1..3).unwrap(), 1..4);
    assert!(scalar_range_to_utf16("short", std::ops::Range { start: 4, end: 3 }).is_err());
    assert!(scalar_range_to_utf16("short", 0..6).is_err());
}

#[test]
fn annotation_inputs_enforce_exact_resource_limits_before_persistence() {
    assert!(QuoteSelector::new(&"x".repeat(MAX_QUOTE_SCALARS), "", "").is_ok());
    assert!(QuoteSelector::new(&"x".repeat(MAX_QUOTE_SCALARS + 1), "", "").is_err());
    assert!(
        QuoteSelector::new(
            "selected",
            &"x".repeat(MAX_QUOTE_CONTEXT_INPUT_SCALARS + 1),
            ""
        )
        .is_err()
    );
    assert!(DocumentFingerprint::new("sha256", 1, vec![0; MAX_FINGERPRINT_BYTES]).is_ok());
    assert!(DocumentFingerprint::new("sha256", 1, vec![0; MAX_FINGERPRINT_BYTES + 1]).is_err());
    assert!(
        DocumentFingerprint::new("x".repeat(MAX_FINGERPRINT_ALGORITHM_BYTES + 1), 1, vec![0])
            .is_err()
    );
    assert!(
        EpubAnchor::new(
            0,
            format!("{}.xhtml", "x".repeat(MAX_EPUB_RESOURCE_PATH_BYTES)),
            0,
            1
        )
        .is_err()
    );
}

#[tokio::test]
async fn exact_persisted_value_limits_round_trip() {
    let (store, pool, _dir) = temp_store().await;
    let resource_path = format!(
        "{}.xhtml",
        "x".repeat(MAX_EPUB_RESOURCE_PATH_BYTES - ".xhtml".len())
    );
    let input = NewAnnotation {
        id: AnnotationId::new(),
        book_id: None,
        local_path: Some("x".repeat(MAX_LOCAL_PATH_BYTES)),
        fingerprint: DocumentFingerprint::new(
            "x".repeat(MAX_FINGERPRINT_ALGORITHM_BYTES),
            1,
            vec![0; MAX_FINGERPRINT_BYTES],
        )
        .unwrap(),
        quote: Some(
            QuoteSelector::new(
                &"x".repeat(MAX_QUOTE_SCALARS),
                &"x".repeat(MAX_QUOTE_CONTEXT_INPUT_SCALARS),
                &"x".repeat(MAX_QUOTE_CONTEXT_INPUT_SCALARS),
            )
            .unwrap(),
        ),
        target: AnnotationTarget::Epub(EpubAnchor::new(0, resource_path, 0, 1).unwrap()),
        color: HighlightColor::Green,
        body: Some("x".repeat(MAX_ANNOTATION_BODY_SCALARS)),
        provenance: Some(ImportProvenance {
            source_system: "x".repeat(MAX_PROVENANCE_SYSTEM_BYTES),
            source_id: Some("x".repeat(MAX_PROVENANCE_ID_BYTES)),
        }),
    };

    let loaded = store.create_async(&input).await.unwrap();
    assert_eq!(loaded.body, input.body);
    assert_eq!(loaded.local_path, input.local_path);
    assert_eq!(loaded.provenance, input.provenance);

    let oversized_utf8_path = "é".repeat(MAX_LOCAL_PATH_BYTES / 2 + 1);
    assert!(
        sqlx::query("UPDATE annotations SET local_path = ? WHERE id = ?")
            .bind(oversized_utf8_path)
            .bind(input.id.to_string())
            .execute(&pool)
            .await
            .is_err(),
        "SQLite byte limits must match the Rust persistence contract"
    );
}

#[test]
fn epub_annotations_reuse_the_authoritative_canonical_path_contract() {
    for invalid in [
        "",
        "/OEBPS/chapter.xhtml",
        "OEBPS//chapter.xhtml",
        "OEBPS/./chapter.xhtml",
        "OEBPS/../chapter.xhtml",
        "OEBPS\\chapter.xhtml",
        "OEBPS/chapter.xhtml/",
        "OEBPS/\u{7f}chapter.xhtml",
    ] {
        assert!(
            EpubAnchor::new(0, invalid, 0, 1).is_err(),
            "accepted noncanonical EPUB path {invalid:?}"
        );
    }
}

#[tokio::test]
async fn epub_annotation_round_trips_and_updates() {
    let (store, pool, _dir) = temp_store().await;
    let book_id: i64 = sqlx::query_scalar(
        "INSERT INTO books (title, format, file_path) VALUES ('Example', 'epub', '/books/example.epub') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let input = epub_annotation(Some(book_id));
    let created = store.create_async(&input).await.unwrap();

    assert_eq!(created.id, input.id);
    assert_eq!(created.book_id, Some(book_id));
    assert_eq!(created.quote.as_ref().unwrap().exact, "Café");
    assert_eq!(created.target, input.target);
    assert!(created.deleted_at.is_none());

    assert!(
        store
            .update_async(&created.id, HighlightColor::Purple, Some("Remember this"))
            .await
            .unwrap()
    );
    let updated = store.get_async(&created.id, false).await.unwrap().unwrap();
    assert_eq!(updated.color, HighlightColor::Purple);
    assert_eq!(updated.body.as_deref(), Some("Remember this"));
    assert_ne!(updated.modified_at, created.modified_at);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(store.list_for_book_async(book_id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn untracked_annotations_reopen_by_device_local_path() {
    let (store, _pool, _dir) = temp_store().await;
    let mut first = epub_annotation(None);
    first.local_path = Some("device://book.epub".to_owned());
    store.create_async(&first).await.unwrap();
    let mut other = epub_annotation(None);
    other.local_path = Some("device://other.epub".to_owned());
    store.create_async(&other).await.unwrap();

    let reopened = store
        .list_for_local_path_async("device://book.epub")
        .await
        .unwrap();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].id, first.id);
}

#[tokio::test]
async fn text_and_geometry_only_pdf_annotations_round_trip() {
    let (store, pool, _dir) = temp_store().await;
    let rectangles = vec![
        PageRect::new(1.0, 2.0, 5.0, 4.0).unwrap(),
        PageRect::new(1.0, 5.0, 8.0, 7.0).unwrap(),
    ];
    let text = NewAnnotation {
        id: AnnotationId::new(),
        book_id: None,
        local_path: Some("/books/example.pdf".into()),
        fingerprint: fingerprint(),
        quote: Some(QuoteSelector::new("selected", "before", "after").unwrap()),
        target: AnnotationTarget::Pdf(
            PdfAnchor::new(3, Some((20, 28)), rectangles.clone()).unwrap(),
        ),
        color: HighlightColor::Blue,
        body: None,
        provenance: Some(ImportProvenance {
            source_system: "pdf-native".into(),
            source_id: Some("42".into()),
        }),
    };
    let created = store.create_async(&text).await.unwrap();
    assert_eq!(created.target, text.target);
    assert!(
        sqlx::query(
            "INSERT INTO annotation_pdf_rectangles
                (annotation_id, rect_index, left, bottom, right, top)
             VALUES (?, ?, 0, 0, 1, 1)"
        )
        .bind(created.id.to_string())
        .bind(i64::try_from(MAX_PDF_RECTANGLES).unwrap())
        .execute(&pool)
        .await
        .is_err(),
        "SQLite must reject rectangle indexes outside the bounded read contract"
    );

    let geometry_only = NewAnnotation {
        id: AnnotationId::new(),
        quote: None,
        target: AnnotationTarget::Pdf(PdfAnchor::new(4, None, rectangles).unwrap()),
        provenance: None,
        ..text
    };
    let loaded = store.create_async(&geometry_only).await.unwrap();
    assert!(loaded.quote.is_none());
    assert_eq!(loaded.target, geometry_only.target);
}

#[tokio::test]
async fn delete_creates_a_hidden_tombstone() {
    let (store, _pool, _dir) = temp_store().await;
    let created = store.create_async(&epub_annotation(None)).await.unwrap();

    assert!(store.delete_async(&created.id).await.unwrap());
    assert!(store.get_async(&created.id, false).await.unwrap().is_none());
    let tombstone = store.get_async(&created.id, true).await.unwrap().unwrap();
    assert!(tombstone.deleted_at.is_some());
    assert!(!store.delete_async(&created.id).await.unwrap());
}

#[tokio::test]
async fn concurrent_deletes_never_make_book_listing_fail() {
    let (store, pool, _dir) = temp_store().await;
    let book_id: i64 = sqlx::query_scalar(
        "INSERT INTO books (title, format, file_path) VALUES ('Example', 'epub', '/books/example.epub') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut ids = Vec::new();
    for _ in 0..32 {
        ids.push(
            store
                .create_async(&epub_annotation(Some(book_id)))
                .await
                .unwrap()
                .id,
        );
    }

    let listing_store = store.clone();
    let listing = tokio::spawn(async move {
        for _ in 0..32 {
            listing_store.list_for_book_async(book_id).await?;
            tokio::task::yield_now().await;
        }
        anyhow::Ok(())
    });
    let deleting_store = store.clone();
    let deleting = tokio::spawn(async move {
        for id in ids {
            deleting_store.delete_async(&id).await?;
            tokio::task::yield_now().await;
        }
        anyhow::Ok(())
    });

    listing.await.unwrap().unwrap();
    deleting.await.unwrap().unwrap();
    assert!(store.list_for_book_async(book_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_cross_format_payloads_are_rejected_before_writing() {
    let (store, pool, _dir) = temp_store().await;
    let mut epub = epub_annotation(None);
    epub.quote = None;
    assert!(store.create_async(&epub).await.is_err());

    let mut pdf = epub_annotation(None);
    pdf.target = AnnotationTarget::Pdf(
        PdfAnchor::new(0, None, vec![PageRect::new(0.0, 0.0, 1.0, 1.0).unwrap()]).unwrap(),
    );
    assert!(store.create_async(&pdf).await.is_err());
    assert!(PageRect::new(0.0, 0.0, f32::NAN, 1.0).is_err());
    assert!(EpubAnchor::new(0, "../chapter.xhtml", 0, 1).is_err());
    assert!(QuoteSelector::new("   ", "", "").is_err());

    let mut oversized = epub_annotation(None);
    oversized.local_path = Some("x".repeat(MAX_LOCAL_PATH_BYTES + 1));
    assert!(store.create_async(&oversized).await.is_err());
    oversized.local_path = None;
    oversized.body = Some("x".repeat(MAX_ANNOTATION_BODY_SCALARS + 1));
    assert!(store.create_async(&oversized).await.is_err());
    assert!(
        store
            .update_async(
                &oversized.id,
                HighlightColor::Yellow,
                Some(&"x".repeat(MAX_ANNOTATION_BODY_SCALARS + 1))
            )
            .await
            .is_err()
    );
    oversized.body = None;
    oversized.provenance = Some(ImportProvenance {
        source_system: "x".repeat(MAX_PROVENANCE_SYSTEM_BYTES + 1),
        source_id: None,
    });
    assert!(store.create_async(&oversized).await.is_err());
    oversized.provenance = Some(ImportProvenance {
        source_system: "test".into(),
        source_id: Some("x".repeat(MAX_PROVENANCE_ID_BYTES + 1)),
    });
    assert!(store.create_async(&oversized).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM annotations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "invalid inputs must be rejected before writing");
}

#[tokio::test]
async fn unknown_required_versions_fail_without_changing_the_record() {
    let (store, pool, _dir) = temp_store().await;
    let created = store.create_async(&epub_annotation(None)).await.unwrap();
    sqlx::query("UPDATE annotations SET anchor_version = 99 WHERE id = ?")
        .bind(created.id.to_string())
        .execute(&pool)
        .await
        .unwrap();

    assert!(store.get_async(&created.id, true).await.is_err());
    let version: i64 = sqlx::query_scalar("SELECT anchor_version FROM annotations WHERE id = ?")
        .bind(created.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version, 99);
}

#[tokio::test]
async fn child_insert_failure_rolls_back_the_annotation_transaction() {
    let (store, pool, _dir) = temp_store().await;
    let input = NewAnnotation {
        id: AnnotationId::new(),
        book_id: None,
        local_path: Some("/books/example.pdf".into()),
        fingerprint: fingerprint(),
        quote: None,
        target: AnnotationTarget::Pdf(
            PdfAnchor::new(0, None, vec![PageRect::new(0.0, 0.0, 1.0, 1.0).unwrap()]).unwrap(),
        ),
        color: HighlightColor::Pink,
        body: None,
        provenance: None,
    };
    sqlx::query(&format!(
        "CREATE TRIGGER reject_test_rectangle BEFORE INSERT ON annotation_pdf_rectangles
         WHEN NEW.annotation_id = '{}'
         BEGIN SELECT RAISE(ABORT, 'test rejection'); END",
        input.id
    ))
    .execute(&pool)
    .await
    .unwrap();

    assert!(store.create_async(&input).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM annotations WHERE id = ?")
        .bind(input.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
