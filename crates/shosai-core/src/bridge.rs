//! Owned, coarse-grained API suitable for a generated Dart/Rust bridge.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use thiserror::Error;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

use crate::annotations::{
    ANNOTATION_SNAPSHOT_BASE_BYTES, Annotation, AnnotationId, AnnotationResolution,
    AnnotationSnapshotLimit, AnnotationStore, AnnotationTarget, DocumentFingerprint, EpubAnchor,
    HighlightColor, MAX_ANNOTATION_BODY_SCALARS, MAX_ANNOTATION_SNAPSHOT_BYTES,
    MAX_TEXT_ANCHOR_RESOLUTION_WORK, NewAnnotation, PageRect, PdfAnchor, QuoteSelector,
    TextAnchorResolutionError, TextAnchorResolver, TextScalarIndex,
};
#[cfg(test)]
use crate::annotations::{AnnotationPersistenceTestGate, MAX_ANNOTATIONS_PER_SNAPSHOT};

use crate::application::{DeviceFileLocator, OpenDocument, OpenDocumentError, OpenDocumentPlan};
use crate::document::{Document, RenderedPage};
#[cfg(test)]
use crate::epub::EpubLimits;
use crate::epub::{
    EPUB_TEXT_MAX_ENDPOINTS, EPUB_TEXT_MAX_PIXELS, EPUB_TEXT_MAX_SCALARS, EpubTextAlign,
    EpubTextDirection, EpubTextEndpoint, EpubTextRequest, EpubTextRun,
};
use crate::library::BookFormat;
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_BRIDGE_BUFFER_BYTES: usize = 160 * 1024 * 1024;
pub const MAX_BRIDGE_RETAINED_BUFFER_BYTES: usize = 320 * 1024 * 1024;
pub const MAX_BRIDGE_RENDER_WORKERS: usize = 2;
pub const MAX_BRIDGE_OPEN_WORKERS: usize = 2;
pub const MAX_BRIDGE_REQUESTS: usize = 64;
pub const MAX_BRIDGE_DOCUMENTS: usize = 64;
pub const MAX_BRIDGE_BUFFERS: usize = 256;
pub const MAX_BRIDGE_RETAINED_DOCUMENT_BYTES: usize = 3 * 1024 * 1024 * 1024;
pub const MAX_BRIDGE_PROBE_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_BRIDGE_LOCAL_ID_BYTES: usize = 4 * 1024;
pub const MAX_BRIDGE_PATH_KEY_BYTES: usize = 64 * 1024;
// The resolver retains its usize/range indexes plus source/profile/normalized
// text at peak. Limiting PDF text to 2 MiB keeps the conservative 144 MiB
// reservation below the process-wide transient buffer budget on 64-bit hosts.
const MAX_ANNOTATION_PDF_TEXT_BYTES: usize = 2 * 1024 * 1024;
const ANNOTATION_RESOLUTION_WORKSPACE_BYTES: u32 = 144 * 1024 * 1024;
const ANNOTATION_GEOMETRY_WORKSPACE_BYTES: u32 = 8 * 1024 * 1024;

static NEXT_REGISTRY_ID: AtomicU64 = AtomicU64::new(1);

