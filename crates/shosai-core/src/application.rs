//! Platform-neutral document admission and format capabilities.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::cbz::CbzDoc;
use crate::document::Document;
use crate::epub::pagination::content_node_text_len;
use crate::epub::{EpubDoc, EpubLimits};
use crate::library::BookFormat;
use crate::path_key::path_key;
use crate::pdf::PdfDoc;

const EPUB_RETAINED_SOURCE_COPIES: usize = 4;
const EPUB_PRESENTATION_UNIT_BYTES: usize = 256;
const EPUB_CONTAINER_OVERHEAD_BYTES: usize = 64 * 1024 * 1024;
const PDF_RETAINED_OVERHEAD_BYTES: usize = 16 * 1024 * 1024;

/// A locator supplied by the current device.
///
/// `local_id` is meaningful only to the platform adapter that issued it. It is
/// deliberately separate from library identity and must not be synchronized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFileLocator {
    local_id: String,
    path: PathBuf,
    format_hint: Option<BookFormat>,
}

impl DeviceFileLocator {
    pub fn new(local_id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            local_id: local_id.into(),
            path: path.into(),
            format_hint: None,
        }
    }

    pub fn with_format_hint(mut self, format: BookFormat) -> Self {
        self.format_hint = Some(format);
        self
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self::new(path_key(&path), path)
    }

    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn format_hint(&self) -> Option<BookFormat> {
        self.format_hint
    }

    pub fn format(&self) -> Result<BookFormat, OpenDocumentError> {
        let extension = self
            .path()
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        self.format_hint()
            .or_else(|| BookFormat::from_extension(&extension))
            .ok_or(OpenDocumentError::UnsupportedFormat(extension))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatCapabilities {
    pub paginated: bool,
    pub continuous: bool,
    pub reflowable: bool,
    pub searchable: bool,
    pub selectable: bool,
}

pub fn format_capabilities(format: BookFormat) -> FormatCapabilities {
    match format {
        BookFormat::Pdf => FormatCapabilities {
            paginated: true,
            continuous: true,
            reflowable: false,
            searchable: true,
            selectable: true,
        },
        BookFormat::Epub => FormatCapabilities {
            paginated: true,
            continuous: true,
            reflowable: true,
            searchable: true,
            selectable: true,
        },
        BookFormat::Cbz => FormatCapabilities {
            paginated: true,
            continuous: true,
            reflowable: false,
            searchable: false,
            selectable: false,
        },
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OpenDocumentError {
    #[error("unsupported file format: .{0}")]
    UnsupportedFormat(String),
    #[error("document was not found")]
    NotFound,
    #[error("document is inaccessible: {0}")]
    Inaccessible(String),
    #[error("{format} exceeds an opening resource limit: {detail}")]
    LimitExceeded { format: BookFormat, detail: String },
    #[error("{format} backend is unavailable: {detail}")]
    BackendUnavailable { format: BookFormat, detail: String },
    #[error("failed to open {format}: {detail}")]
    Open { format: BookFormat, detail: String },
}

#[derive(Debug, Error)]
#[error("{0}")]
pub(crate) struct ResourceLimitError(pub(crate) String);

#[derive(Debug, Clone)]
pub enum OpenDocument {
    Pdf(Arc<PdfDoc>),
    Epub(Arc<EpubDoc>),
    Cbz(Arc<CbzDoc>),
}

#[derive(Debug)]
pub struct OpenDocumentPlan {
    format: BookFormat,
    source_path: PathBuf,
    file: std::fs::File,
    encoded_byte_len: usize,
    retained_admission_byte_len: usize,
    title_hint: Option<String>,
}

pub(crate) struct AdmittedDocumentBytes {
    pub(crate) format: BookFormat,
    pub(crate) data: Vec<u8>,
    pub(crate) title_hint: Option<String>,
    admission: crate::document_admission::ProvisionalDocumentAdmission,
}

fn planned_retained_admission_byte_len<R: Read + Seek>(
    format: BookFormat,
    encoded_byte_len: usize,
    archive_reader: Option<R>,
    is_cancelled: Option<&dyn Fn() -> bool>,
) -> Result<usize, OpenDocumentError> {
    if format == BookFormat::Pdf {
        return OpenDocument::retained_admission_byte_len(format, encoded_byte_len).ok_or_else(
            || OpenDocumentError::LimitExceeded {
                format,
                detail: "retained-memory admission cannot be represented".to_owned(),
            },
        );
    }
    let reader = archive_reader.expect("ZIP admission requires its archive reader");
    let max_entries = match format {
        BookFormat::Epub => EpubLimits::default().max_archive_entries,
        BookFormat::Cbz => crate::cbz::CbzLimits::default().max_entries,
        BookFormat::Pdf => unreachable!(),
    };
    let archive_label = match format {
        BookFormat::Epub => "EPUB",
        BookFormat::Cbz => "CBZ",
        BookFormat::Pdf => unreachable!(),
    };
    let preflight = crate::zip_preflight::preflight(reader, max_entries, is_cancelled)
        .context("invalid ZIP archive")
        .with_context(|| format!("{archive_label} archive is corrupt"))
        .map_err(|error| classify_open_error(format, error))?;
    if format == BookFormat::Cbz {
        let metadata =
            crate::zip_preflight::metadata_allocation_ceiling(preflight).ok_or_else(|| {
                OpenDocumentError::LimitExceeded {
                    format,
                    detail: "archive metadata admission cannot be represented".to_owned(),
                }
            })?;
        return crate::document_admission::cbz_retained_ceiling(
            encoded_byte_len,
            max_entries,
            metadata,
            preflight.copied_filename_ceiling,
        )
        .ok_or_else(|| OpenDocumentError::LimitExceeded {
            format,
            detail: "retained-memory admission cannot be represented".to_owned(),
        });
    }
    let limits = EpubLimits::default();
    if preflight.declared_uncompressed_bytes > limits.max_total_uncompressed_bytes {
        return Err(OpenDocumentError::LimitExceeded {
            format,
            detail: "archive exceeds aggregate uncompressed byte limit".to_owned(),
        });
    }
    crate::document_admission::epub_retained_ceiling(
        encoded_byte_len,
        usize::try_from(preflight.declared_uncompressed_bytes).map_err(|_| {
            OpenDocumentError::LimitExceeded {
                format,
                detail: "archive uncompressed size cannot be represented".to_owned(),
            }
        })?,
        limits.max_total_decoded_font_bytes,
        limits.max_total_presentation_nodes,
        preflight.central_directory_bytes,
    )
    .ok_or_else(|| OpenDocumentError::LimitExceeded {
        format,
        detail: "retained-memory admission cannot be represented".to_owned(),
    })
}

#[cfg(test)]
fn probe_epub_central_directory<R: Read + Seek>(
    mut reader: R,
    max_entries: usize,
) -> Result<(u64, usize), OpenDocumentError> {
    let invalid = |detail: String| OpenDocumentError::Open {
        format: BookFormat::Epub,
        detail: format!("invalid EPUB archive: {detail}"),
    };
    let archive_byte_len = reader
        .seek(std::io::SeekFrom::End(0))
        .map_err(|error| invalid(error.to_string()))?;
    let tail_len = usize::try_from(archive_byte_len)
        .unwrap_or(usize::MAX)
        .min(65_557);
    let mut tail = vec![0_u8; tail_len];
    reader
        .seek(std::io::SeekFrom::Start(archive_byte_len - tail_len as u64))
        .and_then(|_| reader.read_exact(&mut tail))
        .map_err(|error| invalid(error.to_string()))?;
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|offset| {
            tail.get(*offset..*offset + 4) == Some(b"PK\x05\x06")
                && read_le_u16(&tail, *offset + 20)
                    .and_then(|length| offset.checked_add(22 + usize::from(length)))
                    == Some(tail.len())
        })
        .ok_or_else(|| invalid("end-of-central-directory record is missing".to_owned()))?;
    let tail_start = archive_byte_len - tail_len as u64;
    let eocd_position = tail_start + eocd as u64;
    if contains_plausible_eocd_before(&mut reader, eocd_position, archive_byte_len)
        .map_err(|error| invalid(error.to_string()))?
    {
        return Err(invalid(
            "multiple end-of-central-directory records are not supported".to_owned(),
        ));
    }
    let disk_number = read_le_u16(&tail, eocd + 4).unwrap();
    let central_directory_disk = read_le_u16(&tail, eocd + 6).unwrap();
    let mut entries_on_disk = u64::from(read_le_u16(&tail, eocd + 8).unwrap());
    let mut entries = u64::from(read_le_u16(&tail, eocd + 10).unwrap());
    if disk_number != 0 || central_directory_disk != 0 || entries_on_disk != entries {
        return Err(invalid("multi-disk archives are not supported".to_owned()));
    }
    let mut central_size = u64::from(read_le_u32(&tail, eocd + 12).unwrap());
    let mut central_offset = u64::from(read_le_u32(&tail, eocd + 16).unwrap());
    if entries_on_disk == u64::from(u16::MAX)
        || entries == u64::from(u16::MAX)
        || central_offset == u64::from(u32::MAX)
    {
        let locator = eocd
            .checked_sub(20)
            .ok_or_else(|| invalid("ZIP64 locator is missing".to_owned()))?;
        if tail.get(locator..locator + 4) != Some(b"PK\x06\x07") {
            return Err(invalid("ZIP64 locator is missing".to_owned()));
        }
        if read_le_u32(&tail, locator + 4) != Some(0) || read_le_u32(&tail, locator + 16) != Some(1)
        {
            return Err(invalid("multi-disk archives are not supported".to_owned()));
        }
        let zip64_offset = read_le_u64(&tail, locator + 8).unwrap();
        let mut record = [0_u8; 56];
        reader
            .seek(std::io::SeekFrom::Start(zip64_offset))
            .and_then(|_| reader.read_exact(&mut record))
            .map_err(|error| invalid(error.to_string()))?;
        if &record[..4] != b"PK\x06\x06" {
            return Err(invalid("ZIP64 footer is missing".to_owned()));
        }
        let record_size = read_le_u64(&record, 4).unwrap();
        if record_size != 44
            || record_size
                .checked_add(12)
                .and_then(|size| zip64_offset.checked_add(size))
                != Some(tail_start + locator as u64)
        {
            return Err(invalid(
                "ZIP64 extensible data sectors are not supported".to_owned(),
            ));
        }
        let zip64_disk_number = read_le_u32(&record, 16).unwrap();
        let zip64_central_directory_disk = read_le_u32(&record, 20).unwrap();
        entries_on_disk = read_le_u64(&record, 24).unwrap();
        entries = read_le_u64(&record, 32).unwrap();
        if zip64_disk_number != 0 || zip64_central_directory_disk != 0 || entries_on_disk != entries
        {
            return Err(invalid("multi-disk archives are not supported".to_owned()));
        }
        central_size = read_le_u64(&record, 40).unwrap();
        central_offset = read_le_u64(&record, 48).unwrap();
    }
    if entries_on_disk > max_entries as u64 || entries > max_entries as u64 {
        return Err(OpenDocumentError::LimitExceeded {
            format: BookFormat::Epub,
            detail: "archive contains too many entries".to_owned(),
        });
    }
    let central_end = central_offset
        .checked_add(central_size)
        .filter(|end| *end <= archive_byte_len)
        .ok_or_else(|| invalid("central directory is outside the archive".to_owned()))?;
    reader
        .seek(std::io::SeekFrom::Start(central_offset))
        .map_err(|error| invalid(error.to_string()))?;
    let mut declared_uncompressed = 0_u64;
    for _ in 0..entries {
        let mut header = [0_u8; 46];
        reader
            .read_exact(&mut header)
            .map_err(|error| invalid(error.to_string()))?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(invalid("central-directory entry is malformed".to_owned()));
        }
        let flags = read_le_u16(&header, 8).unwrap();
        let declared = read_le_u32(&header, 24).unwrap();
        let name_len = usize::from(read_le_u16(&header, 28).unwrap());
        let extra_len = usize::from(read_le_u16(&header, 30).unwrap());
        let comment_len = usize::from(read_le_u16(&header, 32).unwrap());
        let variable_len = name_len
            .checked_add(extra_len)
            .and_then(|length| length.checked_add(comment_len))
            .ok_or_else(|| invalid("entry metadata overflow".to_owned()))?;
        let mut variable = vec![0_u8; variable_len];
        reader
            .read_exact(&mut variable)
            .map_err(|error| invalid(error.to_string()))?;
        let name = &variable[..name_len];
        let comment = &variable[name_len + extra_len..];
        let expands_when_decoded = if flags & (1 << 11) != 0 {
            std::str::from_utf8(name).is_err() || std::str::from_utf8(comment).is_err()
        } else {
            !name.is_ascii() || !comment.is_ascii()
        };
        if expands_when_decoded {
            return Err(invalid(
                "entry names and comments must have a non-expanding encoding".to_owned(),
            ));
        }
        let uncompressed = if declared == u32::MAX {
            zip64_uncompressed_size(&variable[name_len..name_len + extra_len])
                .ok_or_else(|| invalid("ZIP64 entry size is missing".to_owned()))?
        } else {
            u64::from(declared)
        };
        declared_uncompressed = declared_uncompressed
            .checked_add(uncompressed)
            .ok_or_else(|| invalid("uncompressed size overflow".to_owned()))?;
    }
    let consumed = reader
        .stream_position()
        .map_err(|error| invalid(error.to_string()))?;
    if consumed != central_end {
        return Err(invalid(
            "central-directory size does not match its entries".to_owned(),
        ));
    }
    let central_directory_bytes =
        usize::try_from(central_size).map_err(|_| OpenDocumentError::LimitExceeded {
            format: BookFormat::Epub,
            detail: "central-directory size cannot be represented".to_owned(),
        })?;
    Ok((declared_uncompressed, central_directory_bytes))
}

#[cfg(test)]
fn contains_plausible_eocd_before<R: Read + Seek>(
    reader: &mut R,
    end: u64,
    archive_byte_len: u64,
) -> std::io::Result<bool> {
    reader.seek(std::io::SeekFrom::Start(0))?;
    let expected = u32::from_be_bytes(*b"PK\x05\x06");
    let mut window = 0_u32;
    let mut seen = 0_usize;
    let mut consumed = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while consumed < end {
        let remaining = usize::try_from(end - consumed).unwrap_or(usize::MAX);
        let read_len = remaining.min(buffer.len());
        let read = reader.read(&mut buffer[..read_len])?;
        if read == 0 {
            break;
        }
        for (index, byte) in buffer[..read].iter().enumerate() {
            window = (window << 8) | u32::from(*byte);
            seen += 1;
            let candidate_position = consumed + index as u64 + 1 - seen.min(4) as u64;
            if seen >= 4
                && window == expected
                && is_plausible_eocd(reader, candidate_position, archive_byte_len)?
            {
                return Ok(true);
            }
        }
        consumed += read as u64;
    }
    Ok(false)
}

#[cfg(test)]
fn is_plausible_eocd<R: Read + Seek>(
    reader: &mut R,
    position: u64,
    archive_byte_len: u64,
) -> std::io::Result<bool> {
    let resume = reader.stream_position()?;
    reader.seek(std::io::SeekFrom::Start(position))?;
    let mut header = [0_u8; 22];
    let read_result = reader.read_exact(&mut header);
    reader.seek(std::io::SeekFrom::Start(resume))?;
    if read_result.is_err() || &header[..4] != b"PK\x05\x06" {
        return Ok(false);
    }
    let comment_len = u64::from(read_le_u16(&header, 20).unwrap());
    if position
        .checked_add(22 + comment_len)
        .is_none_or(|end| end > archive_byte_len)
    {
        return Ok(false);
    }
    let disk = read_le_u16(&header, 4).unwrap();
    let central_disk = read_le_u16(&header, 6).unwrap();
    let count = read_le_u16(&header, 8).unwrap();
    let total_count = read_le_u16(&header, 10).unwrap();
    let central_offset = u64::from(read_le_u32(&header, 16).unwrap());
    if (total_count == u16::MAX || central_offset == u64::from(u32::MAX))
        && let Some(locator_position) = position.checked_sub(20)
    {
        let resume = reader.stream_position()?;
        reader.seek(std::io::SeekFrom::Start(locator_position))?;
        let mut locator = [0_u8; 20];
        let read_result = reader.read_exact(&mut locator);
        reader.seek(std::io::SeekFrom::Start(resume))?;
        if read_result.is_ok()
            && &locator[..4] == b"PK\x06\x07"
            && read_le_u32(&locator, 16).is_some_and(|disks| disks <= 1)
        {
            let zip64_offset = read_le_u64(&locator, 8).unwrap();
            return zip64_offset
                .checked_add(4)
                .filter(|end| *end <= locator_position)
                .map_or(Ok(false), |_| {
                    contains_signature_between(
                        reader,
                        zip64_offset,
                        locator_position,
                        *b"PK\x06\x06",
                    )
                });
        }
    }
    if disk != central_disk {
        return Ok(false);
    }
    if count == 0 {
        return Ok(false);
    }
    if total_count == 0 {
        return Ok(true);
    }
    if central_offset >= position {
        return Ok(false);
    }
    contains_signature_between(reader, central_offset, position, *b"PK\x01\x02")
}

#[cfg(test)]
fn contains_signature_between<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    signature: [u8; 4],
) -> std::io::Result<bool> {
    let resume = reader.stream_position()?;
    reader.seek(std::io::SeekFrom::Start(start))?;
    let expected = u32::from_be_bytes(signature);
    let mut window = 0_u32;
    let mut seen = 0_usize;
    let mut consumed = start;
    let mut buffer = [0_u8; 64 * 1024];
    let mut found = false;
    while consumed < end {
        let remaining = usize::try_from(end - consumed).unwrap_or(usize::MAX);
        let read_len = remaining.min(buffer.len());
        let read = reader.read(&mut buffer[..read_len])?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            window = (window << 8) | u32::from(*byte);
            seen += 1;
            if seen >= 4 && window == expected {
                found = true;
                break;
            }
        }
        if found {
            break;
        }
        consumed += read as u64;
    }
    reader.seek(std::io::SeekFrom::Start(resume))?;
    Ok(found)
}

