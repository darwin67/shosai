use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use shosai_core::bridge::{
    Bridge, BridgeError, BufferHandle, Cancellation, DocumentHandle, OpenRequest, RenderRequest,
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

#[derive(Debug, Clone)]
pub struct FlutterDocumentSummary {
    pub handle: FlutterDocumentHandle,
    pub format: FlutterBookFormat,
    pub title: Option<String>,
    pub logical_unit_count: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct FlutterRenderedBuffer {
    pub handle: FlutterBufferHandle,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
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
        Self {
            bridge: Bridge::new(),
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
