//! Owned, coarse-grained API suitable for a generated Dart/Rust bridge.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use thiserror::Error;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

use crate::annotations::{
    Annotation, AnnotationId, AnnotationStore, AnnotationTarget, DocumentFingerprint, EpubAnchor,
    HighlightColor, NewAnnotation, PageRect, PdfAnchor, QuoteSelector,
};

use crate::application::{DeviceFileLocator, OpenDocument, OpenDocumentError, OpenDocumentPlan};
use crate::document::RenderedPage;
#[cfg(test)]
use crate::epub::EpubLimits;
use crate::epub::{EpubTextAlign, EpubTextDirection, EpubTextRequest, EpubTextRun};
use crate::library::BookFormat;

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

/// Owned visible-surface text and hit zones. Pointer movement consumes this
/// value locally and never re-enters PDFium or Rust.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSurface {
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub resource_path: Option<String>,
    pub endpoints: Vec<SelectionEndpoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAnnotation {
    pub id: String,
    pub unit: usize,
    pub start: usize,
    pub end: usize,
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

#[derive(Debug, Default)]
struct Registry {
    documents: HashMap<DocumentHandle, Arc<RetainedDocument>>,
    buffers: HashMap<BufferHandle, RetainedBuffer>,
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

    #[cfg(test)]
    fn with_limits(buffer_bytes: usize, render_workers: usize) -> Self {
        Self::with_admission(Arc::new(BridgeAdmission::new(buffer_bytes, render_workers)))
    }

    fn with_admission(admission: Arc<BridgeAdmission>) -> Self {
        Self {
            registry_id: NEXT_REGISTRY_ID.fetch_add(1, Ordering::Relaxed),
            next_handle: Arc::new(AtomicU64::new(0)),
            registry: Arc::new(Mutex::new(Registry::default())),
            admission,
            annotation_store: Arc::new(tokio::sync::OnceCell::new()),
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
                let state = crate::reading_state::ReadingStateStore::open_async_deferred_backfill()
                    .await
                    .map_err(|error| BridgeError::Storage(error.to_string()))?;
                Ok(AnnotationStore::new(state.pool().clone()))
            })
            .await
    }

    pub async fn create_annotation(
        &self,
        document: DocumentHandle,
        unit: usize,
        start: usize,
        end: usize,
        color: HighlightColor,
        body: Option<String>,
    ) -> Result<BridgeAnnotation, BridgeError> {
        if start >= end {
            return Err(BridgeError::InvalidRequest(
                "annotation range must be non-empty".into(),
            ));
        }
        let retained = self.document(document)?;
        let surface = selection_surface(&retained.document, unit, 1.0, 680.0, 18.0, &|| false)?;
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
        let target = match &retained.document {
            OpenDocument::Epub(_) => AnnotationTarget::Epub(
                EpubAnchor::new(
                    u32::try_from(unit)
                        .map_err(|_| BridgeError::InvalidRequest("unit exceeds range".into()))?,
                    surface.resource_path.as_deref().ok_or_else(|| {
                        BridgeError::InvalidRequest("EPUB resource path missing".into())
                    })?,
                    u32::try_from(start)
                        .map_err(|_| BridgeError::InvalidRequest("range exceeds range".into()))?,
                    u32::try_from(end)
                        .map_err(|_| BridgeError::InvalidRequest("range exceeds range".into()))?,
                )
                .map_err(storage_error)?,
            ),
            OpenDocument::Pdf(document) => {
                let rectangles = document
                    .selection_snapshot(unit, 1.0)
                    .map_err(map_render_error)?
                    .page_rectangles(start, end)
                    .into_iter()
                    .map(|(left, bottom, right, top)| {
                        PageRect::new(left, bottom, right, top).map_err(storage_error)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                AnnotationTarget::Pdf(
                    PdfAnchor::new(
                        u32::try_from(unit).map_err(|_| {
                            BridgeError::InvalidRequest("unit exceeds range".into())
                        })?,
                        Some((u32::try_from(start).unwrap(), u32::try_from(end).unwrap())),
                        rectangles,
                    )
                    .map_err(storage_error)?,
                )
            }
            OpenDocument::Cbz(_) => return Err(BridgeError::UnsupportedOperation(BookFormat::Cbz)),
        };
        let created = self
            .annotation_store()
            .await?
            .create_async(&NewAnnotation {
                id: AnnotationId::new(),
                book_id: None,
                local_path: Some(retained.local_path.clone()),
                fingerprint: retained.fingerprint.clone(),
                quote: Some(quote),
                target,
                color,
                body,
                provenance: None,
            })
            .await
            .map_err(storage_error)?;
        Ok(annotation_dto(created))
    }

    pub async fn list_annotations(
        &self,
        document: DocumentHandle,
    ) -> Result<Vec<BridgeAnnotation>, BridgeError> {
        let retained = self.document(document)?;
        self.annotation_store()
            .await?
            .list_for_local_path_async(&retained.local_path)
            .await
            .map_err(storage_error)
            .map(|items| {
                items
                    .into_iter()
                    .filter(|item| item.fingerprint == retained.fingerprint)
                    .map(annotation_dto)
                    .collect()
            })
    }

    pub async fn update_annotation(
        &self,
        document: DocumentHandle,
        id: &str,
        color: HighlightColor,
        body: Option<String>,
    ) -> Result<bool, BridgeError> {
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
            .map_err(storage_error)
    }

    pub async fn delete_annotation(
        &self,
        document: DocumentHandle,
        id: &str,
    ) -> Result<bool, BridgeError> {
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
        let _request_slot = try_acquire_slot(
            Arc::clone(&self.admission.request_slots),
            BridgeError::RequestLimit,
        )?;
        let render_slot =
            acquire_permits(Arc::clone(&self.admission.render_slots), 1, &cancellation).await?;
        let retained = self.document(document)?;
        let worker_cancellation = cancellation.clone();
        let (surface, guards) = tokio::task::spawn_blocking(move || {
            let surface = guarded(|| {
                selection_surface(&retained.document, unit, scale, width, font_size, &|| {
                    worker_cancellation.is_cancelled()
                })
            });
            (surface, (_request_slot, render_slot))
        })
        .await
        .map_err(|_| BridgeError::Worker)?;
        drop(guards);
        check_cancelled(&cancellation)?;
        surface
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
    is_cancelled: &dyn Fn() -> bool,
) -> Result<SelectionSurface, BridgeError> {
    if is_cancelled() {
        return Err(BridgeError::Cancelled);
    }
    match document {
        OpenDocument::Pdf(document) => {
            let snapshot = document
                .selection_snapshot(unit, scale)
                .map_err(map_render_error)?;
            let (bitmap_width, bitmap_height) = snapshot.bitmap_size();
            let text = document.page_text(unit).map_err(map_render_error)?;
            Ok(SelectionSurface {
                width: bitmap_width as f32,
                height: bitmap_height as f32,
                text,
                resource_path: None,
                endpoints: snapshot
                    .endpoints()
                    .into_iter()
                    .map(|(rect, endpoint)| SelectionEndpoint {
                        offset: endpoint.character,
                        range_start: endpoint.character,
                        range_end: endpoint.character.saturating_add(1),
                        rect: SelectionRect {
                            left: rect.left,
                            top: rect.top,
                            right: rect.right,
                            bottom: rect.bottom,
                        },
                    })
                    .collect(),
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
            let text = chapter.search_text().to_owned();
            let layout = document
                .fonts()
                .measure_text(&EpubTextRequest {
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
                })
                .map_err(map_render_error)?;
            let path = document.chapter(unit).map(|chapter| chapter.path.clone());
            Ok(SelectionSurface {
                width: layout.width,
                height: layout.height,
                text,
                resource_path: path,
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
            })
        }
        OpenDocument::Cbz(_) => Err(BridgeError::UnsupportedOperation(BookFormat::Cbz)),
    }
}

fn storage_error(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::Storage(error.to_string())
}

fn annotation_dto(annotation: Annotation) -> BridgeAnnotation {
    let (unit, start, end) = match annotation.target {
        AnnotationTarget::Epub(anchor) => (
            anchor.spine_occurrence as usize,
            anchor.scalar_start as usize,
            anchor.scalar_end as usize,
        ),
        AnnotationTarget::Pdf(anchor) => {
            let (start, end) = anchor.character_range.unwrap_or((0, 0));
            (anchor.page as usize, start as usize, end as usize)
        }
    };
    BridgeAnnotation {
        id: annotation.id.to_string(),
        unit,
        start,
        end,
        color: annotation.color,
        body: annotation.body,
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
    use super::*;

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