#[cfg(test)]
fn zip64_uncompressed_size(extra: &[u8]) -> Option<u64> {
    let mut offset = 0_usize;
    while offset.checked_add(4)? <= extra.len() {
        let id = read_le_u16(extra, offset)?;
        let length = usize::from(read_le_u16(extra, offset + 2)?);
        let data = extra.get(offset + 4..offset.checked_add(4 + length)?)?;
        if id == 1 {
            return read_le_u64(data, 0);
        }
        offset = offset.checked_add(4 + length)?;
    }
    None
}

#[cfg(test)]
fn read_le_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

#[cfg(test)]
fn read_le_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

#[cfg(test)]
fn read_le_u64(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

impl OpenDocumentPlan {
    pub fn prepare(locator: &DeviceFileLocator) -> Result<Self, OpenDocumentError> {
        Self::prepare_inner(locator, None)
    }

    pub fn prepare_cancellable(
        locator: &DeviceFileLocator,
        cancellation: &crate::bridge::Cancellation,
    ) -> Result<Self, OpenDocumentError> {
        let is_cancelled = || cancellation.is_cancelled();
        Self::prepare_inner(locator, Some(&is_cancelled))
    }

    fn prepare_inner(
        locator: &DeviceFileLocator,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, OpenDocumentError> {
        if is_cancelled.is_some_and(|cancelled| cancelled()) {
            return Err(classify_open_error(
                locator.format()?,
                anyhow::anyhow!("document open cancelled"),
            ));
        }
        let format = locator.format()?;
        let mut file = std::fs::File::open(locator.path()).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => OpenDocumentError::NotFound,
            _ => OpenDocumentError::Inaccessible(error.to_string()),
        })?;
        let max_input_bytes = OpenDocument::max_input_bytes(format);
        let file_size = file
            .metadata()
            .map_err(|error| OpenDocumentError::Inaccessible(error.to_string()))?
            .len();
        if file_size > max_input_bytes {
            return Err(OpenDocumentError::LimitExceeded {
                format,
                detail: format!("input is larger than {max_input_bytes} bytes"),
            });
        }
        let encoded_byte_len =
            usize::try_from(file_size).map_err(|_| OpenDocumentError::LimitExceeded {
                format,
                detail: "input size cannot be represented".to_owned(),
            })?;
        let retained_admission_byte_len = planned_retained_admission_byte_len(
            format,
            encoded_byte_len,
            (format != BookFormat::Pdf).then_some(&file),
            is_cancelled,
        )?;
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|error| OpenDocumentError::Inaccessible(error.to_string()))?;
        Ok(Self {
            format,
            source_path: locator.path().to_path_buf(),
            file,
            encoded_byte_len,
            retained_admission_byte_len,
            title_hint: locator
                .path()
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned()),
        })
    }

    pub fn format(&self) -> BookFormat {
        self.format
    }

    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn retained_admission_byte_len(&self) -> Option<usize> {
        Some(self.retained_admission_byte_len)
    }

    pub(crate) fn read_bytes(self) -> Result<AdmittedDocumentBytes, OpenDocumentError> {
        self.read_bytes_cancellable(None)
    }

    pub(crate) fn read_bytes_cancellable(
        mut self,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<AdmittedDocumentBytes, OpenDocumentError> {
        let retained_bytes = self.retained_admission_byte_len;
        let admission =
            crate::document_admission::ProvisionalDocumentAdmission::acquire(retained_bytes)
                .map_err(|error| classify_open_error(self.format, error))?;
        let mut data = Vec::with_capacity(self.encoded_byte_len);
        let mut buffer = [0_u8; 64 * 1024];
        while data.len() < self.encoded_byte_len {
            if is_cancelled.is_some_and(|cancelled| cancelled()) {
                return Err(classify_open_error(
                    self.format,
                    anyhow::anyhow!("document open cancelled"),
                ));
            }
            let remaining = self.encoded_byte_len - data.len();
            let read_len = remaining.min(buffer.len());
            let read = self
                .file
                .read(&mut buffer[..read_len])
                .map_err(|error| classify_open_error(self.format, error.into()))?;
            if read == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..read]);
        }
        let mut extra = [0_u8; 1];
        let grew = self
            .file
            .read(&mut extra)
            .map_err(|error| classify_open_error(self.format, error.into()))?
            != 0;
        if data.len() != self.encoded_byte_len || grew {
            return Err(OpenDocumentError::LimitExceeded {
                format: self.format,
                detail: "document changed after admission".to_owned(),
            });
        }
        let required = if self.format != BookFormat::Pdf {
            planned_retained_admission_byte_len(
                self.format,
                data.capacity(),
                Some(std::io::Cursor::new(&data)),
                is_cancelled,
            )?
        } else {
            OpenDocument::retained_admission_byte_len(self.format, data.capacity()).ok_or_else(
                || OpenDocumentError::LimitExceeded {
                    format: self.format,
                    detail: "retained-memory admission cannot be represented".to_owned(),
                },
            )?
        };
        if required > retained_bytes {
            return Err(OpenDocumentError::LimitExceeded {
                format: self.format,
                detail: "document changed after admission".to_owned(),
            });
        }
        Ok(AdmittedDocumentBytes {
            format: self.format,
            data,
            title_hint: self.title_hint,
            admission,
        })
    }

    pub fn open(self) -> Result<OpenDocument, OpenDocumentError> {
        OpenDocument::from_admitted_bytes(self.read_bytes()?)
    }

    #[doc(hidden)]
    pub fn open_with_content_hash(self) -> Result<(OpenDocument, String), OpenDocumentError> {
        let admitted = self.read_bytes()?;
        let content_hash = format!("{:x}", Sha256::digest(&admitted.data));
        let document = OpenDocument::from_admitted_bytes(admitted)?;
        Ok((document, content_hash))
    }

    #[doc(hidden)]
    pub fn open_with_content_hash_cancellable(
        self,
        cancellation: crate::bridge::Cancellation,
    ) -> Result<(OpenDocument, String), OpenDocumentError> {
        let is_cancelled = || cancellation.is_cancelled();
        let admitted = self.read_bytes_cancellable(Some(&is_cancelled))?;
        if cancellation.is_cancelled() {
            return Err(classify_open_error(
                admitted.format,
                anyhow::anyhow!("document open cancelled"),
            ));
        }
        let mut hasher = Sha256::new();
        for chunk in admitted.data.chunks(64 * 1024) {
            if cancellation.is_cancelled() {
                return Err(classify_open_error(
                    admitted.format,
                    anyhow::anyhow!("document open cancelled"),
                ));
            }
            hasher.update(chunk);
        }
        let content_hash = format!("{:x}", hasher.finalize());
        let document = OpenDocument::from_admitted_bytes_cancellable(admitted, &is_cancelled)?;
        Ok((document, content_hash))
    }
}

