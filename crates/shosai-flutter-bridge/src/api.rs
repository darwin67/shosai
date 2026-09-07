use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use shosai_core::annotations::HighlightColor;
use shosai_core::bridge::{
    Bridge, BridgeAnnotation, BridgeError, BufferHandle, Cancellation, CreateAnnotationRequest,
    DocumentHandle, OpenRequest, RenderRequest, SelectionHandle, SelectionSurface,
};
use shosai_core::library::BookFormat;
use thiserror::Error;

const MAX_CANCELLATIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlutterBookFormat {
    Pdf,
    Epub,
    Cbz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlutterHighlightColor {
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
}

impl From<FlutterHighlightColor> for HighlightColor {
    fn from(value: FlutterHighlightColor) -> Self {
        match value {
            FlutterHighlightColor::Yellow => Self::Yellow,
            FlutterHighlightColor::Green => Self::Green,
            FlutterHighlightColor::Blue => Self::Blue,
            FlutterHighlightColor::Pink => Self::Pink,
            FlutterHighlightColor::Purple => Self::Purple,
        }
    }
}
impl From<HighlightColor> for FlutterHighlightColor {
    fn from(value: HighlightColor) -> Self {
        match value {
            HighlightColor::Yellow => Self::Yellow,
            HighlightColor::Green => Self::Green,
            HighlightColor::Blue => Self::Blue,
            HighlightColor::Pink => Self::Pink,
            HighlightColor::Purple => Self::Purple,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlutterAnnotation {
    pub id: String,
    pub unit: usize,
    pub start: usize,
    pub end: usize,
    pub color: FlutterHighlightColor,
    pub body: Option<String>,
}
impl From<BridgeAnnotation> for FlutterAnnotation {
    fn from(value: BridgeAnnotation) -> Self {
        Self {
            id: value.id,
            unit: value.unit,
            start: value.start,
            end: value.end,
            color: value.color.into(),
            body: value.body,
        }
    }
}

impl From<FlutterBookFormat> for BookFormat {
    fn from(value: FlutterBookFormat) -> Self {
        match value {
            FlutterBookFormat::Pdf => Self::Pdf,
            FlutterBookFormat::Epub => Self::Epub,
            FlutterBookFormat::Cbz => Self::Cbz,
        }
    }
}

impl From<BookFormat> for FlutterBookFormat {
    fn from(value: BookFormat) -> Self {
        match value {
            BookFormat::Pdf => Self::Pdf,
            BookFormat::Epub => Self::Epub,
            BookFormat::Cbz => Self::Cbz,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlutterOpenRequest {
    pub local_id: String,
    pub path_key: String,
    pub format_hint: Option<FlutterBookFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlutterDocumentHandle {
    pub registry: u64,
    pub id: u64,
}

impl From<DocumentHandle> for FlutterDocumentHandle {
    fn from(value: DocumentHandle) -> Self {
        Self {
            registry: value.registry,
            id: value.id,
        }
    }
}

impl From<FlutterDocumentHandle> for DocumentHandle {
    fn from(value: FlutterDocumentHandle) -> Self {
        Self {
            registry: value.registry,
            id: value.id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlutterBufferHandle {
    pub registry: u64,
    pub id: u64,
}

impl From<BufferHandle> for FlutterBufferHandle {
    fn from(value: BufferHandle) -> Self {
        Self {
            registry: value.registry,
            id: value.id,
        }
    }
}

impl From<FlutterBufferHandle> for BufferHandle {
    fn from(value: FlutterBufferHandle) -> Self {
        Self {
            registry: value.registry,
            id: value.id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlutterSelectionHandle {
    pub registry: u64,
    pub id: u64,
}

impl From<SelectionHandle> for FlutterSelectionHandle {
    fn from(value: SelectionHandle) -> Self {
        Self {
            registry: value.registry,
            id: value.id,
        }
    }
}

impl From<FlutterSelectionHandle> for SelectionHandle {
    fn from(value: FlutterSelectionHandle) -> Self {
        Self {
            registry: value.registry,
            id: value.id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlutterDocumentSummary {
    pub handle: FlutterDocumentHandle,
    pub format: FlutterBookFormat,
    pub title: Option<String>,
    pub logical_unit_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlutterRenderedBuffer {
    pub handle: FlutterBufferHandle,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlutterSelectionRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlutterSelectionEndpoint {
    pub offset: usize,
    pub range_start: usize,
    pub range_end: usize,
    pub rect: FlutterSelectionRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlutterSelectionCaret {
    pub offset: usize,
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlutterSelectionVisualLine {
    pub carets: Vec<FlutterSelectionCaret>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlutterSelectionSurface {
    pub handle: FlutterSelectionHandle,
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub copy_eligible: bool,
    pub resource_path: Option<String>,
    pub raster: Option<FlutterRenderedBuffer>,
    pub endpoints: Vec<FlutterSelectionEndpoint>,
    pub grapheme_boundaries: Vec<u32>,
    pub word_boundaries: Vec<u32>,
    pub visual_lines: Vec<FlutterSelectionVisualLine>,
}

impl From<SelectionSurface> for FlutterSelectionSurface {
    fn from(value: SelectionSurface) -> Self {
        Self {
            handle: value.handle.into(),
            width: value.width,
            height: value.height,
            text: value.text,
            copy_eligible: value.copy_eligible,
            resource_path: value.resource_path,
            raster: value.raster.map(|raster| FlutterRenderedBuffer {
                handle: raster.handle.into(),
                width: raster.width,
                height: raster.height,
                byte_len: raster.byte_len,
            }),
            endpoints: value
                .endpoints
                .into_iter()
                .map(|endpoint| FlutterSelectionEndpoint {
                    offset: endpoint.offset,
                    range_start: endpoint.range_start,
                    range_end: endpoint.range_end,
                    rect: FlutterSelectionRect {
                        left: endpoint.rect.left,
                        top: endpoint.rect.top,
                        right: endpoint.rect.right,
                        bottom: endpoint.rect.bottom,
                    },
                })
                .collect(),
            grapheme_boundaries: value
                .grapheme_boundaries
                .into_iter()
                .map(|offset| offset as u32)
                .collect(),
            word_boundaries: value
                .word_boundaries
                .into_iter()
                .map(|offset| offset as u32)
                .collect(),
            visual_lines: value
                .visual_lines
                .into_iter()
                .map(|line| FlutterSelectionVisualLine {
                    carets: line
                        .carets
                        .into_iter()
                        .map(|caret| FlutterSelectionCaret {
                            offset: caret.offset,
                            x: caret.x,
                            top: caret.top,
                            bottom: caret.bottom,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlutterBridgeErrorKind {
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

#[derive(Debug, Error)]
#[error("{message}")]
pub struct FlutterBridgeError {
    pub kind: FlutterBridgeErrorKind,
    pub message: String,
}

impl From<BridgeError> for FlutterBridgeError {
    fn from(value: BridgeError) -> Self {
        let kind = match value.kind() {
            shosai_core::bridge::BridgeErrorKind::Cancelled => FlutterBridgeErrorKind::Cancelled,
            shosai_core::bridge::BridgeErrorKind::NotFound => FlutterBridgeErrorKind::NotFound,
            shosai_core::bridge::BridgeErrorKind::Inaccessible => {
                FlutterBridgeErrorKind::Inaccessible
            }
            shosai_core::bridge::BridgeErrorKind::Unsupported => {
                FlutterBridgeErrorKind::Unsupported
            }
            shosai_core::bridge::BridgeErrorKind::InvalidRequest => {
                FlutterBridgeErrorKind::InvalidRequest
            }
            shosai_core::bridge::BridgeErrorKind::Malformed => FlutterBridgeErrorKind::Malformed,
            shosai_core::bridge::BridgeErrorKind::LimitExceeded => {
                FlutterBridgeErrorKind::LimitExceeded
            }
            shosai_core::bridge::BridgeErrorKind::BackendUnavailable => {
                FlutterBridgeErrorKind::BackendUnavailable
            }
            shosai_core::bridge::BridgeErrorKind::RenderFailed => {
                FlutterBridgeErrorKind::RenderFailed
            }
        };
        Self {
            kind,
            message: value.to_string(),
        }
    }
}

#[derive(Debug)]
pub struct FlutterBridge {
    bridge: Bridge,
    next_cancellation: AtomicU64,
    cancellations: Mutex<HashMap<u64, Cancellation>>,
}

impl Default for FlutterBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl FlutterBridge {
    #[flutter_rust_bridge::frb(sync)]
    pub fn new() -> Self {
        Self::from_bridge(Bridge::new())
    }

    /// Construct a bridge with a host-provided SQLite database path.
    #[flutter_rust_bridge::frb(sync)]
    pub fn with_database_path(database_path: String) -> Self {
        Self::from_bridge(Bridge::with_database_path(database_path.into()))
    }

    fn from_bridge(bridge: Bridge) -> Self {
        Self {
            bridge,
            next_cancellation: AtomicU64::new(1),
            cancellations: Mutex::new(HashMap::new()),
        }
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn create_cancellation(&self) -> Result<u64, FlutterBridgeError> {
        let mut cancellations = self
            .cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cancellations.len() >= MAX_CANCELLATIONS {
            return Err(invalid_request("too many cancellation tokens"));
        }
        let id = self.next_cancellation.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            return Err(invalid_request("cancellation token IDs are exhausted"));
        }
        cancellations.insert(id, Cancellation::new());
        Ok(id)
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn cancel(&self, id: u64) -> bool {
        let cancellation = self
            .cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
            .cloned();
        cancellation.is_some_and(|cancellation| {
            cancellation.cancel();
            true
        })
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn release_cancellation(&self, id: u64) -> bool {
        self.cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id)
            .is_some()
    }

    pub async fn open_document(
        &self,
        request: FlutterOpenRequest,
        cancellation_id: u64,
    ) -> Result<FlutterDocumentSummary, FlutterBridgeError> {
        let cancellation = self.cancellation(cancellation_id)?;
        let summary = self
            .bridge
            .open_document(
                OpenRequest {
                    book_id: None,
                    local_id: request.local_id,
                    path_key: request.path_key,
                    format_hint: request.format_hint.map(Into::into),
                },
                cancellation,
            )
            .await?;
        Ok(FlutterDocumentSummary {
            handle: summary.handle.into(),
            format: summary.format.into(),
            title: summary.title,
            logical_unit_count: summary.logical_unit_count,
        })
    }

    pub async fn render_page(
        &self,
        document: FlutterDocumentHandle,
        page: usize,
        scale: f32,
        cancellation_id: u64,
    ) -> Result<FlutterRenderedBuffer, FlutterBridgeError> {
        let cancellation = self.cancellation(cancellation_id)?;
        let rendered = self
            .bridge
            .render_page(
                RenderRequest {
                    document: document.into(),
                    page,
                    scale,
                },
                cancellation,
            )
            .await?;
        Ok(FlutterRenderedBuffer {
            handle: rendered.handle.into(),
            width: rendered.width,
            height: rendered.height,
            byte_len: rendered.byte_len,
        })
    }

    pub async fn selection_surface(
        &self,
        document: FlutterDocumentHandle,
        unit: usize,
        scale: f32,
        width: f32,
        font_size: f32,
        cancellation_id: u64,
    ) -> Result<FlutterSelectionSurface, FlutterBridgeError> {
        let cancellation = self.cancellation(cancellation_id)?;
        self.bridge
            .selection_surface(document.into(), unit, scale, width, font_size, cancellation)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)] // FRB exposes these as named Dart arguments.
    pub async fn create_annotation(
        &self,
        document: FlutterDocumentHandle,
        unit: usize,
        start: usize,
        end: usize,
        color: FlutterHighlightColor,
        body: Option<String>,
        cancellation_id: u64,
    ) -> Result<FlutterAnnotation, FlutterBridgeError> {
        let cancellation = self.cancellation(cancellation_id)?;
        self.bridge
            .create_annotation(
                CreateAnnotationRequest {
                    document: document.into(),
                    unit,
                    start,
                    end,
                    color: color.into(),
                    body,
                },
                cancellation,
            )
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    pub async fn list_annotations(
        &self,
        document: FlutterDocumentHandle,
    ) -> Result<Vec<FlutterAnnotation>, FlutterBridgeError> {
        self.bridge
            .list_annotations(document.into())
            .await
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    pub async fn update_annotation(
        &self,
        document: FlutterDocumentHandle,
        id: String,
        color: FlutterHighlightColor,
        body: Option<String>,
    ) -> Result<bool, FlutterBridgeError> {
        self.bridge
            .update_annotation(document.into(), &id, color.into(), body)
            .await
            .map_err(Into::into)
    }

    pub async fn delete_annotation(
        &self,
        document: FlutterDocumentHandle,
        id: String,
    ) -> Result<bool, FlutterBridgeError> {
        self.bridge
            .delete_annotation(document.into(), &id)
            .await
            .map_err(Into::into)
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn take_buffer(&self, handle: FlutterBufferHandle) -> Result<Vec<u8>, FlutterBridgeError> {
        self.bridge.take_buffer(handle.into()).map_err(Into::into)
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn release_document(&self, handle: FlutterDocumentHandle) -> bool {
        self.bridge.release_document(handle.into())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn release_buffer(&self, handle: FlutterBufferHandle) -> bool {
        self.bridge.release_buffer(handle.into())
    }

    #[flutter_rust_bridge::frb(sync)]
    pub fn release_selection(&self, handle: FlutterSelectionHandle) -> bool {
        self.bridge.release_selection(handle.into())
    }

    fn cancellation(&self, id: u64) -> Result<Cancellation, FlutterBridgeError> {
        self.cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
            .cloned()
            .ok_or_else(|| invalid_request("unknown or released cancellation token"))
    }
}

fn invalid_request(message: impl Into<String>) -> FlutterBridgeError {
    FlutterBridgeError {
        kind: FlutterBridgeErrorKind::InvalidRequest,
        message: message.into(),
    }
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_registry_is_bounded_and_releases_tokens() {
        let bridge = FlutterBridge::new();
        let mut ids = Vec::new();
        for _ in 0..MAX_CANCELLATIONS {
            ids.push(bridge.create_cancellation().unwrap());
        }
        assert!(bridge.create_cancellation().is_err());
        assert!(bridge.cancel(ids[0]));
        assert!(bridge.release_cancellation(ids[0]));
        assert!(!bridge.cancel(ids[0]));
        assert!(bridge.create_cancellation().is_ok());
    }
}