/// Fixed-field request that can be generated directly into a Dart value.
#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub book_id: Option<i64>,
    pub local_id: String,
    pub path_key: String,
    pub format_hint: Option<BookFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentHandle {
    pub registry: u64,
    pub id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferHandle {
    pub registry: u64,
    pub id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectionHandle {
    pub registry: u64,
    pub id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalUnit {
    Page,
    Chapter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSummary {
    pub handle: DocumentHandle,
    pub book_id: Option<i64>,
    pub format: BookFormat,
    pub title: Option<String>,
    pub logical_unit: LogicalUnit,
    pub logical_unit_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    pub document: DocumentHandle,
    pub page: usize,
    pub scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderedBuffer {
    pub handle: BufferHandle,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionEndpoint {
    pub offset: usize,
    pub range_start: usize,
    pub range_end: usize,
    pub rect: SelectionRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionPageRect {
    pub character: usize,
    pub rect: SelectionRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionCaret {
    /// Logical scalar/PDFium-character boundary represented by this caret.
    pub offset: usize,
    pub x: f32,
    pub along_line: f32,
    pub vertical: bool,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionVisualLine {
    /// Carets in visual (left-to-right) order. The same offset may occur on two
    /// wrapped lines; its line membership preserves upstream/downstream affinity.
    pub carets: Vec<SelectionCaret>,
}

/// Owned visible-surface text and hit zones. Pointer movement consumes this
/// value locally and never re-enters PDFium or Rust.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSurface {
    /// Retains request and geometry admission until the host discards the surface.
    pub handle: SelectionHandle,
    pub width: f32,
    pub height: f32,
    pub text: String,
    /// Rust-owned completeness decision. False disables copying and makes PDF
    /// persistence omit its text range and quote selector.
    pub copy_eligible: bool,
    pub resource_path: Option<String>,
    /// Retained straight-alpha RGBA raster produced by the same EPUB layout.
    /// The caller owns this handle and must release it after decoding.
    pub raster: Option<RenderedBuffer>,
    pub endpoints: Vec<SelectionEndpoint>,
    /// All legal extended-grapheme caret offsets, in logical order.
    pub grapheme_boundaries: Vec<usize>,
    /// UAX #29 word-segment stops, including punctuation boundaries.
    pub word_boundaries: Vec<usize>,
    /// Renderer/extractor-produced visual line membership and caret geometry.
    pub visual_lines: Vec<SelectionVisualLine>,
    /// Durable PDF page-coordinate character rectangles; empty for EPUB.
    pub page_rectangles: Vec<SelectionPageRect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAnnotation {
    pub id: String,
    pub unit: usize,
    pub resolution: AnnotationResolution,
    /// Text-backed half-open range. Geometry-only PDF annotations have no range.
    pub text_range: Option<AnnotationTextRange>,
    /// Normalized exact quote when the source text mapping was complete.
    pub quote: Option<String>,
    /// Rust-produced PDF display geometry; empty for EPUB annotations.
    pub rectangles: Vec<SelectionRect>,
    pub color: HighlightColor,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationTextRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct CreateAnnotationRequest {
    pub document: DocumentHandle,
    pub unit: usize,
    pub start: usize,
    pub end: usize,
    pub display_scale: f32,
    pub color: HighlightColor,
    pub body: Option<String>,
}

#[derive(Debug, Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
    publication: Mutex<()>,
}

#[derive(Debug, Clone, Default)]
pub struct Cancellation(Arc<CancellationInner>);

impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        let _publication = self
            .0
            .publication
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.0.cancelled.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeErrorKind {
    Cancelled,
    NotFound,
    Inaccessible,
    Unsupported,
    InvalidRequest,
    Malformed,
    LimitExceeded,
    BackendUnavailable,
    RenderFailed,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BridgeError {
    #[error("operation was cancelled")]
    Cancelled,
    #[error("unknown, foreign, or released document handle")]
    InvalidDocumentHandle,
    #[error("unknown, foreign, or released buffer handle")]
    InvalidBufferHandle,
    #[error("document was not found")]
    DocumentNotFound,
    #[error("document is inaccessible")]
    DocumentInaccessible,
    #[error("operation is unsupported for {0}")]
    UnsupportedOperation(BookFormat),
    #[error("unsupported file format: .{0}")]
    UnsupportedFormat(String),
    #[error("invalid page {page}; document has {page_count} pages")]
    InvalidPage { page: usize, page_count: usize },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("failed to open {format}: {detail}")]
    Open { format: BookFormat, detail: String },
    #[error("{format} exceeds an opening resource limit: {detail}")]
    OpenLimit { format: BookFormat, detail: String },
    #[error("{format} backend is unavailable: {detail}")]
    Backend { format: BookFormat, detail: String },
    #[error("document render failed: {0}")]
    Render(String),
    #[error("bridge buffer exceeds its memory budget")]
    BufferLimit,
    #[error("bridge document exceeds its retention budget")]
    DocumentLimit,
    #[error("bridge request count exceeds its admission limit")]
    RequestLimit,
    #[error("bridge buffer count exceeds its retention limit")]
    BufferCountLimit,
    #[error("annotation snapshot exceeds its retention limit")]
    AnnotationLimit,
    #[error("Rust operation panicked")]
    Panic,
    #[error("bridge worker stopped unexpectedly")]
    Worker,
    #[error("annotation storage failed: {0}")]
    Storage(String),
}

impl BridgeError {
    pub fn kind(&self) -> BridgeErrorKind {
        match self {
            Self::Cancelled => BridgeErrorKind::Cancelled,
            Self::InvalidDocumentHandle | Self::InvalidBufferHandle | Self::DocumentNotFound => {
                BridgeErrorKind::NotFound
            }
            Self::DocumentInaccessible => BridgeErrorKind::Inaccessible,
            Self::BufferLimit
            | Self::DocumentLimit
            | Self::RequestLimit
            | Self::BufferCountLimit
            | Self::AnnotationLimit
            | Self::OpenLimit { .. } => BridgeErrorKind::LimitExceeded,
            Self::Panic | Self::Worker | Self::Backend { .. } | Self::Storage(_) => {
                BridgeErrorKind::BackendUnavailable
            }
            Self::Render(_) => BridgeErrorKind::RenderFailed,
            Self::UnsupportedOperation(_) | Self::UnsupportedFormat(_) => {
                BridgeErrorKind::Unsupported
            }
            Self::InvalidPage { .. } | Self::InvalidRequest(_) => BridgeErrorKind::InvalidRequest,
            Self::Open { .. } => BridgeErrorKind::Malformed,
        }
    }
}

#[derive(Debug)]
struct RetainedBuffer {
    pixels: Vec<u8>,
    transferred: bool,
    _bytes: OwnedSemaphorePermit,
    _slot: OwnedSemaphorePermit,
}

#[derive(Debug)]
struct RetainedDocument {
    document: OpenDocument,
    local_path: String,
    fingerprint: DocumentFingerprint,
    _bytes: OwnedSemaphorePermit,
    _slot: OwnedSemaphorePermit,
}

#[derive(Debug)]
struct RetainedSelection {
    _request_slot: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

#[derive(Debug, Default)]
struct Registry {
    documents: HashMap<DocumentHandle, Arc<RetainedDocument>>,
    buffers: HashMap<BufferHandle, RetainedBuffer>,
    selections: HashMap<SelectionHandle, RetainedSelection>,
}

#[derive(Debug)]
struct BridgeAdmission {
    request_slots: Arc<Semaphore>,
    render_slots: Arc<Semaphore>,
    buffer_bytes: Arc<Semaphore>,
    planning_slots: Arc<Semaphore>,
    open_slots: Arc<Semaphore>,
    document_slots: Arc<Semaphore>,
    buffer_slots: Arc<Semaphore>,
    document_bytes: Arc<Semaphore>,
    probe_bytes: Arc<Semaphore>,
    buffer_capacity: usize,
}

#[cfg(test)]
#[derive(Debug)]
struct TestPhaseGate {
    entered: Semaphore,
    release: Semaphore,
}

#[cfg(test)]
impl Default for TestPhaseGate {
    fn default() -> Self {
        Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }
}

#[cfg(test)]
impl TestPhaseGate {
    async fn pause(&self) {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
    }

    async fn wait_until_entered(&self) {
        self.entered.acquire().await.unwrap().forget();
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct AnnotationTestHooks {
    initialization: Option<Arc<TestPhaseGate>>,
    before_acceptance: Option<Arc<TestPhaseGate>>,
    persistence: Option<Arc<AnnotationPersistenceTestGate>>,
    list: Option<Arc<AnnotationPersistenceTestGate>>,
    fail_create_response: bool,
}

impl BridgeAdmission {
    fn new(buffer_bytes: usize, render_workers: usize) -> Self {
        Self {
            request_slots: Arc::new(Semaphore::new(MAX_BRIDGE_REQUESTS)),
            render_slots: Arc::new(Semaphore::new(render_workers)),
            buffer_bytes: Arc::new(Semaphore::new(buffer_bytes)),
            planning_slots: Arc::new(Semaphore::new(MAX_BRIDGE_OPEN_WORKERS)),
            open_slots: Arc::new(Semaphore::new(MAX_BRIDGE_OPEN_WORKERS)),
            document_slots: Arc::new(Semaphore::new(MAX_BRIDGE_DOCUMENTS)),
            buffer_slots: Arc::new(Semaphore::new(MAX_BRIDGE_BUFFERS)),
            document_bytes: Arc::new(Semaphore::new(MAX_BRIDGE_RETAINED_DOCUMENT_BYTES)),
            probe_bytes: Arc::new(Semaphore::new(MAX_BRIDGE_PROBE_BYTES)),
            buffer_capacity: buffer_bytes,
        }
    }
}

static GLOBAL_ADMISSION: OnceLock<Arc<BridgeAdmission>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Bridge {
    registry_id: u64,
    next_handle: Arc<AtomicU64>,
    registry: Arc<Mutex<Registry>>,
    admission: Arc<BridgeAdmission>,
    annotation_store: Arc<tokio::sync::OnceCell<AnnotationStore>>,
    annotation_database: Option<Arc<PathBuf>>,
    #[cfg(test)]
    selection_worker_barrier: Option<Arc<std::sync::Barrier>>,
    #[cfg(test)]
    selection_second_cancellation_barrier: Option<Arc<std::sync::Barrier>>,
    #[cfg(test)]
    annotation_resolution_worker_barrier: Option<Arc<std::sync::Barrier>>,
    #[cfg(test)]
    annotation_test_hooks: Option<Arc<AnnotationTestHooks>>,
}

impl Default for Bridge {
    fn default() -> Self {
        Self::new()
    }
}

impl Bridge {
    pub fn new() -> Self {
        let admission = Arc::clone(GLOBAL_ADMISSION.get_or_init(|| {
            Arc::new(BridgeAdmission::new(
                MAX_BRIDGE_RETAINED_BUFFER_BYTES,
                MAX_BRIDGE_RENDER_WORKERS,
            ))
        }));
        Self::with_admission(admission)
    }

    /// Construct a bridge whose annotation storage uses a host-owned SQLite path.
    ///
    /// Production hosts may keep using [`Self::new`]. Tests and platform hosts
    /// that own application-data placement can inject the exact database path.
    pub fn with_database_path(database: PathBuf) -> Self {
        let admission = Arc::clone(GLOBAL_ADMISSION.get_or_init(|| {
            Arc::new(BridgeAdmission::new(
                MAX_BRIDGE_RETAINED_BUFFER_BYTES,
                MAX_BRIDGE_RENDER_WORKERS,
            ))
        }));
        Self::with_admission_database(admission, Some(Arc::new(database)))
    }

    #[cfg(test)]
    fn with_limits(buffer_bytes: usize, render_workers: usize) -> Self {
        Self::with_admission(Arc::new(BridgeAdmission::new(buffer_bytes, render_workers)))
    }

    fn with_admission(admission: Arc<BridgeAdmission>) -> Self {
        Self::with_admission_database(admission, None)
    }

    fn with_admission_database(
        admission: Arc<BridgeAdmission>,
        annotation_database: Option<Arc<PathBuf>>,
    ) -> Self {
        Self {
            registry_id: NEXT_REGISTRY_ID.fetch_add(1, Ordering::Relaxed),
            next_handle: Arc::new(AtomicU64::new(0)),
            registry: Arc::new(Mutex::new(Registry::default())),
            admission,
            annotation_store: Arc::new(tokio::sync::OnceCell::new()),
            annotation_database,
            #[cfg(test)]
            selection_worker_barrier: None,
            #[cfg(test)]
            selection_second_cancellation_barrier: None,
            #[cfg(test)]
            annotation_resolution_worker_barrier: None,
            #[cfg(test)]
            annotation_test_hooks: None,
        }
    }

    pub async fn open_document(
        &self,
        request: OpenRequest,
        cancellation: Cancellation,
    ) -> Result<DocumentSummary, BridgeError> {
        check_cancelled(&cancellation)?;
        let _request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        if request.local_id.len() > MAX_BRIDGE_LOCAL_ID_BYTES {
            return Err(BridgeError::InvalidRequest(format!(
                "local_id exceeds {MAX_BRIDGE_LOCAL_ID_BYTES} bytes"
            )));
        }
        if request.path_key.len() > MAX_BRIDGE_PATH_KEY_BYTES {
            return Err(BridgeError::InvalidRequest(format!(
                "path_key exceeds {MAX_BRIDGE_PATH_KEY_BYTES} bytes"
            )));
        }
        if request.book_id.is_some() {
            return Err(BridgeError::InvalidRequest(
                "book_id requires a library-backed resolver; use an untracked locator".to_owned(),
            ));
        }
        let path = crate::path_key::try_path_from_key(&request.path_key).map_err(|_| {
            BridgeError::InvalidRequest("path_key uses an invalid reserved encoding".to_owned())
        })?;
        let mut locator = DeviceFileLocator::new(request.local_id, path);
        if let Some(format) = request.format_hint {
            locator = locator.with_format_hint(format);
        }
        let document_slot =
            acquire_permits(Arc::clone(&self.admission.document_slots), 1, &cancellation).await?;
        let planning_slot =
            acquire_permits(Arc::clone(&self.admission.planning_slots), 1, &cancellation).await?;
        let planning_cancellation = cancellation.clone();
        let (plan, guards) = tokio::task::spawn_blocking(move || {
            let plan = guarded(|| {
                OpenDocumentPlan::prepare_cancellable(&locator, &planning_cancellation)
                    .map_err(map_open_error)
            });
            (plan, (_request_slot, document_slot, planning_slot))
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        let (_request_slot, document_slot, planning_slot) = guards;
        check_cancelled(&cancellation)?;
        let plan = plan?;
        drop(planning_slot);
        let maximum_retained_bytes = plan
            .retained_admission_byte_len()
            .filter(|bytes| *bytes <= MAX_BRIDGE_RETAINED_DOCUMENT_BYTES)
            .ok_or(BridgeError::DocumentLimit)?;
        let maximum_byte_permits =
            u32::try_from(maximum_retained_bytes).map_err(|_| BridgeError::DocumentLimit)?;
        let document_bytes = acquire_permits(
            Arc::clone(&self.admission.document_bytes),
            maximum_byte_permits,
            &cancellation,
        )
        .await?;
        let open_slot =
            acquire_permits(Arc::clone(&self.admission.open_slots), 1, &cancellation).await?;
        let worker_cancellation = cancellation.clone();
        let (opened, guards) = tokio::task::spawn_blocking(move || {
            let document = guarded(|| {
                plan.open_with_content_hash_cancellable(worker_cancellation)
                    .map_err(map_open_error)
            });
            (
                document,
                (_request_slot, document_slot, document_bytes, open_slot),
            )
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        let (_request_slot, document_slot, mut document_bytes, open_slot) = guards;
        check_cancelled(&cancellation)?;
        let (document, content_hash) = opened?;
        let actual_retained_bytes = document
            .retained_byte_len()
            .ok_or(BridgeError::DocumentLimit)?;
        if actual_retained_bytes > maximum_retained_bytes {
            return Err(BridgeError::DocumentLimit);
        }
        let unused_bytes = maximum_retained_bytes - actual_retained_bytes;
        drop(document_bytes.split(unused_bytes));
        drop(open_slot);
        let _publication = cancellation
            .0
            .publication
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        check_cancelled(&cancellation)?;

        let handle = self.document_handle();
        let format = document.format();
        let summary = DocumentSummary {
            handle,
            book_id: request.book_id,
            format,
            title: document.title(),
            logical_unit: if format == BookFormat::Epub {
                LogicalUnit::Chapter
            } else {
                LogicalUnit::Page
            },
            logical_unit_count: document.page_count(),
        };
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .documents
            .insert(
                handle,
                Arc::new(RetainedDocument {
                    document,
                    local_path: request.path_key,
                    fingerprint: DocumentFingerprint::new(
                        "sha256-hex",
                        1,
                        content_hash.into_bytes(),
                    )
                    .map_err(storage_error)?,
                    _bytes: document_bytes,
                    _slot: document_slot,
                }),
            );
        Ok(summary)
    }

    async fn annotation_store(&self) -> Result<&AnnotationStore, BridgeError> {
        self.annotation_store
            .get_or_try_init(|| async {
                #[cfg(test)]
                if let Some(gate) = self
                    .annotation_test_hooks
                    .as_ref()
                    .and_then(|hooks| hooks.initialization.as_ref())
                {
                    gate.pause().await;
                }
                let state = match self.annotation_database.as_deref() {
                    Some(path) => {
                        crate::reading_state::ReadingStateStore::open_at_async_deferred_backfill(
                            path,
                        )
                        .await
                    }
                    None => {
                        crate::reading_state::ReadingStateStore::open_async_deferred_backfill()
                            .await
                    }
                }
                .map_err(|error| BridgeError::Storage(error.to_string()))?;
                #[cfg(test)]
                return Ok(AnnotationStore::new_with_test_gates(
                    state.pool().clone(),
                    self.annotation_test_hooks
                        .as_ref()
                        .and_then(|hooks| hooks.persistence.clone()),
                    self.annotation_test_hooks
                        .as_ref()
                        .and_then(|hooks| hooks.list.clone()),
                    self.annotation_test_hooks
                        .as_ref()
                        .is_some_and(|hooks| hooks.fail_create_response),
                ));
                #[cfg(not(test))]
                Ok(AnnotationStore::new(state.pool().clone()))
            })
            .await
    }

    pub async fn create_annotation(
        &self,
        request: CreateAnnotationRequest,
        cancellation: Cancellation,
    ) -> Result<BridgeAnnotation, BridgeError> {
        let CreateAnnotationRequest {
            document,
            unit,
            start,
            end,
            display_scale,
            color,
            body,
        } = request;
        validate_annotation_body(body.as_deref())?;
        if start >= end {
            return Err(BridgeError::InvalidRequest(
                "annotation range must be non-empty".into(),
            ));
        }
        if !display_scale.is_finite() || display_scale <= 0.0 {
            return Err(BridgeError::InvalidRequest(
                "annotation display scale must be finite and positive".into(),
            ));
        }
        let request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let retained = self.document(document)?;
        check_cancelled(&cancellation)?;
        let store = tokio::select! {
            store = self.annotation_store() => store?,
            () = cancellation.cancelled() => return Err(BridgeError::Cancelled),
        };
        let extraction_scale = if matches!(&retained.document, OpenDocument::Pdf(_)) {
            display_scale
        } else {
            1.0
        };
        let extracted = self
            .extract_selection(
                document,
                Arc::clone(&retained),
                unit,
                extraction_scale,
                680.0,
                18.0,
                false,
                &cancellation,
                request_slot,
            )
            .await?;
        let surface = &extracted.surface;
        let chars: Vec<char> = surface.text.chars().collect();
        if end > chars.len() {
            return Err(BridgeError::InvalidRequest(
                "annotation range exceeds text".into(),
            ));
        }
        let quote = QuoteSelector::new(
            &chars[start..end].iter().collect::<String>(),
            &chars[..start].iter().collect::<String>(),
            &chars[end..].iter().collect::<String>(),
        )
        .map_err(storage_error)?;
        let (target, quote) = match &retained.document {
            OpenDocument::Epub(_) => (
                AnnotationTarget::Epub(
                    EpubAnchor::new(
                        u32::try_from(unit).map_err(|_| {
                            BridgeError::InvalidRequest("unit exceeds range".into())
                        })?,
                        surface.resource_path.as_deref().ok_or_else(|| {
                            BridgeError::InvalidRequest("EPUB resource path missing".into())
                        })?,
                        u32::try_from(start).map_err(|_| {
                            BridgeError::InvalidRequest("range exceeds range".into())
                        })?,
                        u32::try_from(end).map_err(|_| {
                            BridgeError::InvalidRequest("range exceeds range".into())
                        })?,
                    )
                    .map_err(storage_error)?,
                ),
                Some(quote),
            ),
            OpenDocument::Pdf(_) => {
                let rectangles = surface
                    .page_rectangles
                    .iter()
                    .copied()
                    .filter(|value| start <= value.character && value.character < end)
                    .map(|value| {
                        PageRect::new(
                            value.rect.left,
                            value.rect.top,
                            value.rect.right,
                            value.rect.bottom,
                        )
                        .map_err(storage_error)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let complete = surface.copy_eligible;
                (
                    AnnotationTarget::Pdf(
                        PdfAnchor::new(
                            u32::try_from(unit).map_err(|_| {
                                BridgeError::InvalidRequest("unit exceeds range".into())
                            })?,
                            complete.then(|| {
                                (
                                    u32::try_from(start).expect("bounded PDF start"),
                                    u32::try_from(end).expect("bounded PDF end"),
                                )
                            }),
                            rectangles,
                        )
                        .map_err(storage_error)?,
                    ),
                    complete.then_some(quote),
                )
            }
            OpenDocument::Cbz(_) => return Err(BridgeError::UnsupportedOperation(BookFormat::Cbz)),
        };
        let annotation = NewAnnotation {
            id: AnnotationId::new(),
            book_id: None,
            local_path: Some(retained.local_path.clone()),
            fingerprint: retained.fingerprint.clone(),
            quote,
            target,
            color,
            body,
            provenance: None,
        };
        drop(chars);
        let ExtractedSelection {
            surface,
            request_slot,
            retained_bytes,
        } = extracted;
        drop(surface);
        drop(retained_bytes);
        let conversion_slot =
            acquire_permits(Arc::clone(&self.admission.render_slots), 1, &cancellation).await?;
        let pending = Annotation {
            id: annotation.id.clone(),
            book_id: annotation.book_id,
            local_path: annotation.local_path.clone(),
            fingerprint: annotation.fingerprint.clone(),
            quote: annotation.quote.clone(),
            target: annotation.target.clone(),
            color: annotation.color,
            body: annotation.body.clone(),
            provenance: annotation.provenance.clone(),
            created_at: String::new(),
            modified_at: String::new(),
            deleted_at: None,
        };
        let (mut prepared, response_guards) = self
            .resolve_annotation_dtos(
                Arc::clone(&retained),
                vec![pending],
                display_scale,
                cancellation.clone(),
                vec![request_slot, conversion_slot],
                false,
            )
            .await?;
        let prepared = prepared.remove(0);
        #[cfg(test)]
        if let Some(gate) = self
            .annotation_test_hooks
            .as_ref()
            .and_then(|hooks| hooks.before_acceptance.as_ref())
        {
            gate.pause().await;
        }
        // This is the persistence acceptance boundary: cancellation takes the
        // same lock, so no write can begin after cancellation has won.
        {
            let _publication = cancellation
                .0
                .publication
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            check_cancelled(&cancellation)?;
        }
        store
            .create_async(&annotation)
            .await
            .map_err(annotation_storage_error)?;
        drop(response_guards);
        Ok(prepared)
    }

    pub async fn list_annotations(
        &self,
        document: DocumentHandle,
        scale: f32,
        cancellation: Cancellation,
    ) -> Result<Vec<BridgeAnnotation>, BridgeError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(BridgeError::InvalidRequest(
                "annotation display scale must be finite and positive".into(),
            ));
        }
        let request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let retained = self.document(document)?;
        let store = tokio::select! {
            store = self.annotation_store() => store?,
            () = cancellation.cancelled() => return Err(BridgeError::Cancelled),
        };
        check_cancelled(&cancellation)?;
        let listed = tokio::select! {
            result = store.list_for_local_document_async(
                &retained.local_path,
                Some(&retained.fingerprint),
            ) => {
                check_cancelled(&cancellation)?;
                result.map_err(annotation_storage_error)?
            },
            () = cancellation.cancelled() => return Err(BridgeError::Cancelled),
        };
        let items = bounded_annotation_snapshot(
            listed
                .into_iter()
                .filter(|item| item.fingerprint == retained.fingerprint)
                .collect(),
        )?;
        let render_slot =
            acquire_permits(Arc::clone(&self.admission.render_slots), 1, &cancellation).await?;
        self.resolve_annotation_dtos(
            retained,
            items,
            scale,
            cancellation,
            vec![request_slot, render_slot],
            true,
        )
        .await
        .map(|(items, _guards)| items)
    }

    async fn resolve_annotation_dtos(
        &self,
        retained: Arc<RetainedDocument>,
        annotations: Vec<Annotation>,
        scale: f32,
        cancellation: Cancellation,
        mut guards: Vec<OwnedSemaphorePermit>,
        resolve_persisted_text: bool,
    ) -> Result<(Vec<BridgeAnnotation>, Vec<OwnedSemaphorePermit>), BridgeError> {
        if annotations.is_empty() {
            return Ok((Vec::new(), guards));
        }
        let workspace_bytes = if resolve_persisted_text
            && annotations.iter().any(|annotation| {
                matches!(&annotation.target, AnnotationTarget::Epub(_))
                    || matches!(
                        &annotation.target,
                        AnnotationTarget::Pdf(anchor) if anchor.character_range.is_some()
                    )
            }) {
            ANNOTATION_RESOLUTION_WORKSPACE_BYTES
        } else {
            ANNOTATION_GEOMETRY_WORKSPACE_BYTES
        };
        if workspace_bytes as usize > self.admission.buffer_capacity {
            return Err(BridgeError::BufferLimit);
        }
        if workspace_bytes != 0 {
            guards.push(
                acquire_permits(
                    Arc::clone(&self.admission.buffer_bytes),
                    workspace_bytes,
                    &cancellation,
                )
                .await?,
            );
        }
        let worker_cancellation = cancellation.clone();
        #[cfg(test)]
        let worker_barrier = self.annotation_resolution_worker_barrier.clone();
        let (result, guards) = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(barrier) = worker_barrier {
                barrier.wait();
                barrier.wait();
            }
            let result = guarded(|| {
                annotation_dtos(
                    annotations,
                    &retained.document,
                    scale,
                    resolve_persisted_text,
                    &|| worker_cancellation.is_cancelled(),
                )
            });
            (result, guards)
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        check_cancelled(&cancellation)?;
        Ok((result?, guards))
    }

    pub async fn update_annotation(
        &self,
        document: DocumentHandle,
        id: &str,
        color: HighlightColor,
        body: Option<String>,
    ) -> Result<bool, BridgeError> {
        validate_annotation_body(body.as_deref())?;
        let _request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let retained = self.document(document)?;
        let id = AnnotationId::from_str(id)
            .map_err(|_| BridgeError::InvalidRequest("invalid annotation ID".into()))?;
        if !self
            .annotation_belongs_to(&id, &retained.local_path, &retained.fingerprint)
            .await?
        {
            return Ok(false);
        }
        self.annotation_store()
            .await?
            .update_async(&id, color, body.as_deref())
            .await
            .map_err(annotation_storage_error)
    }

    pub async fn delete_annotation(
        &self,
        document: DocumentHandle,
        id: &str,
    ) -> Result<bool, BridgeError> {
        let _request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let retained = self.document(document)?;
        let id = AnnotationId::from_str(id)
            .map_err(|_| BridgeError::InvalidRequest("invalid annotation ID".into()))?;
        if !self
            .annotation_belongs_to(&id, &retained.local_path, &retained.fingerprint)
            .await?
        {
            return Ok(false);
        }
        self.annotation_store()
            .await?
            .delete_async(&id)
            .await
            .map_err(storage_error)
    }

    async fn annotation_belongs_to(
        &self,
        id: &AnnotationId,
        local_path: &str,
        fingerprint: &DocumentFingerprint,
    ) -> Result<bool, BridgeError> {
        self.annotation_store()
            .await?
            .get_async(id, false)
            .await
            .map_err(storage_error)
            .map(|annotation| {
                annotation.is_some_and(|value| {
                    value.book_id.is_none()
                        && value.local_path.as_deref() == Some(local_path)
                        && value.fingerprint == *fingerprint
                })
            })
    }

    pub async fn render_page(
        &self,
        request: RenderRequest,
        cancellation: Cancellation,
    ) -> Result<RenderedBuffer, BridgeError> {
        check_cancelled(&cancellation)?;
        let _request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let buffer_slot = try_acquire_slot(
            Arc::clone(&self.admission.buffer_slots),
            BridgeError::BufferCountLimit,
        )?;
        if !request.scale.is_finite() || request.scale <= 0.0 {
            return Err(BridgeError::InvalidRequest(
                "render scale must be finite and positive".to_owned(),
            ));
        }
        let retained_document = self.document(request.document)?;
        let probe_byte_len = render_probe_byte_len(&retained_document.document, request.page)?;
        let probe_byte_permits =
            u32::try_from(probe_byte_len).map_err(|_| BridgeError::BufferLimit)?;
        let render_slot =
            acquire_permits(Arc::clone(&self.admission.render_slots), 1, &cancellation).await?;
        let probe_bytes = acquire_permits(
            Arc::clone(&self.admission.probe_bytes),
            probe_byte_permits,
            &cancellation,
        )
        .await?;
        let preflight_document = Arc::clone(&retained_document);
        let page = request.page;
        let scale = request.scale;
        let preflight_cancellation = cancellation.clone();
        let (byte_len, guards) = tokio::task::spawn_blocking(move || {
            let byte_len = guarded(|| {
                render_byte_len(&preflight_document.document, page, scale, &|| {
                    preflight_cancellation.is_cancelled()
                })
            });
            (
                byte_len,
                (_request_slot, buffer_slot, render_slot, probe_bytes),
            )
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        let (_request_slot, buffer_slot, render_slot, probe_bytes) = guards;
        check_cancelled(&cancellation)?;
        let byte_len = byte_len?;
        if byte_len > MAX_BRIDGE_BUFFER_BYTES {
            return Err(BridgeError::BufferLimit);
        }
        let render_transient_byte_len = match &retained_document.document {
            OpenDocument::Pdf(_) => byte_len,
            OpenDocument::Cbz(document) => document
                .render_admission_byte_len_at_scale(request.page, request.scale)
                .ok_or(BridgeError::BufferLimit)?,
            OpenDocument::Epub(_) => {
                return Err(BridgeError::UnsupportedOperation(BookFormat::Epub));
            }
        };
        drop(probe_bytes);
        if render_transient_byte_len > MAX_BRIDGE_PROBE_BYTES {
            return Err(BridgeError::BufferLimit);
        }
        let render_transient_permits =
            u32::try_from(render_transient_byte_len).map_err(|_| BridgeError::BufferLimit)?;
        let render_transient_bytes = acquire_permits(
            Arc::clone(&self.admission.probe_bytes),
            render_transient_permits,
            &cancellation,
        )
        .await?;
        let transfer_peak = byte_len.checked_mul(2).ok_or(BridgeError::BufferLimit)?;
        let byte_permits = u32::try_from(transfer_peak).map_err(|_| BridgeError::BufferLimit)?;
        let buffer_bytes = acquire_permits(
            Arc::clone(&self.admission.buffer_bytes),
            byte_permits,
            &cancellation,
        )
        .await?;
        check_cancelled(&cancellation)?;
        let render_cancellation = cancellation.clone();
        let (rendered, guards) = tokio::task::spawn_blocking(move || {
            let rendered = guarded(|| {
                render(
                    retained_document.document.clone(),
                    request.page,
                    request.scale,
                    &|| render_cancellation.is_cancelled(),
                )
            });
            (
                rendered,
                (
                    _request_slot,
                    buffer_slot,
                    render_slot,
                    render_transient_bytes,
                    buffer_bytes,
                ),
            )
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        let (_request_slot, buffer_slot, render_slot, render_transient_bytes, buffer_bytes) =
            guards;
        check_cancelled(&cancellation)?;
        let rendered = rendered?;
        drop(render_slot);
        drop(render_transient_bytes);
        let _publication = cancellation
            .0
            .publication
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        check_cancelled(&cancellation)?;
        self.store_buffer(request.document, rendered, buffer_bytes, buffer_slot)
    }

    pub async fn selection_surface(
        &self,
        document: DocumentHandle,
        unit: usize,
        scale: f32,
        width: f32,
        font_size: f32,
        cancellation: Cancellation,
    ) -> Result<SelectionSurface, BridgeError> {
        check_cancelled(&cancellation)?;
        if !scale.is_finite()
            || scale <= 0.0
            || !width.is_finite()
            || width <= 0.0
            || !font_size.is_finite()
            || font_size <= 0.0
        {
            return Err(BridgeError::InvalidRequest(
                "selection layout values must be finite and positive".to_owned(),
            ));
        }
        let retained = self.document(document)?;
        let request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let mut extracted = self
            .extract_selection(
                document,
                retained,
                unit,
                scale,
                width,
                font_size,
                true,
                &cancellation,
                request_slot,
            )
            .await?;
        let handle = self.selection_handle();
        extracted.surface.handle = handle;
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !registry.documents.contains_key(&document) {
            if let Some(raster) = extracted.surface.raster {
                registry.buffers.remove(&raster.handle);
            }
            return Err(BridgeError::InvalidDocumentHandle);
        }
        registry.selections.insert(
            handle,
            RetainedSelection {
                _request_slot: extracted.request_slot,
                _bytes: extracted.retained_bytes,
            },
        );
        drop(registry);
        Ok(extracted.surface)
    }

    #[allow(clippy::too_many_arguments)]
    async fn extract_selection(
        &self,
        document_handle: DocumentHandle,
        retained: Arc<RetainedDocument>,
        unit: usize,
        scale: f32,
        width: f32,
        font_size: f32,
        retain_raster: bool,
        cancellation: &Cancellation,
        request_slot: OwnedSemaphorePermit,
    ) -> Result<ExtractedSelection, BridgeError> {
        let render_slot =
            acquire_permits(Arc::clone(&self.admission.render_slots), 1, cancellation).await?;
        let transient =
            selection_transient_byte_len(&retained.document, unit, scale, retain_raster)?;
        let transient = u32::try_from(transient).map_err(|_| BridgeError::BufferLimit)?;
        let transient_bytes = acquire_permits(
            Arc::clone(&self.admission.probe_bytes),
            transient,
            cancellation,
        )
        .await?;
        let (buffer_slot, buffer_bytes) =
            if retain_raster && matches!(retained.document, OpenDocument::Epub(_)) {
                let slot = try_acquire_slot(
                    Arc::clone(&self.admission.buffer_slots),
                    BridgeError::BufferCountLimit,
                )?;
                let bytes = acquire_permits(
                    Arc::clone(&self.admission.buffer_bytes),
                    u32::try_from(EPUB_TEXT_MAX_PIXELS * 4 * 2)
                        .map_err(|_| BridgeError::BufferLimit)?,
                    cancellation,
                )
                .await?;
                (Some(slot), Some(bytes))
            } else {
                (None, None)
            };
        let worker_cancellation = cancellation.clone();
        let retained_document = Arc::clone(&retained);
        #[cfg(test)]
        let worker_barrier = self.selection_worker_barrier.clone();
        #[cfg(test)]
        let cancellation_barrier = self.selection_second_cancellation_barrier.clone();
        let (extraction, guards) = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(barrier) = worker_barrier {
                barrier.wait();
                barrier.wait();
            }
            #[cfg(test)]
            let cancellation_checks = std::sync::atomic::AtomicUsize::new(0);
            let surface = guarded(|| {
                selection_surface(
                    &retained_document.document,
                    unit,
                    scale,
                    width,
                    font_size,
                    retain_raster,
                    &|| {
                        #[cfg(test)]
                        if cancellation_checks.fetch_add(1, Ordering::Relaxed) == 1
                            && let Some(barrier) = &cancellation_barrier
                        {
                            barrier.wait();
                            barrier.wait();
                        }
                        worker_cancellation.is_cancelled()
                    },
                )
            });
            (
                surface,
                (
                    request_slot,
                    render_slot,
                    transient_bytes,
                    buffer_slot,
                    buffer_bytes,
                ),
            )
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        let (request_slot, render_slot, transient_bytes, buffer_slot, buffer_bytes) = guards;
        let _publication = cancellation
            .0
            .publication
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        check_cancelled(cancellation)?;
        let mut extraction = extraction?;
        let retained_len = selection_retained_byte_len(&extraction.surface)?;
        let mut transient_bytes = transient_bytes;
        let retained_bytes = transient_bytes
            .split(retained_len)
            .ok_or(BridgeError::BufferLimit)?;
        drop(transient_bytes);
        if let Some(pixels) = extraction.raster.take() {
            extraction.surface.raster = Some(self.store_owned_buffer(
                document_handle,
                extraction.raster_width,
                extraction.raster_height,
                pixels,
                buffer_bytes.expect("EPUB raster bytes reserved"),
                buffer_slot.expect("EPUB raster slot reserved"),
            )?);
        }
        drop(render_slot);
        Ok(ExtractedSelection {
            surface: extraction.surface,
            request_slot,
            retained_bytes,
        })
    }

    /// Copy a retained raster into the bridge generator's `Uint8List` representation.
    /// The caller must release the handle after the Dart list is no longer retained.
    pub fn take_buffer(&self, handle: BufferHandle) -> Result<Vec<u8>, BridgeError> {
        self.ensure_buffer_handle(handle)?;
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let buffer = registry
            .buffers
            .get_mut(&handle)
            .ok_or(BridgeError::InvalidBufferHandle)?;
        if buffer.transferred {
            return Err(BridgeError::InvalidBufferHandle);
        }
        buffer.transferred = true;
        Ok(buffer.pixels.clone())
    }

    pub fn release_document(&self, handle: DocumentHandle) -> bool {
        if handle.registry != self.registry_id {
            return false;
        }
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .documents
            .remove(&handle)
            .is_some()
    }

    pub fn release_buffer(&self, handle: BufferHandle) -> bool {
        if handle.registry != self.registry_id {
            return false;
        }
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .buffers
            .remove(&handle)
            .is_some()
    }

    pub fn release_selection(&self, handle: SelectionHandle) -> bool {
        if handle.registry != self.registry_id {
            return false;
        }
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .selections
            .remove(&handle)
            .is_some()
    }

    fn document(&self, handle: DocumentHandle) -> Result<Arc<RetainedDocument>, BridgeError> {
        if handle.registry != self.registry_id {
            return Err(BridgeError::InvalidDocumentHandle);
        }
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .documents
            .get(&handle)
            .cloned()
            .ok_or(BridgeError::InvalidDocumentHandle)
    }

    fn ensure_buffer_handle(&self, handle: BufferHandle) -> Result<(), BridgeError> {
        (handle.registry == self.registry_id)
            .then_some(())
            .ok_or(BridgeError::InvalidBufferHandle)
    }

    fn document_handle(&self) -> DocumentHandle {
        DocumentHandle {
            registry: self.registry_id,
            id: self.next_id(),
        }
    }

    fn buffer_handle(&self) -> BufferHandle {
        BufferHandle {
            registry: self.registry_id,
            id: self.next_id(),
        }
    }

    fn selection_handle(&self) -> SelectionHandle {
        SelectionHandle {
            registry: self.registry_id,
            id: self.next_id(),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_handle.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn store_buffer(
        &self,
        document: DocumentHandle,
        rendered: RenderedPage,
        bytes: OwnedSemaphorePermit,
        slot: OwnedSemaphorePermit,
    ) -> Result<RenderedBuffer, BridgeError> {
        let byte_len = rendered.pixels.len();
        if byte_len
            .checked_mul(2)
            .is_none_or(|peak| peak > bytes.num_permits())
            || rendered.pixels.len() > MAX_BRIDGE_BUFFER_BYTES
        {
            return Err(BridgeError::BufferLimit);
        }
        let pixels = rendered.pixels.to_vec();
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if document.registry != self.registry_id || !registry.documents.contains_key(&document) {
            return Err(BridgeError::InvalidDocumentHandle);
        }
        let handle = self.buffer_handle();
        let result = RenderedBuffer {
            handle,
            width: rendered.width,
            height: rendered.height,
            byte_len,
        };
        registry.buffers.insert(
            handle,
            RetainedBuffer {
                pixels,
                transferred: false,
                _bytes: bytes,
                _slot: slot,
            },
        );
        Ok(result)
    }

    fn store_owned_buffer(
        &self,
        document: DocumentHandle,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        bytes: OwnedSemaphorePermit,
        slot: OwnedSemaphorePermit,
    ) -> Result<RenderedBuffer, BridgeError> {
        let byte_len = pixels.len();
        if byte_len
            .checked_mul(2)
            .is_none_or(|peak| peak > bytes.num_permits())
            || byte_len > MAX_BRIDGE_BUFFER_BYTES
        {
            return Err(BridgeError::BufferLimit);
        }
        let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        if !registry.documents.contains_key(&document) {
            return Err(BridgeError::InvalidDocumentHandle);
        }
        let handle = self.buffer_handle();
        registry.buffers.insert(
            handle,
            RetainedBuffer {
                pixels,
                transferred: false,
                _bytes: bytes,
                _slot: slot,
            },
        );
        Ok(RenderedBuffer {
            handle,
            width,
            height,
            byte_len,
        })
    }
}

async fn acquire_permits(
    semaphore: Arc<Semaphore>,
    permits: u32,
    cancellation: &Cancellation,
) -> Result<OwnedSemaphorePermit, BridgeError> {
    tokio::select! {
        permit = semaphore.acquire_many_owned(permits) => permit.map_err(|_| BridgeError::Worker),
        () = cancellation.cancelled() => Err(BridgeError::Cancelled),
    }
}

fn try_acquire_slot(
    semaphore: Arc<Semaphore>,
    error: BridgeError,
) -> Result<OwnedSemaphorePermit, BridgeError> {
    semaphore.try_acquire_owned().map_err(|_| error)
}

fn guarded<T>(operation: impl FnOnce() -> Result<T, BridgeError>) -> Result<T, BridgeError> {
    catch_unwind(AssertUnwindSafe(operation)).map_err(|_| BridgeError::Panic)?
}

fn check_cancelled(cancellation: &Cancellation) -> Result<(), BridgeError> {
    if cancellation.is_cancelled() {
        Err(BridgeError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn retained_document_byte_len(
    format: BookFormat,
    encoded_byte_len: usize,
) -> Result<usize, BridgeError> {
    let retained = OpenDocument::retained_admission_byte_len(format, encoded_byte_len)
        .ok_or(BridgeError::DocumentLimit)?;
    if retained > MAX_BRIDGE_RETAINED_DOCUMENT_BYTES {
        return Err(BridgeError::DocumentLimit);
    }
    Ok(retained)
}

fn map_open_error(error: OpenDocumentError) -> BridgeError {
    match error {
        OpenDocumentError::UnsupportedFormat(extension) => {
            BridgeError::UnsupportedFormat(extension)
        }
        OpenDocumentError::NotFound => BridgeError::DocumentNotFound,
        OpenDocumentError::Inaccessible(_) => BridgeError::DocumentInaccessible,
        OpenDocumentError::LimitExceeded { format, detail } => {
            BridgeError::OpenLimit { format, detail }
        }
        OpenDocumentError::BackendUnavailable { format, detail } => {
            BridgeError::Backend { format, detail }
        }
        OpenDocumentError::Open { format, detail } => BridgeError::Open { format, detail },
    }
}

fn render_probe_byte_len(document: &OpenDocument, page: usize) -> Result<usize, BridgeError> {
    let page_count = document.page_count();
    if page >= page_count {
        return Err(BridgeError::InvalidPage { page, page_count });
    }
    let byte_len = match document {
        OpenDocument::Cbz(document) => document
            .render_admission_byte_len(page)
            .ok_or(BridgeError::InvalidPage { page, page_count })?,
        OpenDocument::Pdf(_) | OpenDocument::Epub(_) => 0,
    };
    if byte_len > MAX_BRIDGE_PROBE_BYTES {
        return Err(BridgeError::BufferLimit);
    }
    Ok(byte_len)
}

fn render_byte_len(
    document: &OpenDocument,
    page: usize,
    scale: f32,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<usize, BridgeError> {
    if is_cancelled() {
        return Err(BridgeError::Cancelled);
    }
    let page_count = document.page_count();
    if page >= page_count {
        return Err(BridgeError::InvalidPage { page, page_count });
    }
    match document {
        OpenDocument::Pdf(document) => {
            let byte_len = document
                .rendered_byte_len(page, scale)
                .map_err(map_preflight_error)?;
            if is_cancelled() {
                Err(BridgeError::Cancelled)
            } else {
                Ok(byte_len)
            }
        }
        OpenDocument::Cbz(document) => document
            .rendered_byte_len_cancellable(page, scale, is_cancelled)
            .map_err(map_preflight_error),
        OpenDocument::Epub(_) => Err(BridgeError::UnsupportedOperation(BookFormat::Epub)),
    }
}

fn render(
    document: OpenDocument,
    page: usize,
    scale: f32,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RenderedPage, BridgeError> {
    match document {
        OpenDocument::Pdf(document) => document
            .render_page_with_highlights_cancellable(page, scale, &[], is_cancelled)
            .map_err(map_render_error),
        OpenDocument::Cbz(document) => document
            .render_page_cancellable(page, scale, is_cancelled)
            .map_err(map_render_error),
        OpenDocument::Epub(_) => Err(BridgeError::UnsupportedOperation(BookFormat::Epub)),
    }
}

fn selection_surface(
    document: &OpenDocument,
    unit: usize,
    scale: f32,
    width: f32,
    font_size: f32,
    rasterize: bool,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<SelectionExtraction, BridgeError> {
    if is_cancelled() {
        return Err(BridgeError::Cancelled);
    }
    match document {
        OpenDocument::Pdf(document) => {
            let snapshot = document
                .selection_snapshot_cancellable(unit, scale, is_cancelled)
                .map_err(map_render_error)?;
            let (bitmap_width, bitmap_height) = snapshot.bitmap_size();
            let text = snapshot.text().to_owned();
            let copy_eligible = snapshot.text_mapping_complete();
            let (grapheme_boundaries, word_boundaries) = navigation_boundaries(&text);
            let page_rectangles = snapshot
                .page_rectangles(0, text.chars().count())
                .into_iter()
                .map(
                    |(character, (left, bottom, right, top))| SelectionPageRect {
                        character,
                        rect: SelectionRect {
                            left,
                            top: bottom,
                            right,
                            bottom: top,
                        },
                    },
                )
                .collect();
            Ok(SelectionExtraction {
                surface: SelectionSurface {
                    handle: SelectionHandle { registry: 0, id: 0 },
                    width: bitmap_width as f32,
                    height: bitmap_height as f32,
                    text,
                    copy_eligible,
                    resource_path: None,
                    raster: None,
                    endpoints: snapshot
                        .endpoints()
                        .into_iter()
                        .map(pdf_selection_endpoint)
                        .collect(),
                    grapheme_boundaries,
                    word_boundaries,
                    visual_lines: snapshot
                        .visual_lines_cancellable(is_cancelled)
                        .map_err(map_render_error)?
                        .into_iter()
                        .map(|line| SelectionVisualLine {
                            carets: line
                                .carets
                                .into_iter()
                                .map(|caret| SelectionCaret {
                                    offset: caret.character,
                                    x: caret.x,
                                    along_line: caret.along_line,
                                    vertical: caret.vertical,
                                    top: caret.top,
                                    bottom: caret.bottom,
                                })
                                .collect(),
                        })
                        .collect(),
                    page_rectangles,
                },
                raster: None,
                raster_width: 0,
                raster_height: 0,
            })
        }
        OpenDocument::Epub(document) => {
            let chapter =
                document
                    .presentation()
                    .chapter(unit)
                    .ok_or(BridgeError::InvalidPage {
                        page: unit,
                        page_count: document.chapter_count(),
                    })?;
            let text = bounded_epub_selection_text(chapter.search_text(), is_cancelled)?;
            let request = EpubTextRequest {
                runs: vec![EpubTextRun {
                    text: text.clone(),
                    family: None,
                    monospace: false,
                    font_size,
                    bold: false,
                    italic: false,
                    foreground: [0, 0, 0, 255],
                    link: None,
                }],
                max_width: width,
                line_height: font_size * 1.5,
                scale,
                align: EpubTextAlign::Left,
                direction: EpubTextDirection::LeftToRight,
                highlights: Vec::new(),
            };
            let layout = if rasterize {
                document
                    .fonts()
                    .layout_text_cancellable(&request, is_cancelled)
            } else {
                document
                    .fonts()
                    .measure_text_cancellable(&request, is_cancelled)
            }
            .map_err(|error| {
                if is_cancelled() {
                    BridgeError::Cancelled
                } else {
                    map_render_error(error)
                }
            })?;
            let surface_width = layout.width.max(1.0 / scale);
            let surface_height = layout.height.max(1.0 / scale);
            let raster_width = (surface_width * scale).ceil() as u32;
            let raster_height = (surface_height * scale).ceil() as u32;
            let raster_pixels = (raster_width as usize)
                .checked_mul(raster_height as usize)
                .ok_or(BridgeError::BufferLimit)?;
            if rasterize && raster_pixels > EPUB_TEXT_MAX_PIXELS {
                return Err(BridgeError::BufferLimit);
            }
            let mut raster = if rasterize {
                vec![
                    0;
                    raster_pixels
                        .checked_mul(4)
                        .ok_or(BridgeError::BufferLimit)?
                ]
            } else {
                Vec::new()
            };
            for line in layout.lines.iter().filter(|_| rasterize) {
                if is_cancelled() {
                    return Err(BridgeError::Cancelled);
                }
                let top = (line.top * scale).round() as usize;
                let copy_width = raster_width.min(line.pixel_width) as usize;
                for row in 0..line.pixel_height as usize {
                    if is_cancelled() {
                        return Err(BridgeError::Cancelled);
                    }
                    let destination = (top + row)
                        .checked_mul(raster_width as usize)
                        .and_then(|offset| offset.checked_mul(4))
                        .and_then(|offset| offset.checked_add(copy_width * 4))
                        .ok_or(BridgeError::BufferLimit)?;
                    if destination > raster.len() {
                        return Err(BridgeError::BufferLimit);
                    }
                    let source = row * line.pixel_width as usize * 4;
                    raster[destination - copy_width * 4..destination]
                        .copy_from_slice(&line.rgba[source..source + copy_width * 4]);
                }
            }
            let path = document.chapter(unit).map(|chapter| chapter.path.clone());
            let (grapheme_boundaries, word_boundaries) = navigation_boundaries(&text);
            let visual_lines = epub_visual_lines(&text, &layout, scale);
            Ok(SelectionExtraction {
                surface: SelectionSurface {
                    handle: SelectionHandle { registry: 0, id: 0 },
                    width: surface_width,
                    height: surface_height,
                    text,
                    copy_eligible: true,
                    resource_path: path,
                    raster: None,
                    endpoints: layout
                        .endpoints
                        .into_iter()
                        .map(|endpoint| SelectionEndpoint {
                            offset: endpoint.scalar,
                            range_start: endpoint.scalar_start,
                            range_end: endpoint.scalar_end,
                            rect: SelectionRect {
                                left: endpoint.rect.x,
                                top: endpoint.rect.y,
                                right: endpoint.rect.x + endpoint.rect.width,
                                bottom: endpoint.rect.y + endpoint.rect.height,
                            },
                        })
                        .collect(),
                    grapheme_boundaries,
                    word_boundaries,
                    visual_lines,
                    page_rectangles: Vec::new(),
                },
                raster: rasterize.then_some(raster),
                raster_width,
                raster_height,
            })
        }
        OpenDocument::Cbz(_) => Err(BridgeError::UnsupportedOperation(BookFormat::Cbz)),
    }
}

fn epub_visual_lines(
    text: &str,
    layout: &crate::epub::EpubTextLayout,
    scale: f32,
) -> Vec<SelectionVisualLine> {
    let mut line_carets = vec![Vec::new(); layout.lines.len()];
    for endpoint in &layout.endpoints {
        if let Some(carets) = line_carets.get_mut(endpoint.visual_line) {
            carets.push(SelectionCaret {
                offset: endpoint.scalar,
                x: endpoint.caret_x,
                along_line: endpoint.caret_x,
                vertical: false,
                top: endpoint.rect.y,
                bottom: endpoint.rect.y + endpoint.rect.height,
            });
        }
    }
    if !text.is_empty() {
        let scalar_count = text.chars().count();
        for (line, carets) in layout.lines.iter().zip(&mut line_carets) {
            if carets.is_empty() && line.scalars.start <= scalar_count {
                carets.push(SelectionCaret {
                    offset: line.scalars.start,
                    x: if line.rtl { line.width } else { 0.0 },
                    along_line: if line.rtl { line.width } else { 0.0 },
                    vertical: false,
                    top: line.top,
                    bottom: line.top + line.pixel_height as f32 / scale,
                });
            }
        }
    }
    line_carets
        .into_iter()
        .map(|mut carets| {
            carets.sort_by(|left, right| left.x.total_cmp(&right.x));
            carets.dedup_by(|left, right| left.offset == right.offset);
            SelectionVisualLine { carets }
        })
        .collect()
}

fn navigation_boundaries(text: &str) -> (Vec<usize>, Vec<usize>) {
    let mut scalar = 0;
    let mut graphemes = vec![0];
    for grapheme in text.graphemes(true) {
        scalar += grapheme.chars().count();
        graphemes.push(scalar);
    }
    let mut words = Vec::new();
    scalar = 0;
    for segment in text.split_word_bounds() {
        let end = scalar + segment.chars().count();
        if segment.unicode_words().next().is_some() {
            words.extend([scalar, end]);
        }
        scalar = end;
    }
    words.extend([0, scalar]);
    words.sort_unstable();
    words.dedup();
    (graphemes, words)
}

struct SelectionExtraction {
    surface: SelectionSurface,
    raster: Option<Vec<u8>>,
    raster_width: u32,
    raster_height: u32,
}

struct ExtractedSelection {
    surface: SelectionSurface,
    request_slot: OwnedSemaphorePermit,
    retained_bytes: OwnedSemaphorePermit,
}

fn selection_retained_byte_len(surface: &SelectionSurface) -> Result<usize, BridgeError> {
    let vectors = surface
        .endpoints
        .capacity()
        .checked_mul(std::mem::size_of::<SelectionEndpoint>())
        .and_then(|bytes| {
            bytes.checked_add(surface.grapheme_boundaries.capacity() * std::mem::size_of::<usize>())
        })
        .and_then(|bytes| {
            bytes.checked_add(surface.word_boundaries.capacity() * std::mem::size_of::<usize>())
        })
        .and_then(|bytes| {
            bytes.checked_add(
                surface.visual_lines.capacity() * std::mem::size_of::<SelectionVisualLine>(),
            )
        })
        .and_then(|bytes| {
            surface.visual_lines.iter().try_fold(bytes, |total, line| {
                total.checked_add(line.carets.capacity() * std::mem::size_of::<SelectionCaret>())
            })
        })
        .and_then(|bytes| {
            bytes.checked_add(
                surface.page_rectangles.capacity() * std::mem::size_of::<SelectionPageRect>(),
            )
        })
        .ok_or(BridgeError::BufferLimit)?;
    std::mem::size_of::<SelectionSurface>()
        .checked_add(surface.text.capacity())
        .and_then(|bytes| {
            bytes.checked_add(surface.resource_path.as_ref().map_or(0, String::capacity))
        })
        .and_then(|bytes| bytes.checked_add(vectors))
        .ok_or(BridgeError::BufferLimit)
}

fn bounded_epub_selection_text(
    chapter_text: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<String, BridgeError> {
    let mut scalar_count = 0;
    for _ in chapter_text.chars() {
        if scalar_count % 1024 == 0 && is_cancelled() {
            return Err(BridgeError::Cancelled);
        }
        scalar_count += 1;
        if scalar_count > EPUB_TEXT_MAX_SCALARS {
            return Err(BridgeError::BufferLimit);
        }
    }
    Ok(chapter_text.to_owned())
}

fn pdf_selection_endpoint(
    (rect, endpoint): (
        crate::pdf::PdfSelectionRect,
        crate::pdf::PdfSelectionEndpoint,
    ),
) -> SelectionEndpoint {
    SelectionEndpoint {
        offset: endpoint.character,
        range_start: endpoint.underlying_character,
        range_end: endpoint.underlying_character.saturating_add(1),
        rect: SelectionRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
    }
}

fn selection_transient_byte_len(
    document: &OpenDocument,
    unit: usize,
    scale: f32,
    rasterize: bool,
) -> Result<usize, BridgeError> {
    if unit >= document.page_count() {
        return Err(BridgeError::InvalidPage {
            page: unit,
            page_count: document.page_count(),
        });
    }
    match document {
        OpenDocument::Pdf(document) => document
            .selection_admission_byte_len(unit, scale)
            .map_err(map_preflight_error),
        OpenDocument::Epub(_) => {
            let native_workspace = EPUB_TEXT_MAX_ENDPOINTS
                .checked_mul(std::mem::size_of::<EpubTextEndpoint>())
                // Chapter text, request runs, shaping text/control buffers, and
                // scalar-boundary indexes coexist during native layout.
                .and_then(|bytes| bytes.checked_add(EPUB_TEXT_MAX_SCALARS * 4 * 12))
                .ok_or(BridgeError::BufferLimit)?;
            // Bridge geometry is built while native layout remains live. Caret
            // vectors may retain up to four slots for each endpoint because
            // Vec's first growth allocates four elements; boundary vectors can
            // retain the next power-of-two capacity above the scalar ceiling.
            let bridge_geometry = EPUB_TEXT_MAX_ENDPOINTS
                .checked_mul(std::mem::size_of::<SelectionEndpoint>())
                .and_then(|bytes| {
                    bytes.checked_add(
                        EPUB_TEXT_MAX_ENDPOINTS * 4 * std::mem::size_of::<SelectionCaret>(),
                    )
                })
                .and_then(|bytes| {
                    bytes.checked_add(
                        EPUB_TEXT_MAX_ENDPOINTS * std::mem::size_of::<SelectionVisualLine>(),
                    )
                })
                .and_then(|bytes| {
                    bytes.checked_add(EPUB_TEXT_MAX_SCALARS * 2 * 2 * std::mem::size_of::<usize>())
                })
                .and_then(|bytes| bytes.checked_add(EPUB_TEXT_MAX_SCALARS * 4))
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<SelectionSurface>()))
                .ok_or(BridgeError::BufferLimit)?;
            let rasters = if rasterize {
                EPUB_TEXT_MAX_PIXELS * 4 * 2
            } else {
                0
            };
            native_workspace
                .checked_add(bridge_geometry)
                .and_then(|bytes| bytes.checked_add(rasters))
                .ok_or(BridgeError::BufferLimit)
        }
        OpenDocument::Cbz(_) => Err(BridgeError::UnsupportedOperation(BookFormat::Cbz)),
    }
}

fn storage_error(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::Storage(error.to_string())
}

fn annotation_storage_error(error: anyhow::Error) -> BridgeError {
    if error.is::<AnnotationSnapshotLimit>() {
        BridgeError::AnnotationLimit
    } else {
        storage_error(error)
    }
}

fn bounded_annotation_snapshot(
    annotations: Vec<Annotation>,
) -> Result<Vec<Annotation>, BridgeError> {
    let retained_bytes = annotations.iter().try_fold(0usize, |total, annotation| {
        let strings = annotation.id.to_string().len()
            + annotation.body.as_ref().map_or(0, String::len)
            + annotation.quote.as_ref().map_or(0, |quote| {
                quote.original.as_ref().map_or(0, String::len)
                    + quote.exact.len()
                    + quote.prefix.len()
                    + quote.suffix.len()
            });
        // Text-backed PDF highlights paint through the retained selection
        // surface, so only geometry-only annotations retain DTO rectangles.
        let rectangles = match &annotation.target {
            AnnotationTarget::Pdf(anchor) if anchor.character_range.is_none() => {
                anchor.rectangles.len() * std::mem::size_of::<PageRect>()
            }
            AnnotationTarget::Pdf(_) | AnnotationTarget::Epub(_) => 0,
        };
        total
            .checked_add(ANNOTATION_SNAPSHOT_BASE_BYTES + strings + rectangles)
            .ok_or(BridgeError::AnnotationLimit)
    })?;
    if retained_bytes > MAX_ANNOTATION_SNAPSHOT_BYTES {
        return Err(BridgeError::AnnotationLimit);
    }
    Ok(annotations)
}

fn annotation_dtos(
    annotations: Vec<Annotation>,
    document: &OpenDocument,
    scale: f32,
    resolve_persisted_text: bool,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Vec<BridgeAnnotation>, BridgeError> {
    let mut annotations_by_unit = HashMap::<usize, Vec<usize>>::new();
    for (index, annotation) in annotations.iter().enumerate() {
        if !resolve_persisted_text {
            continue;
        }
        if is_cancelled() {
            return Err(BridgeError::Cancelled);
        }
        let unit = match (&annotation.target, document) {
            (AnnotationTarget::Epub(anchor), OpenDocument::Epub(document)) => {
                let unit = anchor.spine_occurrence as usize;
                if document
                    .chapter(unit)
                    .is_none_or(|chapter| chapter.path != anchor.resource_path.as_str())
                {
                    continue;
                }
                unit
            }
            (AnnotationTarget::Pdf(anchor), OpenDocument::Pdf(document))
                if anchor.character_range.is_some()
                    && (anchor.page as usize) < document.page_count() =>
            {
                anchor.page as usize
            }
            _ => continue,
        };
        annotations_by_unit.entry(unit).or_default().push(index);
    }
    let mut resolved_texts = if resolve_persisted_text {
        vec![None; annotations.len()]
    } else {
        annotations
            .iter()
            .map(|annotation| {
                let range = match &annotation.target {
                    AnnotationTarget::Epub(anchor) => {
                        Some(anchor.scalar_start as usize..anchor.scalar_end as usize)
                    }
                    AnnotationTarget::Pdf(anchor) => anchor
                        .character_range
                        .map(|(start, end)| start as usize..end as usize),
                };
                range.map(|range| crate::annotations::ResolvedTextAnchor {
                    resolution: AnnotationResolution::Exact,
                    range: Some(range),
                })
            })
            .collect()
    };
    let mut remaining_work = MAX_TEXT_ANCHOR_RESOLUTION_WORK;
    for (unit, indices) in annotations_by_unit {
        let text = match document {
            OpenDocument::Epub(document) => document
                .presentation()
                .chapter(unit)
                .map(|chapter| bounded_epub_selection_text(chapter.search_text(), is_cancelled))
                .transpose()?,
            OpenDocument::Pdf(document) if unit < document.page_count() => Some(
                document
                    .page_text_bounded(unit, MAX_ANNOTATION_PDF_TEXT_BYTES, is_cancelled)
                    .map_err(|error| match error {
                        crate::pdf::BoundedPageTextError::Cancelled => BridgeError::Cancelled,
                        crate::pdf::BoundedPageTextError::Limit { .. } => BridgeError::BufferLimit,
                        crate::pdf::BoundedPageTextError::Document(error) => {
                            map_render_error(error)
                        }
                    })?,
            ),
            OpenDocument::Pdf(_) => None,
            OpenDocument::Cbz(_) => None,
        };
        let Some(text) = text else {
            continue;
        };
        let scalar_index = TextScalarIndex::new(&text, &mut remaining_work, is_cancelled)
            .map_err(map_text_anchor_resolution_error)?;
        let mut unresolved = Vec::new();
        for index in indices {
            let (stored_range, quote) =
                match (&annotations[index].target, &annotations[index].quote) {
                    (AnnotationTarget::Epub(anchor), Some(quote)) => (
                        anchor.scalar_start as usize..anchor.scalar_end as usize,
                        quote,
                    ),
                    (AnnotationTarget::Pdf(anchor), Some(quote)) => {
                        let Some((start, end)) = anchor.character_range else {
                            continue;
                        };
                        (start as usize..end as usize, quote)
                    }
                    _ => continue,
                };
            if let Some(resolved) = scalar_index
                .resolve_exact(stored_range, quote, &mut remaining_work, is_cancelled)
                .map_err(map_text_anchor_resolution_error)?
            {
                resolved_texts[index] = Some(resolved);
            } else {
                unresolved.push(index);
            }
        }
        if unresolved.is_empty() {
            continue;
        }
        let resolver =
            TextAnchorResolver::from_index(scalar_index, &mut remaining_work, is_cancelled)
                .map_err(map_text_anchor_resolution_error)?;
        for index in unresolved {
            let (stored_range, quote) =
                match (&annotations[index].target, &annotations[index].quote) {
                    (AnnotationTarget::Epub(anchor), Some(quote)) => (
                        anchor.scalar_start as usize..anchor.scalar_end as usize,
                        quote,
                    ),
                    (AnnotationTarget::Pdf(anchor), Some(quote)) => {
                        let Some((start, end)) = anchor.character_range else {
                            continue;
                        };
                        (start as usize..end as usize, quote)
                    }
                    _ => continue,
                };
            resolved_texts[index] = Some(
                resolver
                    .resolve(stored_range, quote, &mut remaining_work, is_cancelled)
                    .map_err(map_text_anchor_resolution_error)?,
            );
        }
    }
    let batches = annotations
        .iter()
        .enumerate()
        .filter_map(|(index, annotation)| match (&annotation.target, document) {
            (AnnotationTarget::Pdf(anchor), OpenDocument::Pdf(pdf))
                if anchor.character_range.is_none()
                    && (anchor.page as usize) < pdf.page_count() =>
            {
                Some((
                    index,
                    (
                        anchor.page as usize,
                        anchor
                            .rectangles
                            .iter()
                            .map(|rect| (rect.left, rect.bottom, rect.right, rect.top))
                            .collect(),
                    ),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut resolved_rectangles = if batches.is_empty() {
        HashMap::new()
    } else {
        let OpenDocument::Pdf(pdf) = document else {
            unreachable!("only matching PDF batches are collected");
        };
        let geometry = batches
            .iter()
            .map(|(_, batch)| batch.clone())
            .collect::<Vec<_>>();
        let converted = pdf
            .page_rectangle_batches_to_pixels(&geometry, scale, is_cancelled)
            .map_err(map_render_error)?
            .into_iter();
        batches
            .into_iter()
            .map(|(index, _)| index)
            .zip(converted)
            .collect()
    };
    annotations
        .into_iter()
        .enumerate()
        .map(|(index, annotation)| {
            if is_cancelled() {
                return Err(BridgeError::Cancelled);
            }
            annotation_dto(
                annotation,
                resolved_rectangles.remove(&index),
                resolved_texts[index].take(),
            )
        })
        .collect()
}

fn annotation_dto(
    annotation: Annotation,
    resolved_pdf_rectangles: Option<Vec<crate::pdf::PdfSelectionRect>>,
    resolved_text: Option<crate::annotations::ResolvedTextAnchor>,
) -> Result<BridgeAnnotation, BridgeError> {
    let quote = annotation
        .quote
        .as_ref()
        .and_then(|quote| quote.original.clone());
    let geometry_only = matches!(
        &annotation.target,
        AnnotationTarget::Pdf(anchor) if anchor.character_range.is_none()
    );
    let geometry_resolved = resolved_pdf_rectangles.is_some();
    let resolution = resolved_text.as_ref().map_or_else(
        || {
            if geometry_only && geometry_resolved {
                AnnotationResolution::Exact
            } else {
                AnnotationResolution::Orphaned
            }
        },
        |resolved| resolved.resolution,
    );
    let (unit, text_range, rectangles) = match annotation.target {
        AnnotationTarget::Epub(anchor) => (
            anchor.spine_occurrence as usize,
            resolved_text
                .and_then(|resolved| resolved.range)
                .map(|range| AnnotationTextRange {
                    start: range.start,
                    end: range.end,
                }),
            Vec::new(),
        ),
        AnnotationTarget::Pdf(anchor) => {
            let text_range = resolved_text
                .and_then(|resolved| resolved.range)
                .map(|range| AnnotationTextRange {
                    start: range.start,
                    end: range.end,
                });
            let page = anchor.page as usize;
            let rectangles = if anchor.character_range.is_some() {
                Vec::new()
            } else {
                resolved_pdf_rectangles
                    .unwrap_or_default()
                    .into_iter()
                    .map(|rect| SelectionRect {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.bottom,
                    })
                    .collect()
            };
            (page, text_range, rectangles)
        }
    };
    Ok(BridgeAnnotation {
        id: annotation.id.to_string(),
        unit,
        resolution,
        text_range,
        quote,
        rectangles,
        color: annotation.color,
        body: annotation.body,
    })
}

fn map_text_anchor_resolution_error(error: TextAnchorResolutionError) -> BridgeError {
    match error {
        TextAnchorResolutionError::InvalidSelector => {
            BridgeError::InvalidRequest(error.to_string())
        }
        TextAnchorResolutionError::Cancelled => BridgeError::Cancelled,
        TextAnchorResolutionError::WorkLimit => BridgeError::BufferLimit,
    }
}

fn validate_annotation_body(body: Option<&str>) -> Result<(), BridgeError> {
    if body.is_some_and(|body| {
        body.chars().take(MAX_ANNOTATION_BODY_SCALARS + 1).count() > MAX_ANNOTATION_BODY_SCALARS
    }) {
        Err(BridgeError::InvalidRequest(format!(
            "annotation body exceeds {MAX_ANNOTATION_BODY_SCALARS} Unicode scalars"
        )))
    } else {
        Ok(())
    }
}

fn map_preflight_error(error: anyhow::Error) -> BridgeError {
    if is_resource_limit(&error) {
        BridgeError::BufferLimit
    } else {
        BridgeError::InvalidRequest(error.to_string())
    }
}

fn map_render_error(error: anyhow::Error) -> BridgeError {
    if is_resource_limit(&error) {
        BridgeError::BufferLimit
    } else {
        BridgeError::Render(error.to_string())
    }
}

fn is_resource_limit(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<crate::application::ResourceLimitError>()
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn cbz_request() -> OpenRequest {
        OpenRequest {
            book_id: None,
            local_id: "fixture".to_owned(),
            path_key: crate::path_key::path_key(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/sample.cbz"
            ))),
            format_hint: Some(BookFormat::Cbz),
        }
    }

    #[tokio::test]
    async fn bridge_rejects_unverified_book_identity_pairings() {
        let bridge = Bridge::new();
        let mut request = cbz_request();
        request.book_id = Some(7);

        assert!(matches!(
            bridge.open_document(request, Cancellation::new()).await,
            Err(BridgeError::InvalidRequest(message)) if message.contains("library-backed")
        ));
    }

    fn pdf_request() -> OpenRequest {
        OpenRequest {
            book_id: None,
            local_id: "pdf-fixture".to_owned(),
            path_key: crate::path_key::path_key(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/sample.pdf"
            ))),
            format_hint: Some(BookFormat::Pdf),
        }
    }

    fn epub_request() -> OpenRequest {
        OpenRequest {
            book_id: None,
            local_id: "epub-fixture".to_owned(),
            path_key: crate::path_key::path_key(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/sample.epub"
            ))),
            format_hint: Some(BookFormat::Epub),
        }
    }

    fn annotation_request(document: DocumentHandle) -> CreateAnnotationRequest {
        CreateAnnotationRequest {
            document,
            unit: 0,
            start: 0,
            end: 1,
            display_scale: 1.0,
            color: HighlightColor::Yellow,
            body: None,
        }
    }

    fn empty_epub() -> Vec<u8> {
        epub_with_body("")
    }

    fn epub_with_body(body: &str) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let chapter =
            format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><body>{body}</body></html>");
        let entries: Vec<(&str, &[u8])> = vec![
            ("mimetype", b"application/epub+zip"),
            (
                "META-INF/container.xml",
                br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="OPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
            ),
            (
                "OPS/content.opf",
                br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">empty</dc:identifier><dc:title>Empty</dc:title><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#,
            ),
            (
                "OPS/chapter.xhtml", chapter.as_bytes(),
            ),
        ];
        for (path, contents) in &entries {
            archive
                .start_file(*path, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(contents).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    fn selectable_pdf_with_media_box(width: u32, height: u32, text: &str) -> Vec<u8> {
        let content = format!("BT /F1 200 Tf 1 0 0 1 100 {} Tm ({text}) Tj ET", height / 2);
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
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
    fn bridge_errors_expose_stable_categories() {
        assert_eq!(
            BridgeError::InvalidDocumentHandle.kind(),
            BridgeErrorKind::NotFound
        );
        assert_eq!(
            BridgeError::DocumentInaccessible.kind(),
            BridgeErrorKind::Inaccessible
        );
        assert_eq!(
            BridgeError::InvalidRequest("bad scale".to_owned()).kind(),
            BridgeErrorKind::InvalidRequest
        );
        assert_eq!(
            BridgeError::Open {
                format: BookFormat::Cbz,
                detail: "entry exceeds byte limit".to_owned(),
            }
            .kind(),
            BridgeErrorKind::Malformed,
            "detail text must not determine the category"
        );
        assert_eq!(
            BridgeError::UnsupportedOperation(BookFormat::Epub).kind(),
            BridgeErrorKind::Unsupported
        );
        assert_eq!(
            BridgeError::BufferLimit.kind(),
            BridgeErrorKind::LimitExceeded
        );
        assert_eq!(
            map_open_error(OpenDocumentError::BackendUnavailable {
                format: BookFormat::Pdf,
                detail: "missing PDFium".to_owned(),
            })
            .kind(),
            BridgeErrorKind::BackendUnavailable
        );
        assert_eq!(
            BridgeError::Worker.kind(),
            BridgeErrorKind::BackendUnavailable
        );
        assert_eq!(
            BridgeError::Render("backend error".to_owned()).kind(),
            BridgeErrorKind::RenderFailed
        );
        assert_eq!(
            map_preflight_error(anyhow::Error::new(crate::application::ResourceLimitError(
                "decoded image limit".to_owned()
            ))),
            BridgeError::BufferLimit
        );
        assert_eq!(
            map_render_error(anyhow::Error::new(crate::application::ResourceLimitError(
                "PDF endpoint limit".to_owned()
            ))),
            BridgeError::BufferLimit
        );
    }

    #[test]
    fn pdf_caret_endpoint_keeps_its_underlying_character_range() {
        let mapped = pdf_selection_endpoint((
            crate::pdf::PdfSelectionRect {
                left: 1.0,
                top: 2.0,
                right: 3.0,
                bottom: 4.0,
            },
            crate::pdf::PdfSelectionEndpoint {
                underlying_character: 7,
                character: 8,
                page_x: 0.0,
                page_y: 0.0,
            },
        ));

        assert_eq!(mapped.offset, 8);
        assert_eq!((mapped.range_start, mapped.range_end), (7, 8));
    }

    #[test]
    fn navigation_stops_are_grapheme_safe_and_unicode_word_aware() {
        let text = "Cafe\u{301}—naïve! 東京";
        let (graphemes, words) = navigation_boundaries(text);

        assert!(graphemes.contains(&5));
        assert!(!graphemes.contains(&4), "decomposed accent is one grapheme");
        assert_eq!(words, vec![0, 5, 6, 11, 13, 14, 15]);
    }

    #[test]
    fn word_stops_skip_standalone_whitespace_and_punctuation() {
        let (_, words) = navigation_boundaries("one,  two?! 三");
        assert_eq!(words, vec![0, 3, 6, 9, 12, 13]);
    }

    #[test]
    fn geometry_only_pdf_annotation_dto_does_not_invent_text() {
        let pdf = crate::pdf::PdfDoc::open(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.pdf"
        )))
        .unwrap();
        let canonical = pdf
            .selection_snapshot(0, 1.0)
            .unwrap()
            .page_rectangles(0, 1)[0]
            .1;
        let expected = pdf.page_rectangles_to_pixels(0, 2.0, &[canonical]).unwrap()[0];
        let annotation = Annotation {
            id: AnnotationId::new(),
            book_id: None,
            local_path: Some("sample.pdf".into()),
            fingerprint: DocumentFingerprint::new("sha256", 1, vec![7; 32]).unwrap(),
            quote: None,
            target: AnnotationTarget::Pdf(
                PdfAnchor::new(
                    0,
                    None,
                    vec![
                        PageRect::new(canonical.0, canonical.1, canonical.2, canonical.3).unwrap(),
                    ],
                )
                .unwrap(),
            ),
            color: HighlightColor::Yellow,
            body: None,
            provenance: None,
            created_at: "now".into(),
            modified_at: "now".into(),
            deleted_at: None,
        };

        let dto = annotation_dtos(
            vec![annotation],
            &OpenDocument::Pdf(pdf.into()),
            2.0,
            true,
            &|| false,
        )
        .unwrap()
        .remove(0);
        assert_eq!(dto.resolution, AnnotationResolution::Exact);
        assert_eq!(dto.text_range, None);
        assert_eq!(dto.quote, None);
        assert_eq!(
            dto.rectangles,
            vec![SelectionRect {
                left: expected.left,
                top: expected.top,
                right: expected.right,
                bottom: expected.bottom,
            }]
        );
    }

    #[test]
    fn missing_pdf_pages_orphan_only_the_affected_annotations() {
        let pdf = crate::pdf::PdfDoc::open(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.pdf"
        )))
        .unwrap();
        let fingerprint = DocumentFingerprint::new("sha256", 1, vec![7; 32]).unwrap();
        let annotation = |target, quote| Annotation {
            id: AnnotationId::new(),
            book_id: None,
            local_path: Some("sample.pdf".into()),
            fingerprint: fingerprint.clone(),
            quote,
            target,
            color: HighlightColor::Yellow,
            body: None,
            provenance: None,
            created_at: "now".into(),
            modified_at: "now".into(),
            deleted_at: None,
        };
        let missing_page = u32::try_from(pdf.page_count()).unwrap();
        let geometry = annotation(
            AnnotationTarget::Pdf(
                PdfAnchor::new(
                    missing_page,
                    None,
                    vec![PageRect::new(0.0, 0.0, 1.0, 1.0).unwrap()],
                )
                .unwrap(),
            ),
            None,
        );
        let text = annotation(
            AnnotationTarget::Pdf(
                PdfAnchor::new(
                    missing_page,
                    Some((0, 1)),
                    vec![PageRect::new(0.0, 0.0, 1.0, 1.0).unwrap()],
                )
                .unwrap(),
            ),
            Some(QuoteSelector::new("x", "", "").unwrap()),
        );
        let valid_geometry = annotation(
            AnnotationTarget::Pdf(
                PdfAnchor::new(0, None, vec![PageRect::new(0.0, 0.0, 1.0, 1.0).unwrap()]).unwrap(),
            ),
            None,
        );
        let incompatible = annotation(
            AnnotationTarget::Epub(EpubAnchor::new(0, "missing.xhtml", 0, 1).unwrap()),
            Some(QuoteSelector::new("x", "", "").unwrap()),
        );

        let resolved = annotation_dtos(
            vec![geometry, text, valid_geometry, incompatible],
            &OpenDocument::Pdf(pdf.into()),
            1.0,
            true,
            &|| false,
        )
        .unwrap();
        assert_eq!(resolved.len(), 4);
        assert!(resolved[..2].iter().all(|item| {
            item.resolution == AnnotationResolution::Orphaned
                && item.text_range.is_none()
                && item.rectangles.is_empty()
        }));
        assert_eq!(resolved[2].resolution, AnnotationResolution::Exact);
        assert!(!resolved[2].rectangles.is_empty());
        assert_eq!(resolved[3].resolution, AnnotationResolution::Orphaned);
    }

    #[test]
    fn annotation_snapshot_rejects_aggregate_retained_bytes() {
        let annotations = (0..=MAX_ANNOTATION_SNAPSHOT_BYTES
            / crate::annotations::MAX_ANNOTATION_BODY_SCALARS)
            .map(|_| Annotation {
                id: AnnotationId::new(),
                book_id: None,
                local_path: Some("sample.epub".into()),
                fingerprint: DocumentFingerprint::new("sha256", 1, vec![7; 32]).unwrap(),
                quote: None,
                target: AnnotationTarget::Epub(EpubAnchor::new(0, "chapter.xhtml", 0, 1).unwrap()),
                color: HighlightColor::Yellow,
                body: Some("x".repeat(crate::annotations::MAX_ANNOTATION_BODY_SCALARS)),
                provenance: None,
                created_at: "now".into(),
                modified_at: "now".into(),
                deleted_at: None,
            })
            .collect();

        assert_eq!(
            bounded_annotation_snapshot(annotations).unwrap_err(),
            BridgeError::AnnotationLimit
        );
    }

    #[test]
    fn public_bridge_facades_share_process_admission() {
        let first = Bridge::new();
        let second = Bridge::new();

        assert!(Arc::ptr_eq(&first.admission, &second.admission));
    }

    #[tokio::test]
    async fn document_slots_are_shared_across_bridge_facades() {
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.document_slots = Arc::new(Semaphore::new(1));
        let admission = Arc::new(admission);
        let first_bridge = Bridge::with_admission(Arc::clone(&admission));
        let second_bridge = Bridge::with_admission(admission);
        let first = first_bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        let mut waiting = Box::pin(second_bridge.open_document(cbz_request(), Cancellation::new()));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting)
                .await
                .is_err(),
            "the second facade must wait for the shared document slot"
        );
        assert!(first_bridge.release_document(first.handle));
        let second = waiting.await.unwrap();
        assert!(second_bridge.release_document(second.handle));
    }

    #[tokio::test]
    async fn request_count_rejects_without_creating_waiters() {
        let admission = Arc::new(BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1));
        let bridge = Bridge::with_admission(Arc::clone(&admission));
        let _requests = Arc::clone(&admission.request_slots)
            .acquire_many_owned(MAX_BRIDGE_REQUESTS as u32)
            .await
            .unwrap();

        let error = bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap_err();

        assert_eq!(error, BridgeError::RequestLimit);
    }

    #[tokio::test]
    async fn annotation_mutations_share_request_admission() {
        let directory = tempfile::tempdir().unwrap();
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.request_slots = Arc::new(Semaphore::new(1));
        let admission = Arc::new(admission);
        let bridge = Bridge::with_admission_database(
            Arc::clone(&admission),
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let id = AnnotationId::new().to_string();
        let request = Arc::clone(&admission.request_slots)
            .acquire_owned()
            .await
            .unwrap();

        assert_eq!(
            bridge
                .update_annotation(document.handle, &id, HighlightColor::Green, None,)
                .await,
            Err(BridgeError::RequestLimit)
        );
        assert_eq!(
            bridge.delete_annotation(document.handle, &id).await,
            Err(BridgeError::RequestLimit)
        );

        drop(request);
        assert!(
            !bridge
                .update_annotation(document.handle, &id, HighlightColor::Green, None,)
                .await
                .unwrap()
        );
        assert_eq!(admission.request_slots.available_permits(), 1);
        assert!(
            !bridge
                .delete_annotation(document.handle, &id)
                .await
                .unwrap()
        );
        assert_eq!(admission.request_slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn buffer_count_is_shared_and_released_with_the_buffer() {
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.buffer_slots = Arc::new(Semaphore::new(1));
        let admission = Arc::new(admission);
        let first = Bridge::with_admission(Arc::clone(&admission));
        let second = Bridge::with_admission(admission);
        let first_document = first
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        let second_document = second
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        let request = |document| RenderRequest {
            document,
            page: 0,
            scale: 1.0,
        };
        let buffer = first
            .render_page(request(first_document.handle), Cancellation::new())
            .await
            .unwrap();

        assert_eq!(
            second
                .render_page(request(second_document.handle), Cancellation::new())
                .await,
            Err(BridgeError::BufferCountLimit)
        );
        assert!(first.release_buffer(buffer.handle));
        let buffer = second
            .render_page(request(second_document.handle), Cancellation::new())
            .await
            .unwrap();
        assert!(second.release_buffer(buffer.handle));
    }

    #[tokio::test]
    async fn cbz_probe_and_decode_peak_is_acquired_atomically() {
        let document = OpenDocument::open(&DeviceFileLocator::from_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.cbz"
        )))
        .unwrap();
        let permits = render_probe_byte_len(&document, 0).unwrap();
        let semaphore = Arc::new(Semaphore::new(permits));
        let first = acquire_permits(
            Arc::clone(&semaphore),
            u32::try_from(permits).unwrap(),
            &Cancellation::new(),
        )
        .await
        .unwrap();
        let cancellation = Cancellation::new();
        let mut second = Box::pin(acquire_permits(
            Arc::clone(&semaphore),
            u32::try_from(permits).unwrap(),
            &cancellation,
        ));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut second)
                .await
                .is_err()
        );
        drop(first);
        let _second = second.await.unwrap();
    }

    #[tokio::test]
    async fn pdf_render_waits_for_transient_memory_admission() {
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.probe_bytes = Arc::new(Semaphore::new(0));
        let bridge = Bridge::with_admission(Arc::new(admission));
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let mut render = Box::pin(bridge.render_page(
            RenderRequest {
                document: document.handle,
                page: 0,
                scale: 1.0,
            },
            cancellation.clone(),
        ));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut render)
                .await
                .is_err()
        );
        cancellation.cancel();
        assert_eq!(render.await, Err(BridgeError::Cancelled));
    }

    #[tokio::test]
    async fn pdf_selection_uses_request_render_and_transient_admission_before_worker() {
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.probe_bytes = Arc::new(Semaphore::new(0));
        let admission = Arc::new(admission);
        let bridge = Bridge::with_admission(Arc::clone(&admission));
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let mut selection = Box::pin(bridge.selection_surface(
            document.handle,
            0,
            1.0,
            680.0,
            18.0,
            cancellation.clone(),
        ));

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut selection)
                .await
                .is_err()
        );
        assert_eq!(
            admission.request_slots.available_permits(),
            MAX_BRIDGE_REQUESTS - 1
        );
        assert_eq!(admission.render_slots.available_permits(), 0);
        cancellation.cancel();
        assert_eq!(selection.await, Err(BridgeError::Cancelled));
        assert_eq!(
            admission.request_slots.available_permits(),
            MAX_BRIDGE_REQUESTS
        );
        assert_eq!(admission.render_slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn epub_selection_raster_is_retained_until_explicit_release() {
        let buffer_budget = EPUB_TEXT_MAX_PIXELS * 4 * 2;
        let bridge = Bridge::with_limits(buffer_budget, 1);
        let document = bridge
            .open_document(epub_request(), Cancellation::new())
            .await
            .unwrap();

        let surface = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let raster = surface.raster.expect("EPUB selection owns a raster");
        assert!(raster.byte_len > 0);
        assert_eq!(bridge.admission.buffer_bytes.available_permits(), 0);
        assert!(bridge.release_selection(surface.handle));
        assert_eq!(bridge.admission.buffer_bytes.available_permits(), 0);
        assert_eq!(
            bridge.take_buffer(raster.handle).unwrap().len(),
            raster.byte_len
        );
        assert_eq!(bridge.admission.buffer_bytes.available_permits(), 0);
        assert!(bridge.release_buffer(raster.handle));
        assert_eq!(
            bridge.admission.buffer_bytes.available_permits(),
            buffer_budget
        );
        assert_eq!(
            bridge.take_buffer(raster.handle),
            Err(BridgeError::InvalidBufferHandle)
        );
        assert!(!bridge.release_selection(surface.handle));
    }

    #[tokio::test]
    async fn retained_selection_exhausts_and_releases_request_admission() {
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.request_slots = Arc::new(Semaphore::new(1));
        let admission = Arc::new(admission);
        let bridge = Bridge::with_admission(Arc::clone(&admission));
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();

        let first = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        assert_eq!(admission.request_slots.available_permits(), 0);
        assert_eq!(
            bridge
                .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
                .await,
            Err(BridgeError::RequestLimit)
        );

        assert!(bridge.release_selection(first.handle));
        let second = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        assert!(bridge.release_selection(second.handle));
        assert_eq!(admission.request_slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn retained_pdf_selections_do_not_hold_transient_render_admission() {
        let document = OpenDocument::open(&DeviceFileLocator::from_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.pdf"
        )))
        .unwrap();
        let selection_peak = selection_transient_byte_len(&document, 0, 1.0, false).unwrap();
        let probe_capacity = selection_peak * 2;
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.probe_bytes = Arc::new(Semaphore::new(probe_capacity));
        let admission = Arc::new(admission);
        let bridge = Bridge::with_admission(Arc::clone(&admission));
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let first = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let second = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();

        let rendered = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            bridge.render_page(
                RenderRequest {
                    document: document.handle,
                    page: 0,
                    scale: 1.0,
                },
                Cancellation::new(),
            ),
        )
        .await
        .expect("render admission must not depend on releasing retained selections")
        .unwrap();
        assert!(bridge.release_buffer(rendered.handle));
        assert!(bridge.release_selection(first.handle));
        assert!(bridge.release_selection(second.handle));
        assert_eq!(admission.probe_bytes.available_permits(), probe_capacity);
    }

    #[tokio::test]
    async fn dropped_epub_selection_keeps_admission_until_worker_exits() {
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.request_slots = Arc::new(Semaphore::new(1));
        admission.buffer_slots = Arc::new(Semaphore::new(1));
        let admission = Arc::new(admission);
        let worker_barrier = Arc::new(std::sync::Barrier::new(2));
        let mut bridge = Bridge::with_admission(Arc::clone(&admission));
        bridge.selection_worker_barrier = Some(Arc::clone(&worker_barrier));
        let bridge = Arc::new(bridge);
        let document = bridge
            .open_document(epub_request(), Cancellation::new())
            .await
            .unwrap();
        let (drop_tx, drop_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = std::sync::mpsc::sync_channel(1);
        let operation_bridge = Arc::clone(&bridge);
        let operation = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    tokio::select! {
                        _ = operation_bridge.selection_surface(
                            document.handle,
                            0,
                            1.0,
                            680.0,
                            18.0,
                            Cancellation::new(),
                        ) => panic!("selection must remain blocked"),
                        _ = drop_rx => {}
                    }
                    dropped_tx.send(()).unwrap();
                });
        });

        worker_barrier.wait();
        drop_tx.send(()).unwrap();
        dropped_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("the outer selection future must be dropped");
        assert_eq!(admission.request_slots.available_permits(), 0);
        assert_eq!(admission.buffer_slots.available_permits(), 0);

        worker_barrier.wait();
        operation.join().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while admission.request_slots.available_permits() == 0
                || admission.buffer_slots.available_permits() == 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the detached blocking worker must release its admission");
        assert!(bridge.registry.lock().unwrap().buffers.is_empty());
        assert!(bridge.registry.lock().unwrap().selections.is_empty());
    }

    #[tokio::test]
    async fn dropped_annotation_create_keeps_request_admission_until_worker_exits() {
        let directory = tempfile::tempdir().unwrap();
        let worker_barrier = Arc::new(std::sync::Barrier::new(2));
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.request_slots = Arc::new(Semaphore::new(1));
        let admission = Arc::new(admission);
        let mut bridge = Bridge::with_admission_database(
            Arc::clone(&admission),
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        bridge.annotation_resolution_worker_barrier = Some(Arc::clone(&worker_barrier));
        let bridge = Arc::new(bridge);
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let (drop_tx, drop_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = std::sync::mpsc::sync_channel(1);
        let operation_bridge = Arc::clone(&bridge);
        let operation = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    tokio::select! {
                        _ = operation_bridge.create_annotation(
                            annotation_request(document.handle),
                            Cancellation::new(),
                        ) => panic!("annotation create must remain blocked"),
                        _ = drop_rx => {}
                    }
                    dropped_tx.send(()).unwrap();
                });
        });

        worker_barrier.wait();
        drop_tx.send(()).unwrap();
        dropped_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("the outer annotation create future must be dropped");
        assert_eq!(admission.request_slots.available_permits(), 0);

        worker_barrier.wait();
        operation.join().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while admission.request_slots.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached annotation conversion must release request admission");
        assert_eq!(admission.request_slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn pdf_selection_cancelled_during_worker_reports_cancellation() {
        let cancellation_barrier = Arc::new(std::sync::Barrier::new(2));
        let mut bridge = Bridge::new();
        bridge.selection_second_cancellation_barrier = Some(Arc::clone(&cancellation_barrier));
        let bridge = Arc::new(bridge);
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let operation_bridge = Arc::clone(&bridge);
        let operation_cancellation = cancellation.clone();
        let operation = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(operation_bridge.selection_surface(
                    document.handle,
                    0,
                    1.0,
                    680.0,
                    18.0,
                    operation_cancellation,
                ))
        });

        cancellation_barrier.wait();
        cancellation.cancel();
        cancellation_barrier.wait();
        assert_eq!(operation.join().unwrap(), Err(BridgeError::Cancelled));
    }

    #[tokio::test]
    async fn pdf_annotation_persists_quote_and_underlying_page_rectangles() {
        let directory = tempfile::tempdir().unwrap();
        let bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let surface = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let endpoint = surface
            .endpoints
            .iter()
            .find(|endpoint| endpoint.range_start < endpoint.range_end)
            .copied()
            .unwrap();
        let chars: Vec<_> = surface.text.chars().collect();
        let expected_quote: String = chars[endpoint.range_start..endpoint.range_end]
            .iter()
            .collect();
        assert!(bridge.release_selection(surface.handle));

        bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.handle,
                    unit: 0,
                    start: endpoint.range_start,
                    end: endpoint.range_end,
                    display_scale: 2.0,
                    color: HighlightColor::Yellow,
                    body: None,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let retained = bridge.document(document.handle).unwrap();
        let stored = bridge
            .annotation_store()
            .await
            .unwrap()
            .list_for_local_path_async(&retained.local_path)
            .await
            .unwrap();

        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].quote.as_ref().unwrap().original.as_deref(),
            Some(expected_quote.as_str())
        );
        let AnnotationTarget::Pdf(anchor) = &stored[0].target else {
            panic!("PDF selection must persist a PDF anchor");
        };
        assert_eq!(
            anchor.character_range,
            Some((endpoint.range_start as u32, endpoint.range_end as u32))
        );
        assert!(!anchor.rectangles.is_empty());
        let listed = bridge
            .list_annotations(document.handle, 2.0, Cancellation::new())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].resolution, AnnotationResolution::Exact);
        assert_eq!(
            listed[0].text_range,
            Some(AnnotationTextRange {
                start: endpoint.range_start,
                end: endpoint.range_end,
            })
        );
    }

    #[tokio::test]
    async fn pdf_annotation_creation_uses_the_displayed_scale() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large-page.pdf");
        std::fs::write(&path, selectable_pdf_with_media_box(7_000, 7_000, "scale")).unwrap();
        let bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        let document = bridge
            .open_document(
                OpenRequest {
                    book_id: None,
                    local_id: "large-page".into(),
                    path_key: crate::path_key::path_key(&path),
                    format_hint: Some(BookFormat::Pdf),
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let retained = bridge.document(document.handle).unwrap();
        assert!(matches!(
            selection_transient_byte_len(&retained.document, 0, 1.0, false),
            Err(BridgeError::BufferLimit)
        ));
        let surface = bridge
            .selection_surface(document.handle, 0, 0.1, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let endpoint = surface
            .endpoints
            .iter()
            .find(|endpoint| endpoint.range_start < endpoint.range_end)
            .copied()
            .unwrap();
        assert!(bridge.release_selection(surface.handle));

        let created = bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.handle,
                    unit: 0,
                    start: endpoint.range_start,
                    end: endpoint.range_end,
                    display_scale: 0.1,
                    color: HighlightColor::Yellow,
                    body: None,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(created.resolution, AnnotationResolution::Exact);
        assert_eq!(
            bridge
                .list_annotations(document.handle, 0.1, Cancellation::new())
                .await
                .unwrap(),
            vec![created]
        );
    }

    #[tokio::test]
    async fn annotation_response_preparation_fails_before_persistence() {
        let directory = tempfile::tempdir().unwrap();
        let admission = Arc::new(BridgeAdmission::new(
            ANNOTATION_GEOMETRY_WORKSPACE_BYTES as usize - 1,
            1,
        ));
        let bridge = Bridge::with_admission_database(
            admission,
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let retained = bridge.document(document.handle).unwrap();

        assert_eq!(
            bridge
                .create_annotation(annotation_request(document.handle), Cancellation::new())
                .await,
            Err(BridgeError::BufferLimit)
        );
        assert!(
            bridge
                .annotation_store()
                .await
                .unwrap()
                .list_for_local_path_async(&retained.local_path)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn inserted_annotation_decode_failure_rolls_back() {
        let directory = tempfile::tempdir().unwrap();
        let mut bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
            fail_create_response: true,
            ..AnnotationTestHooks::default()
        }));
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let retained = bridge.document(document.handle).unwrap();

        assert!(matches!(
            bridge
                .create_annotation(annotation_request(document.handle), Cancellation::new())
                .await,
            Err(BridgeError::Storage(message))
                if message.contains("response preparation failure")
        ));
        assert!(
            bridge
                .annotation_store()
                .await
                .unwrap()
                .list_for_local_path_async(&retained.local_path)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn oversized_annotation_bodies_fail_before_async_work() {
        let bridge = Bridge::new();
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let body = "x".repeat(MAX_ANNOTATION_BODY_SCALARS + 1);
        let mut request = annotation_request(document.handle);
        request.body = Some(body.clone());

        assert!(matches!(
            bridge.create_annotation(request, Cancellation::new()).await,
            Err(BridgeError::InvalidRequest(message)) if message.contains("annotation body")
        ));
        assert!(bridge.annotation_store.get().is_none());
        assert!(matches!(
            bridge
                .update_annotation(
                    document.handle,
                    &AnnotationId::new().to_string(),
                    HighlightColor::Green,
                    Some(body),
                )
                .await,
            Err(BridgeError::InvalidRequest(message)) if message.contains("annotation body")
        ));
        assert!(bridge.annotation_store.get().is_none());
    }

    #[tokio::test]
    async fn exact_annotations_survive_oversized_graphemes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized-grapheme.epub");
        let body = format!("ordinary e{}", "\u{301}".repeat(1_025));
        std::fs::write(&path, epub_with_body(&body)).unwrap();
        let bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        let request = || OpenRequest {
            book_id: None,
            local_id: "oversized-grapheme".into(),
            path_key: crate::path_key::path_key(&path),
            format_hint: Some(BookFormat::Epub),
        };
        let document = bridge
            .open_document(request(), Cancellation::new())
            .await
            .unwrap();
        let surface = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let chars = surface.text.chars().collect::<Vec<_>>();
        let start = chars
            .windows("ordinary".len())
            .position(|window| window.iter().collect::<String>() == "ordinary")
            .unwrap();
        let end = start + "ordinary".len();
        assert!(bridge.release_selection(surface.handle));

        let created = bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.handle,
                    unit: 0,
                    start,
                    end,
                    display_scale: 1.0,
                    color: HighlightColor::Yellow,
                    body: None,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(created.resolution, AnnotationResolution::Exact);
        let oversized_start = chars
            .iter()
            .rposition(|character| *character == 'e')
            .unwrap();
        let oversized = bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.handle,
                    unit: 0,
                    start: oversized_start,
                    end: chars.len(),
                    display_scale: 1.0,
                    color: HighlightColor::Green,
                    body: None,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(oversized.resolution, AnnotationResolution::Exact);
        let listed = bridge
            .list_annotations(document.handle, 1.0, Cancellation::new())
            .await
            .unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&created));
        assert!(listed.contains(&oversized));

        assert!(bridge.release_document(document.handle));
        let reopened = bridge
            .open_document(request(), Cancellation::new())
            .await
            .unwrap();
        assert_eq!(
            bridge
                .list_annotations(reopened.handle, 1.0, Cancellation::new())
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn epub_annotation_measurement_ignores_unused_raster_limits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tall-layout.epub");
        let body = "x".repeat(20);
        std::fs::write(&path, epub_with_body(&body)).unwrap();
        let bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        let document = bridge
            .open_document(
                OpenRequest {
                    book_id: None,
                    local_id: "tall-layout".into(),
                    path_key: crate::path_key::path_key(&path),
                    format_hint: Some(BookFormat::Epub),
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let retained = bridge.document(document.handle).unwrap();
        match selection_surface(&retained.document, 0, 1.0, 680.0, 1024.0, true, &|| false) {
            Err(BridgeError::BufferLimit) => {}
            Err(BridgeError::Render(message))
                if message.contains("16777216-pixel per-call ceiling") => {}
            Err(error) => panic!("unexpected raster failure: {error:?}"),
            Ok(extraction) => panic!(
                "default raster unexpectedly fit: {}x{}, text={}, lines={}",
                extraction.raster_width,
                extraction.raster_height,
                extraction.surface.text.chars().count(),
                extraction.surface.visual_lines.len(),
            ),
        }
        let measurement =
            selection_surface(&retained.document, 0, 1.0, 680.0, 1024.0, false, &|| false);
        if let Err(error) = measurement {
            panic!("measurement failed: {error:?}");
        }
        let surface = bridge
            .selection_surface(document.handle, 0, 0.1, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let endpoint = surface
            .endpoints
            .iter()
            .find(|endpoint| endpoint.range_start < endpoint.range_end)
            .copied()
            .unwrap();
        assert!(bridge.release_buffer(surface.raster.unwrap().handle));
        assert!(bridge.release_selection(surface.handle));

        let created = bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.handle,
                    unit: 0,
                    start: endpoint.range_start,
                    end: endpoint.range_end,
                    display_scale: 0.1,
                    color: HighlightColor::Yellow,
                    body: None,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(created.resolution, AnnotationResolution::Exact);
        assert_eq!(
            bridge
                .list_annotations(document.handle, 0.1, Cancellation::new())
                .await
                .unwrap(),
            vec![created]
        );
    }

    #[tokio::test]
    async fn epub_annotation_admits_retained_caret_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("many-caret-lines.epub");
        let body = format!("<p>{}</p>", "i".repeat(65)).repeat(496);
        std::fs::write(&path, epub_with_body(&body)).unwrap();
        let admission = Arc::new(BridgeAdmission::new(
            MAX_BRIDGE_RETAINED_BUFFER_BYTES,
            MAX_BRIDGE_RENDER_WORKERS,
        ));
        let bridge = Bridge::with_admission_database(
            Arc::clone(&admission),
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        let document = bridge
            .open_document(
                OpenRequest {
                    book_id: None,
                    local_id: "many-caret-lines".into(),
                    path_key: crate::path_key::path_key(&path),
                    format_hint: Some(BookFormat::Epub),
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        let initial_probe_bytes = admission.probe_bytes.available_permits();
        let surface = bridge
            .selection_surface(document.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
            .await
            .unwrap();
        let endpoint = surface
            .endpoints
            .iter()
            .find(|endpoint| endpoint.range_start < endpoint.range_end)
            .copied()
            .unwrap();
        assert!(bridge.release_buffer(surface.raster.unwrap().handle));
        assert!(bridge.release_selection(surface.handle));
        assert_eq!(
            admission.probe_bytes.available_permits(),
            initial_probe_bytes
        );

        let created = bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.handle,
                    unit: 0,
                    start: endpoint.range_start,
                    end: endpoint.range_end,
                    display_scale: 1.0,
                    color: HighlightColor::Yellow,
                    body: None,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            created.text_range,
            Some(AnnotationTextRange {
                start: endpoint.range_start,
                end: endpoint.range_end,
            })
        );
        assert_eq!(
            bridge
                .list_annotations(document.handle, 1.0, Cancellation::new())
                .await
                .unwrap(),
            vec![created]
        );
        assert_eq!(
            admission.probe_bytes.available_permits(),
            initial_probe_bytes
        );
    }

    #[test]
    fn exact_snapshot_reuses_its_scalar_index() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exact-snapshot.epub");
        let body = "e\u{301}".repeat(32_767);
        std::fs::write(&path, epub_with_body(&body)).unwrap();
        let document = OpenDocument::open(&DeviceFileLocator::from_path(&path)).unwrap();
        let quote = QuoteSelector::new("e\u{301}", "", "").unwrap();
        let annotations = (0..MAX_ANNOTATIONS_PER_SNAPSHOT)
            .map(|_| Annotation {
                id: AnnotationId::new(),
                book_id: None,
                local_path: Some(path.to_string_lossy().into_owned()),
                fingerprint: DocumentFingerprint::new("sha256", 1, vec![7; 32]).unwrap(),
                quote: Some(quote.clone()),
                target: AnnotationTarget::Epub(
                    EpubAnchor::new(0, "OPS/chapter.xhtml", 65_532, 65_534).unwrap(),
                ),
                color: HighlightColor::Yellow,
                body: None,
                provenance: None,
                created_at: "now".into(),
                modified_at: "now".into(),
                deleted_at: None,
            })
            .collect();

        let resolved = annotation_dtos(annotations, &document, 1.0, true, &|| false).unwrap();
        assert_eq!(resolved.len(), MAX_ANNOTATIONS_PER_SNAPSHOT);
        assert!(
            resolved
                .iter()
                .all(|annotation| annotation.resolution == AnnotationResolution::Exact)
        );
    }

    #[tokio::test]
    async fn cancellation_interrupts_pending_annotation_store_initialization() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(TestPhaseGate::default());
        let admission = Arc::new(BridgeAdmission::new(
            MAX_BRIDGE_RETAINED_BUFFER_BYTES,
            MAX_BRIDGE_RENDER_WORKERS,
        ));
        let mut bridge = Bridge::with_admission_database(
            admission,
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
            initialization: Some(Arc::clone(&gate)),
            ..AnnotationTestHooks::default()
        }));
        let bridge = Arc::new(bridge);
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let document_handle = document.handle;
        let operation_bridge = Arc::clone(&bridge);
        let operation_cancellation = cancellation.clone();
        let operation = tokio::spawn(async move {
            operation_bridge
                .create_annotation(annotation_request(document_handle), operation_cancellation)
                .await
        });

        gate.wait_until_entered().await;
        assert_eq!(
            bridge.admission.request_slots.available_permits(),
            MAX_BRIDGE_REQUESTS - 1
        );
        cancellation.cancel();
        assert_eq!(operation.await.unwrap(), Err(BridgeError::Cancelled));
        assert_eq!(
            bridge.admission.request_slots.available_permits(),
            MAX_BRIDGE_REQUESTS
        );
        gate.release();
        assert!(
            bridge
                .list_annotations(document.handle, 1.0, Cancellation::new())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cancellation_after_extraction_before_acceptance_cannot_persist_annotation() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(TestPhaseGate::default());
        let admission = Arc::new(BridgeAdmission::new(
            MAX_BRIDGE_RETAINED_BUFFER_BYTES,
            MAX_BRIDGE_RENDER_WORKERS,
        ));
        let mut bridge = Bridge::with_admission_database(
            admission,
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        bridge.annotation_store().await.unwrap();
        bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
            before_acceptance: Some(Arc::clone(&gate)),
            ..AnnotationTestHooks::default()
        }));
        let bridge = Arc::new(bridge);
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let document_handle = document.handle;
        let operation_bridge = Arc::clone(&bridge);
        let operation_cancellation = cancellation.clone();
        let operation = tokio::spawn(async move {
            operation_bridge
                .create_annotation(annotation_request(document_handle), operation_cancellation)
                .await
        });

        gate.wait_until_entered().await;
        assert_eq!(
            bridge.admission.request_slots.available_permits(),
            MAX_BRIDGE_REQUESTS - 1
        );
        assert_eq!(
            bridge.admission.probe_bytes.available_permits(),
            MAX_BRIDGE_PROBE_BYTES
        );
        cancellation.cancel();
        gate.release();
        assert_eq!(operation.await.unwrap(), Err(BridgeError::Cancelled));
        assert_eq!(
            bridge.admission.request_slots.available_permits(),
            MAX_BRIDGE_REQUESTS
        );
        assert_eq!(
            bridge.admission.probe_bytes.available_permits(),
            MAX_BRIDGE_PROBE_BYTES
        );
        assert!(
            bridge
                .list_annotations(document.handle, 1.0, Cancellation::new())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn accepted_annotation_finishes_after_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(AnnotationPersistenceTestGate::new());
        let mut bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
            persistence: Some(Arc::clone(&gate)),
            ..AnnotationTestHooks::default()
        }));
        bridge.annotation_store().await.unwrap();
        let bridge = Arc::new(bridge);
        let document = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let document_handle = document.handle;
        let operation_bridge = Arc::clone(&bridge);
        let operation_cancellation = cancellation.clone();
        let operation = tokio::spawn(async move {
            operation_bridge
                .create_annotation(annotation_request(document_handle), operation_cancellation)
                .await
        });

        gate.wait_until_entered().await;
        cancellation.cancel();
        gate.release();
        operation.await.unwrap().unwrap();
        assert_eq!(
            bridge
                .list_annotations(document.handle, 1.0, Cancellation::new())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn annotation_preparation_releases_probe_bytes_before_persistence() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(AnnotationPersistenceTestGate::new());
        let document = OpenDocument::open(&DeviceFileLocator::from_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.pdf"
        )))
        .unwrap();
        let probe_bytes = selection_transient_byte_len(&document, 0, 1.0, false).unwrap();
        let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        admission.probe_bytes = Arc::new(Semaphore::new(probe_bytes));
        let admission = Arc::new(admission);
        let mut bridge = Bridge::with_admission_database(
            Arc::clone(&admission),
            Some(Arc::new(directory.path().join("annotations.sqlite"))),
        );
        bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
            persistence: Some(Arc::clone(&gate)),
            ..AnnotationTestHooks::default()
        }));
        bridge.annotation_store().await.unwrap();
        let bridge = Arc::new(bridge);
        let summary = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let create_bridge = Arc::clone(&bridge);
        let create = tokio::spawn(async move {
            create_bridge
                .create_annotation(annotation_request(summary.handle), Cancellation::new())
                .await
        });
        gate.wait_until_entered().await;
        assert_eq!(admission.probe_bytes.available_permits(), probe_bytes);

        let selection_bridge = Arc::clone(&bridge);
        let selection = tokio::spawn(async move {
            selection_bridge
                .selection_surface(summary.handle, 0, 1.0, 680.0, 18.0, Cancellation::new())
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(admission.render_slots.available_permits(), 0);
        gate.release();

        let surface = tokio::time::timeout(std::time::Duration::from_secs(5), selection)
            .await
            .expect("selection must not deadlock")
            .unwrap()
            .unwrap();
        assert!(bridge.release_selection(surface.handle));
        tokio::time::timeout(std::time::Duration::from_secs(5), create)
            .await
            .expect("accepted create must finish")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_annotation_database_listing() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(AnnotationPersistenceTestGate::new());
        let mut bridge = Bridge::with_database_path(directory.path().join("annotations.sqlite"));
        bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
            list: Some(Arc::clone(&gate)),
            ..AnnotationTestHooks::default()
        }));
        bridge.annotation_store().await.unwrap();
        let bridge = Arc::new(bridge);
        let summary = bridge
            .open_document(pdf_request(), Cancellation::new())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let list_bridge = Arc::clone(&bridge);
        let list_cancellation = cancellation.clone();
        let list = tokio::spawn(async move {
            list_bridge
                .list_annotations(summary.handle, 1.0, list_cancellation)
                .await
        });

        gate.wait_until_entered().await;
        cancellation.cancel();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), list)
                .await
                .expect("cancelled list must not wait for the database gate")
                .unwrap(),
            Err(BridgeError::Cancelled)
        );
    }

    #[tokio::test]
    async fn annotation_admission_spans_blocked_persistence_success_and_failure() {
        for fail in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let gate = Arc::new(AnnotationPersistenceTestGate::new());
            let mut admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
            admission.request_slots = Arc::new(Semaphore::new(1));
            let admission = Arc::new(admission);
            let mut bridge = Bridge::with_admission_database(
                Arc::clone(&admission),
                Some(Arc::new(directory.path().join("annotations.sqlite"))),
            );
            bridge.annotation_test_hooks = Some(Arc::new(AnnotationTestHooks {
                persistence: Some(Arc::clone(&gate)),
                ..AnnotationTestHooks::default()
            }));
            let store = bridge.annotation_store().await.unwrap();
            if fail {
                store
                    .execute_test_sql(
                        "CREATE TRIGGER reject_annotation BEFORE INSERT ON annotations \
                         BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END",
                    )
                    .await
                    .unwrap();
            }
            let bridge = Arc::new(bridge);
            let document = bridge
                .open_document(pdf_request(), Cancellation::new())
                .await
                .unwrap();
            let operation_bridge = Arc::clone(&bridge);
            let operation = tokio::spawn(async move {
                operation_bridge
                    .create_annotation(annotation_request(document.handle), Cancellation::new())
                    .await
            });

            gate.wait_until_entered().await;
            assert_eq!(admission.request_slots.available_permits(), 0);
            assert_eq!(
                admission.probe_bytes.available_permits(),
                MAX_BRIDGE_PROBE_BYTES
            );
            gate.release();
            let result = operation.await.unwrap();
            if fail {
                assert!(matches!(result, Err(BridgeError::Storage(_))));
            } else {
                result.unwrap();
            }
            assert_eq!(admission.request_slots.available_permits(), 1);
            assert_eq!(
                admission.probe_bytes.available_permits(),
                MAX_BRIDGE_PROBE_BYTES
            );
        }
    }

    #[test]
    fn epub_selection_peak_includes_line_assembled_and_geometry_storage() {
        let document = OpenDocument::open(&DeviceFileLocator::from_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.epub"
        )))
        .unwrap();
        let peak = selection_transient_byte_len(&document, 0, 1.0, true).unwrap();
        let two_rasters = EPUB_TEXT_MAX_PIXELS * 4 * 2;

        assert!(peak > two_rasters);
        assert!(selection_transient_byte_len(&document, 0, 1.0, false).unwrap() < peak);
    }

    #[test]
    fn empty_epub_chapter_has_a_decodable_blank_surface() {
        let document = OpenDocument::Epub(Arc::new(
            crate::epub::EpubDoc::from_bytes(empty_epub()).unwrap(),
        ));

        let extraction =
            selection_surface(&document, 0, 2.0, 680.0, 18.0, true, &|| false).unwrap();

        assert_eq!(extraction.raster_width, 1);
        assert!(extraction.raster_height > 0);
        assert_eq!(
            extraction.raster.as_ref().unwrap().len(),
            extraction.raster_height as usize * 4
        );
        assert_eq!(extraction.surface.width, 0.5);
        assert!(extraction.surface.height > 0.0);
        assert!(extraction.surface.text.is_empty());
        assert!(extraction.surface.endpoints.is_empty());
        assert!(
            extraction
                .surface
                .visual_lines
                .iter()
                .all(|line| line.carets.is_empty())
        );
    }

    #[test]
    fn epub_blank_paragraphs_export_real_newline_carets() {
        let document = OpenDocument::Epub(Arc::new(
            crate::epub::EpubDoc::from_bytes(empty_epub()).unwrap(),
        ));
        let OpenDocument::Epub(document) = document else {
            unreachable!()
        };
        let text = "\nAlpha\n\nOmega";
        let layout = document
            .fonts()
            .measure_text(&EpubTextRequest {
                runs: vec![EpubTextRun {
                    text: text.into(),
                    family: None,
                    monospace: false,
                    font_size: 18.0,
                    bold: false,
                    italic: false,
                    foreground: [0, 0, 0, 255],
                    link: None,
                }],
                max_width: 680.0,
                line_height: 27.0,
                scale: 1.0,
                align: EpubTextAlign::Left,
                direction: EpubTextDirection::LeftToRight,
                highlights: Vec::new(),
            })
            .unwrap();
        let visual_lines = epub_visual_lines(text, &layout, 1.0);
        let newline_offsets = text
            .chars()
            .enumerate()
            .filter_map(|(offset, character)| (character == '\n').then_some(offset))
            .collect::<Vec<_>>();
        let blank_lines = visual_lines
            .iter()
            .filter(|line| line.carets.len() == 1)
            .collect::<Vec<_>>();

        assert!(
            blank_lines.len() >= 2,
            "leading and interior blank lines survive: text={text:?}, lines={:?}",
            visual_lines
        );
        assert!(blank_lines.iter().all(|line| {
            newline_offsets.contains(&line.carets[0].offset)
                && line.carets[0].top < line.carets[0].bottom
                && line.carets[0].x.is_finite()
        }));
        assert!(visual_lines.iter().all(|line| !line.carets.is_empty()));

        let terminal_text = "Alpha\n";
        let terminal_layout = document
            .fonts()
            .measure_text(&EpubTextRequest {
                runs: vec![EpubTextRun {
                    text: terminal_text.into(),
                    family: None,
                    monospace: false,
                    font_size: 18.0,
                    bold: false,
                    italic: false,
                    foreground: [0, 0, 0, 255],
                    link: None,
                }],
                max_width: 680.0,
                line_height: 27.0,
                scale: 1.0,
                align: EpubTextAlign::Left,
                direction: EpubTextDirection::LeftToRight,
                highlights: Vec::new(),
            })
            .unwrap();
        let terminal_lines = epub_visual_lines(terminal_text, &terminal_layout, 1.0);
        assert_eq!(
            terminal_lines.last().unwrap().carets[0].offset,
            terminal_text.chars().count()
        );
        assert!(terminal_lines.iter().all(|line| !line.carets.is_empty()));
    }

    #[test]
    fn epub_chapter_text_is_bounded_before_ownership_clone() {
        let oversized = "x".repeat(EPUB_TEXT_MAX_SCALARS + 1);
        assert_eq!(
            bounded_epub_selection_text(&oversized, &|| false),
            Err(BridgeError::BufferLimit)
        );
        assert_eq!(
            bounded_epub_selection_text("chapter", &|| true),
            Err(BridgeError::Cancelled)
        );
    }

    #[test]
    fn epub_retention_charge_includes_bounded_expansion() {
        let encoded = 1024;

        let retained = retained_document_byte_len(BookFormat::Epub, encoded).unwrap();

        assert!(retained > encoded);
        assert!(
            retained
                >= usize::try_from(EpubLimits::default().max_total_uncompressed_bytes).unwrap()
        );
        assert!(retained <= MAX_BRIDGE_RETAINED_DOCUMENT_BYTES);
    }

    #[tokio::test]
    async fn parsed_epub_charge_allows_two_retained_documents() {
        let bridge = Bridge::new();
        let first = bridge
            .open_document(epub_request(), Cancellation::new())
            .await
            .unwrap();

        let second = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            bridge.open_document(epub_request(), Cancellation::new()),
        )
        .await
        .expect("a small second EPUB must not wait for the first handle")
        .unwrap();

        assert!(bridge.release_document(first.handle));
        assert!(bridge.release_document(second.handle));
    }

    #[test]
    fn cbz_retention_charge_includes_copied_directory_names_and_indexes() {
        let encoded = 1024;

        let retained = retained_document_byte_len(BookFormat::Cbz, encoded).unwrap();

        assert!(retained > encoded * 2);
    }

    #[tokio::test]
    async fn queued_document_users_keep_retention_admission() {
        let admission = Arc::new(BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1));
        let bridge = Bridge::with_admission(Arc::clone(&admission));
        let summary = bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        let retained = bridge.document(summary.handle).unwrap();
        let admitted = admission.document_bytes.available_permits();

        assert!(bridge.release_document(summary.handle));
        assert_eq!(admission.document_bytes.available_permits(), admitted);
        drop(retained);
        assert_eq!(
            admission.document_bytes.available_permits(),
            MAX_BRIDGE_RETAINED_DOCUMENT_BYTES
        );
    }

    #[tokio::test]
    async fn byte_admission_precedes_open_worker_and_parsing() {
        let mut configured = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 1);
        configured.open_slots = Arc::new(Semaphore::new(2));
        configured.document_bytes = Arc::new(Semaphore::new(0));
        let admission = Arc::new(configured);
        let bridge = Bridge::with_admission(Arc::clone(&admission));
        let cancellation = Cancellation::new();
        let mut opens = Vec::new();
        for _ in 0..3 {
            let bridge = bridge.clone();
            let cancellation = cancellation.clone();
            opens.push(tokio::spawn(async move {
                bridge.open_document(epub_request(), cancellation).await
            }));
        }

        tokio::task::yield_now().await;
        assert_eq!(admission.open_slots.available_permits(), 2);

        cancellation.cancel();
        for open in opens {
            assert_eq!(open.await.unwrap(), Err(BridgeError::Cancelled));
        }
        assert_eq!(admission.open_slots.available_permits(), 2);
    }

    #[tokio::test]
    async fn cancellation_prevents_opening_and_allocating_handles() {
        let bridge = Bridge::new();
        let cancellation = Cancellation::new();
        cancellation.cancel();

        let error = bridge
            .open_document(cbz_request(), cancellation)
            .await
            .unwrap_err();

        assert_eq!(error, BridgeError::Cancelled);
        assert!(bridge.registry.lock().unwrap().documents.is_empty());
    }

    #[tokio::test]
    async fn missing_documents_have_a_stable_not_found_category() {
        let bridge = Bridge::new();
        let mut request = cbz_request();
        request.path_key = "/definitely/missing/shosai-book.cbz".to_owned();

        let error = bridge
            .open_document(request, Cancellation::new())
            .await
            .unwrap_err();

        assert_eq!(error, BridgeError::DocumentNotFound);
        assert_eq!(error.kind(), BridgeErrorKind::NotFound);
    }

    #[tokio::test]
    async fn malformed_reserved_path_keys_are_invalid_requests() {
        let bridge = Bridge::new();
        let mut request = cbz_request();
        request.path_key = "\0unix-path-v1:not-hex".to_owned();

        let error = bridge
            .open_document(request, Cancellation::new())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), BridgeErrorKind::InvalidRequest);
    }

    #[tokio::test]
    async fn oversized_request_strings_are_rejected_before_document_admission() {
        let bridge = Bridge::new();
        let mut local_id = cbz_request();
        local_id.local_id = "x".repeat(MAX_BRIDGE_LOCAL_ID_BYTES + 1);
        let mut path_key = cbz_request();
        path_key.path_key = "x".repeat(MAX_BRIDGE_PATH_KEY_BYTES + 1);

        for request in [local_id, path_key] {
            assert!(matches!(
                bridge.open_document(request, Cancellation::new()).await,
                Err(BridgeError::InvalidRequest(_))
            ));
        }
        assert!(bridge.registry.lock().unwrap().documents.is_empty());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[tokio::test]
    async fn open_requests_decode_lossless_native_path_keys() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(OsStr::from_bytes(b"book-\x80.cbz"));
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.cbz"),
            &path,
        )
        .unwrap();
        let request = OpenRequest {
            book_id: None,
            local_id: "native-path".to_owned(),
            path_key: crate::path_key::path_key(&path),
            format_hint: Some(BookFormat::Cbz),
        };

        let summary = Bridge::new()
            .open_document(request, Cancellation::new())
            .await
            .unwrap();

        assert_eq!(summary.format, BookFormat::Cbz);
    }

    #[test]
    fn cancellation_waits_for_the_publication_barrier() {
        let cancellation = Cancellation::new();
        let publication = cancellation.0.publication.lock().unwrap();
        let cancelling = cancellation.clone();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            cancelling.cancel();
            finished_tx.send(()).unwrap();
        });

        assert!(
            finished_rx
                .recv_timeout(std::time::Duration::from_millis(20))
                .is_err()
        );
        drop(publication);
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        thread.join().unwrap();
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn owned_buffers_and_documents_are_released_deterministically() {
        let bridge = Bridge::new();
        let document = bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        assert_eq!(document.logical_unit, LogicalUnit::Page);
        let rendered = bridge
            .render_page(
                RenderRequest {
                    document: document.handle,
                    page: 0,
                    scale: 1.0,
                },
                Cancellation::new(),
            )
            .await
            .unwrap();

        let bytes = bridge.take_buffer(rendered.handle).unwrap();
        assert_eq!(bytes.len(), rendered.byte_len);
        assert_eq!(
            bridge.take_buffer(rendered.handle),
            Err(BridgeError::InvalidBufferHandle)
        );
        assert!(bridge.release_buffer(rendered.handle));
        assert!(bridge.release_document(document.handle));
        assert!(!bridge.release_document(document.handle));
    }

    #[tokio::test]
    async fn retained_and_in_flight_buffers_share_one_budget() {
        let bridge = Bridge::with_limits(8, 1);
        let document = bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        let permit = Arc::clone(&bridge.admission.buffer_bytes)
            .acquire_many_owned(8)
            .await
            .unwrap();
        let slot = Arc::clone(&bridge.admission.buffer_slots)
            .try_acquire_owned()
            .unwrap();
        let buffer = bridge
            .store_buffer(
                document.handle,
                RenderedPage {
                    width: 1,
                    height: 1,
                    pixels: vec![0; 4].into(),
                },
                permit,
                slot,
            )
            .unwrap();
        assert_eq!(bridge.admission.buffer_bytes.available_permits(), 0);
        let retained_pointer = bridge.registry.lock().unwrap().buffers[&buffer.handle]
            .pixels
            .as_ptr();
        let transferred = bridge.take_buffer(buffer.handle).unwrap();
        assert_ne!(transferred.as_ptr(), retained_pointer);
        assert_eq!(bridge.admission.buffer_bytes.available_permits(), 0);
        assert!(bridge.release_buffer(buffer.handle));
        assert_eq!(bridge.admission.buffer_bytes.available_permits(), 8);
    }

    #[tokio::test]
    async fn annotation_resolution_workspace_caps_peak_concurrency() {
        const INDEX_AND_TEXT_BYTES_PER_INPUT_BYTE: usize = 64;
        const NORMALIZATION_AND_MATCHER_OVERHEAD: usize = 16 * 1024 * 1024;
        assert!(
            MAX_ANNOTATION_PDF_TEXT_BYTES * INDEX_AND_TEXT_BYTES_PER_INPUT_BYTE
                + NORMALIZATION_AND_MATCHER_OVERHEAD
                <= ANNOTATION_RESOLUTION_WORKSPACE_BYTES as usize
        );
        assert!(ANNOTATION_RESOLUTION_WORKSPACE_BYTES as usize <= MAX_BRIDGE_BUFFER_BYTES);

        let admission = BridgeAdmission::new(MAX_BRIDGE_RETAINED_BUFFER_BYTES, 3);
        let cancellation = Cancellation::new();
        let first = acquire_permits(
            Arc::clone(&admission.buffer_bytes),
            ANNOTATION_RESOLUTION_WORKSPACE_BYTES,
            &cancellation,
        )
        .await
        .unwrap();
        let second = acquire_permits(
            Arc::clone(&admission.buffer_bytes),
            ANNOTATION_RESOLUTION_WORKSPACE_BYTES,
            &cancellation,
        )
        .await
        .unwrap();
        let third = acquire_permits(
            Arc::clone(&admission.buffer_bytes),
            ANNOTATION_RESOLUTION_WORKSPACE_BYTES,
            &cancellation,
        );
        tokio::pin!(third);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut third)
                .await
                .is_err()
        );
        drop(first);
        let _third = tokio::time::timeout(std::time::Duration::from_secs(1), third)
            .await
            .expect("a third workspace must be admitted after one completes")
            .unwrap();
        drop(second);
    }

    #[tokio::test]
    async fn released_document_cannot_publish_an_in_flight_result() {
        let bridge = Bridge::with_limits(8, 1);
        let document = bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();
        let permit = Arc::clone(&bridge.admission.buffer_bytes)
            .acquire_many_owned(8)
            .await
            .unwrap();
        let slot = Arc::clone(&bridge.admission.buffer_slots)
            .try_acquire_owned()
            .unwrap();
        assert!(bridge.release_document(document.handle));

        assert_eq!(
            bridge.store_buffer(
                document.handle,
                RenderedPage {
                    width: 1,
                    height: 1,
                    pixels: vec![0; 4].into(),
                },
                permit,
                slot,
            ),
            Err(BridgeError::InvalidDocumentHandle)
        );
        assert!(bridge.registry.lock().unwrap().buffers.is_empty());
    }

    #[tokio::test]
    async fn cancellation_interrupts_waiting_for_buffer_budget() {
        let bridge = Bridge::with_limits(4, 1);
        let _permit = Arc::clone(&bridge.admission.buffer_bytes)
            .acquire_many_owned(4)
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        let waiting = acquire_permits(Arc::clone(&bridge.admission.buffer_bytes), 4, &cancellation);
        tokio::pin!(waiting);
        cancellation.cancel();

        assert_eq!(waiting.await.unwrap_err(), BridgeError::Cancelled);
    }

    #[tokio::test]
    async fn cancellation_cannot_be_lost_while_waiters_register() {
        for _ in 0..1_000 {
            let cancellation = Cancellation::new();
            let waiting = cancellation.cancelled();
            tokio::pin!(waiting);
            cancellation.cancel();
            tokio::time::timeout(std::time::Duration::from_millis(100), waiting)
                .await
                .expect("a cancellation notification must not be lost");
        }
    }

    #[tokio::test]
    async fn handles_cannot_be_used_with_another_bridge_registry() {
        let first = Bridge::new();
        let second = Bridge::new();
        let document = first
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();

        assert_eq!(
            second
                .render_page(
                    RenderRequest {
                        document: document.handle,
                        page: 0,
                        scale: 1.0,
                    },
                    Cancellation::new(),
                )
                .await,
            Err(BridgeError::InvalidDocumentHandle)
        );
    }

    #[tokio::test]
    async fn invalid_pages_and_oversized_scales_fail_before_rendering() {
        let bridge = Bridge::new();
        let document = bridge
            .open_document(cbz_request(), Cancellation::new())
            .await
            .unwrap();

        assert!(matches!(
            bridge
                .render_page(
                    RenderRequest {
                        document: document.handle,
                        page: usize::MAX,
                        scale: 1.0,
                    },
                    Cancellation::new(),
                )
                .await,
            Err(BridgeError::InvalidPage { .. })
        ));
        assert_eq!(
            bridge
                .render_page(
                    RenderRequest {
                        document: document.handle,
                        page: 0,
                        scale: 100_000.0,
                    },
                    Cancellation::new(),
                )
                .await,
            Err(BridgeError::BufferLimit)
        );
    }
}