impl OpenDocument {
    /// Conservative charge that must be admitted before parsing this format.
    #[doc(hidden)]
    pub fn maximum_retained_byte_len(format: BookFormat) -> Option<usize> {
        let encoded_byte_len = usize::try_from(Self::max_input_bytes(format)).ok()?;
        Self::retained_admission_byte_len(format, encoded_byte_len)
    }

    /// Conservative retained charge for a stable encoded input length.
    #[doc(hidden)]
    pub fn retained_admission_byte_len(
        format: BookFormat,
        encoded_byte_len: usize,
    ) -> Option<usize> {
        let expansion = match format {
            BookFormat::Epub => {
                let limits = EpubLimits::default();
                usize::try_from(limits.max_total_uncompressed_bytes)
                    .ok()?
                    .checked_mul(EPUB_RETAINED_SOURCE_COPIES)?
                    .checked_add(limits.max_total_decoded_font_bytes)?
                    .checked_add(
                        limits
                            .max_total_presentation_nodes
                            .checked_mul(EPUB_PRESENTATION_UNIT_BYTES)?,
                    )?
                    .checked_add(encoded_byte_len.checked_mul(16)?)?
                    .checked_add(EPUB_CONTAINER_OVERHEAD_BYTES)?
            }
            BookFormat::Pdf => PDF_RETAINED_OVERHEAD_BYTES,
            BookFormat::Cbz => {
                let limits = crate::cbz::CbzLimits::default();
                encoded_byte_len
                    .checked_mul(16)?
                    .checked_add(limits.max_entries.checked_mul(512)?)?
                    .checked_add(64 * 1024)?
                    .checked_add(limits.max_entries.checked_mul(
                        std::mem::size_of::<String>()
                            + std::mem::size_of::<usize>()
                            + std::mem::size_of::<Option<(u32, u32)>>()
                            + std::mem::size_of::<Option<usize>>(),
                    )?)?
                    .checked_add(4 * 1024)?
            }
        };
        encoded_byte_len.checked_add(expansion)
    }

    /// Conservative byte charge for memory retained by this parsed document.
    #[doc(hidden)]
    pub fn retained_byte_len(&self) -> Option<usize> {
        match self {
            Self::Pdf(document) => document.retained_byte_len(),
            Self::Epub(document) => document.retained_byte_len(),
            Self::Cbz(document) => document.retained_byte_len(),
        }
    }

    pub fn open(locator: &DeviceFileLocator) -> Result<Self, OpenDocumentError> {
        OpenDocumentPlan::prepare(locator)?.open()
    }

    pub(crate) fn max_input_bytes(format: BookFormat) -> u64 {
        format.max_input_bytes()
    }

    #[cfg(test)]
    pub(crate) fn from_bytes(
        format: BookFormat,
        data: Vec<u8>,
        title_hint: Option<String>,
    ) -> Result<Self, OpenDocumentError> {
        match format {
            BookFormat::Pdf => PdfDoc::from_bytes(data)
                .map(|document| Self::Pdf(Arc::new(document)))
                .map_err(|error| classify_open_error(format, error)),
            BookFormat::Epub => EpubDoc::from_bytes(data)
                .map(|document| Self::Epub(Arc::new(document)))
                .map_err(|error| classify_open_error(format, error)),
            BookFormat::Cbz => CbzDoc::from_bytes_with_title_hint(data, title_hint)
                .map(|document| Self::Cbz(Arc::new(document)))
                .map_err(|error| classify_open_error(format, error)),
        }
    }

    pub(crate) fn from_admitted_bytes(
        admitted: AdmittedDocumentBytes,
    ) -> Result<Self, OpenDocumentError> {
        Self::from_admitted_bytes_inner(admitted, None)
    }

    pub(crate) fn from_admitted_bytes_cancellable(
        admitted: AdmittedDocumentBytes,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, OpenDocumentError> {
        Self::from_admitted_bytes_inner(admitted, Some(is_cancelled))
    }

    fn from_admitted_bytes_inner(
        admitted: AdmittedDocumentBytes,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, OpenDocumentError> {
        let AdmittedDocumentBytes {
            format,
            data,
            title_hint,
            admission,
        } = admitted;
        match format {
            BookFormat::Pdf => {
                PdfDoc::from_bytes_admitted_cancellable(data, admission, is_cancelled)
                    .map(|document| Self::Pdf(Arc::new(document)))
                    .map_err(|error| classify_open_error(format, error))
            }
            BookFormat::Epub => {
                EpubDoc::from_bytes_admitted_cancellable(data, admission, is_cancelled)
                    .map(|document| Self::Epub(Arc::new(document)))
                    .map_err(|error| classify_open_error(format, error))
            }
            BookFormat::Cbz => CbzDoc::from_bytes_with_title_hint_admitted_cancellable(
                data,
                title_hint,
                admission,
                is_cancelled,
            )
            .map(|document| Self::Cbz(Arc::new(document)))
            .map_err(|error| classify_open_error(format, error)),
        }
    }

    pub fn format(&self) -> BookFormat {
        match self {
            Self::Pdf(_) => BookFormat::Pdf,
            Self::Epub(_) => BookFormat::Epub,
            Self::Cbz(_) => BookFormat::Cbz,
        }
    }

    pub fn capabilities(&self) -> FormatCapabilities {
        format_capabilities(self.format())
    }

    pub fn page_count(&self) -> usize {
        match self {
            Self::Pdf(document) => document.page_count(),
            Self::Epub(document) => document.chapter_count(),
            Self::Cbz(document) => document.page_count(),
        }
    }

    pub fn title(&self) -> Option<String> {
        match self {
            Self::Pdf(document) => document.title(),
            Self::Epub(document) => document.metadata().title,
            Self::Cbz(document) => document.metadata().title,
        }
    }

    pub fn max_location_offset(&self, page: usize) -> Option<usize> {
        let Self::Epub(document) = self else {
            return None;
        };
        document.presentation().chapter(page).map(|chapter| {
            chapter.nodes().iter().fold(0usize, |offset, node| {
                offset.saturating_add(content_node_text_len(node).saturating_add(1))
            })
        })
    }
}

fn classify_open_error(format: BookFormat, error: anyhow::Error) -> OpenDocumentError {
    let detail = format!("{error:#}");
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<ResourceLimitError>().is_some())
    {
        return OpenDocumentError::LimitExceeded { format, detail };
    }
    if format == BookFormat::Pdf && crate::pdf::is_backend_unavailable(&error) {
        return OpenDocumentError::BackendUnavailable { format, detail };
    }
    if let Some(io_error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
    {
        return match io_error.kind() {
            std::io::ErrorKind::NotFound => OpenDocumentError::NotFound,
            std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::ReadOnlyFilesystem
            | std::io::ErrorKind::ResourceBusy => OpenDocumentError::Inaccessible(detail),
            _ => OpenDocumentError::Open { format, detail },
        };
    }
    OpenDocumentError::Open { format, detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbz::CbzLimits;
    use crate::pdf::MAX_PDF_INPUT_BYTES;
    use std::io::Write;

    #[test]
    fn locator_identity_is_device_local_and_does_not_rewrite_the_path() {
        let locator = DeviceFileLocator::new("android:42", "content/books/example.epub");

        assert_eq!(locator.local_id(), "android:42");
        assert_eq!(locator.path(), Path::new("content/books/example.epub"));
    }

    #[test]
    fn format_capabilities_are_centralized() {
        assert!(format_capabilities(BookFormat::Epub).reflowable);
        assert!(format_capabilities(BookFormat::Pdf).selectable);
        assert!(!format_capabilities(BookFormat::Cbz).searchable);
    }

    #[test]
    fn unsupported_extensions_fail_before_document_io() {
        let error = OpenDocument::open(&DeviceFileLocator::from_path("missing.txt")).unwrap_err();

        assert_eq!(
            error,
            OpenDocumentError::UnsupportedFormat("txt".to_owned())
        );
    }

    #[test]
    fn prepared_document_open_honors_preexisting_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("book.cbz");
        std::fs::write(&path, include_bytes!("../tests/fixtures/sample.cbz")).unwrap();
        let plan = OpenDocumentPlan::prepare(&DeviceFileLocator::from_path(path)).unwrap();
        let cancellation = crate::bridge::Cancellation::new();
        cancellation.cancel();

        let error = plan
            .open_with_content_hash_cancellable(cancellation)
            .unwrap_err();

        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn small_epub_plans_fit_three_documents_within_the_production_budget() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.epub");
        let charge = OpenDocumentPlan::prepare(&DeviceFileLocator::from_path(path))
            .unwrap()
            .retained_admission_byte_len()
            .unwrap();

        assert!(charge.checked_mul(3).unwrap() <= 3 * 1024 * 1024 * 1024);
        assert!(charge < OpenDocument::maximum_retained_byte_len(BookFormat::Epub).unwrap());
    }

    #[test]
    fn epub_plan_rejects_zip64_entry_count_before_reading_entries() {
        let entries = EpubLimits::default().max_archive_entries as u64 + 1;
        let mut archive = vec![0_u8; 56 + 20 + 22];
        archive[..4].copy_from_slice(b"PK\x06\x06");
        archive[4..12].copy_from_slice(&44_u64.to_le_bytes());
        archive[24..32].copy_from_slice(&entries.to_le_bytes());
        archive[32..40].copy_from_slice(&entries.to_le_bytes());
        archive[56..60].copy_from_slice(b"PK\x06\x07");
        archive[64..72].copy_from_slice(&0_u64.to_le_bytes());
        archive[72..76].copy_from_slice(&1_u32.to_le_bytes());
        archive[76..80].copy_from_slice(b"PK\x05\x06");
        archive[84..86].copy_from_slice(&u16::MAX.to_le_bytes());
        archive[86..88].copy_from_slice(&u16::MAX.to_le_bytes());
        archive[88..92].copy_from_slice(&u32::MAX.to_le_bytes());
        archive[92..96].copy_from_slice(&u32::MAX.to_le_bytes());

        let error = probe_epub_central_directory(
            std::io::Cursor::new(&archive),
            EpubLimits::default().max_archive_entries,
        )
        .unwrap_err();

        assert!(matches!(error, OpenDocumentError::LimitExceeded { .. }));
    }

    #[test]
    fn fallback_probe_matches_zip32_zero_total_count_behavior() {
        let position = 100_usize;
        let mut archive = vec![0_u8; position + 22];
        archive[position..position + 4].copy_from_slice(b"PK\x05\x06");
        archive[position + 8..position + 10].copy_from_slice(&10_001_u16.to_le_bytes());

        assert!(
            is_plausible_eocd(
                &mut std::io::Cursor::new(&archive),
                position as u64,
                archive.len() as u64,
            )
            .unwrap()
        );
    }

    #[test]
    fn fallback_probe_uses_zip32_when_zip64_locator_is_missing() {
        let position = 100_usize;
        let mut archive = vec![0_u8; position + 22];
        archive[..4].copy_from_slice(b"PK\x01\x02");
        archive[position..position + 4].copy_from_slice(b"PK\x05\x06");
        archive[position + 8..position + 10].copy_from_slice(&10_001_u16.to_le_bytes());
        archive[position + 10..position + 12].copy_from_slice(&u16::MAX.to_le_bytes());

        assert!(
            is_plausible_eocd(
                &mut std::io::Cursor::new(&archive),
                position as u64,
                archive.len() as u64,
            )
            .unwrap()
        );
    }

    #[test]
    fn epub_plan_charges_large_central_directory_fields() {
        let name_len = 1_024_usize;
        let extra_len = 2_048_usize;
        let comment_len = 4_096_usize;
        let central_len = 46 + name_len + extra_len + comment_len;
        let mut archive = vec![0_u8; central_len + 22];
        archive[..4].copy_from_slice(b"PK\x01\x02");
        archive[24..28].copy_from_slice(&7_u32.to_le_bytes());
        archive[28..30].copy_from_slice(&(name_len as u16).to_le_bytes());
        archive[30..32].copy_from_slice(&(extra_len as u16).to_le_bytes());
        archive[32..34].copy_from_slice(&(comment_len as u16).to_le_bytes());
        archive[central_len..central_len + 4].copy_from_slice(b"PK\x05\x06");
        archive[central_len + 8..central_len + 10].copy_from_slice(&1_u16.to_le_bytes());
        archive[central_len + 10..central_len + 12].copy_from_slice(&1_u16.to_le_bytes());
        archive[central_len + 12..central_len + 16]
            .copy_from_slice(&(central_len as u32).to_le_bytes());

        let charge = planned_retained_admission_byte_len(
            BookFormat::Epub,
            archive.len(),
            Some(std::io::Cursor::new(&archive)),
            None,
        )
        .unwrap();
        let limits = EpubLimits::default();
        let expected = crate::document_admission::epub_retained_ceiling(
            archive.len(),
            7,
            limits.max_total_decoded_font_bytes,
            limits.max_total_presentation_nodes,
            central_len,
        )
        .unwrap();
        let without_central_metadata = crate::document_admission::epub_retained_ceiling(
            archive.len(),
            7,
            limits.max_total_decoded_font_bytes,
            limits.max_total_presentation_nodes,
            0,
        )
        .unwrap();

        assert_eq!(charge, expected);
        assert_eq!(charge - without_central_metadata, central_len * 16);
    }

    #[test]
    fn prepared_plans_reject_encoded_growth_for_every_format() {
        let directory = tempfile::tempdir().unwrap();
        for (name, fixture) in [
            (
                "book.pdf",
                include_bytes!("../tests/fixtures/sample.pdf").as_slice(),
            ),
            (
                "book.epub",
                include_bytes!("../tests/fixtures/sample.epub").as_slice(),
            ),
            (
                "book.cbz",
                include_bytes!("../tests/fixtures/sample.cbz").as_slice(),
            ),
        ] {
            let path = directory.path().join(name);
            std::fs::write(&path, fixture).unwrap();
            let plan = OpenDocumentPlan::prepare(&DeviceFileLocator::from_path(&path)).unwrap();
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(&[0])
                .unwrap();

            let error = plan.open().unwrap_err();

            assert!(
                matches!(error, OpenDocumentError::LimitExceeded { .. }),
                "{name} growth should exceed its prepared admission: {error}"
            );
        }
    }

    #[test]
    fn platform_format_hint_is_checked_before_document_io() {
        let locator = DeviceFileLocator::new("content:42", "missing-provider-file")
            .with_format_hint(BookFormat::Epub);
        let error = OpenDocument::open(&locator).unwrap_err();

        assert_eq!(error, OpenDocumentError::NotFound);
    }

    #[test]
    fn oversized_inputs_have_a_structural_limit_category() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.pdf");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_PDF_INPUT_BYTES + 1)
            .unwrap();

        let error = OpenDocument::open(&DeviceFileLocator::from_path(path)).unwrap_err();

        assert!(matches!(
            error,
            OpenDocumentError::LimitExceeded {
                format: BookFormat::Pdf,
                ..
            }
        ));
    }

    #[test]
    fn parser_limits_have_a_structural_limit_category() {
        let limits = CbzLimits {
            max_entries: 0,
            ..CbzLimits::default()
        };
        let error = CbzDoc::from_bytes_with_limits(
            include_bytes!("../tests/fixtures/sample.cbz").to_vec(),
            limits,
        )
        .unwrap_err();

        assert!(matches!(
            classify_open_error(BookFormat::Cbz, error),
            OpenDocumentError::LimitExceeded {
                format: BookFormat::Cbz,
                ..
            }
        ));
    }

    #[test]
    fn malformed_io_is_not_classified_as_inaccessible() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed stream",
        ));

        assert!(matches!(
            classify_open_error(BookFormat::Epub, error),
            OpenDocumentError::Open {
                format: BookFormat::Epub,
                ..
            }
        ));
    }

    #[test]
    fn malformed_cbz_bytes_have_a_structural_open_category() {
        let error =
            OpenDocument::from_bytes(BookFormat::Cbz, b"not a zip".to_vec(), None).unwrap_err();

        assert!(matches!(
            error,
            OpenDocumentError::Open {
                format: BookFormat::Cbz,
                ..
            }
        ));
    }

    #[test]
    fn malformed_pdf_bytes_have_a_structural_open_category() {
        let error =
            OpenDocument::from_bytes(BookFormat::Pdf, b"not a pdf".to_vec(), None).unwrap_err();

        assert!(matches!(
            error,
            OpenDocumentError::Open {
                format: BookFormat::Pdf,
                ..
            }
        ));
    }

    #[test]
    fn pdf_backend_failures_have_a_structural_category() {
        let error = anyhow::Error::new(crate::pdf::PdfBackendUnavailable(
            "missing PDFium".to_owned(),
        ));

        assert!(matches!(
            classify_open_error(BookFormat::Pdf, error),
            OpenDocumentError::BackendUnavailable {
                format: BookFormat::Pdf,
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn path_locators_keep_non_unicode_paths_distinct() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let first = DeviceFileLocator::from_path(Path::new(OsStr::from_bytes(b"book-\x80.epub")));
        let second = DeviceFileLocator::from_path(Path::new(OsStr::from_bytes(b"book-\x81.epub")));

        assert_ne!(first.local_id(), second.local_id());
        assert_eq!(first.path().as_os_str().as_bytes(), b"book-\x80.epub");
    }
}
