//! Renderer-independent text annotations and their SQLite persistence.

use std::ops::Range;
use std::str::FromStr;
#[cfg(test)]
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnection, SqlitePool, SqliteRow};
use thiserror::Error;
#[cfg(test)]
use tokio::sync::Semaphore;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use crate::epub::CanonicalEpubPath;

pub const ANCHOR_VERSION: u32 = 1;
pub const QUOTE_PROFILE_V1: &str = "shosai-quote-v1";
pub const MAX_QUOTE_SCALARS: usize = 65_536;
pub const MAX_QUOTE_CONTEXT_INPUT_SCALARS: usize = 65_536;
pub const MAX_CONTEXT_SCALARS: usize = 32;
pub const MAX_PDF_RECTANGLES: usize = 16_384;
pub const MAX_ANNOTATIONS_PER_SNAPSHOT: usize = 1_024;
pub const MAX_PDF_RECTANGLES_PER_SNAPSHOT: usize = 65_536;
pub const MAX_ANNOTATION_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub const ANNOTATION_SNAPSHOT_BASE_BYTES: usize = 256;
pub const MAX_ANNOTATION_BODY_SCALARS: usize = 65_536;
pub const MAX_FINGERPRINT_BYTES: usize = 1_024;
pub const MAX_FINGERPRINT_ALGORITHM_BYTES: usize = 64;
pub const MAX_LOCAL_PATH_BYTES: usize = 32_768;
pub const MAX_EPUB_RESOURCE_PATH_BYTES: usize = 4_096;
pub const MAX_PROVENANCE_SYSTEM_BYTES: usize = 256;
pub const MAX_PROVENANCE_ID_BYTES: usize = 4_096;
pub(crate) const MAX_TEXT_ANCHOR_RESOLUTION_WORK: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
#[error("annotation snapshot exceeds its aggregate retention limit")]
pub struct AnnotationSnapshotLimit;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnnotationId(Uuid);

impl AnnotationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AnnotationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AnnotationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for AnnotationId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightColor {
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
}

impl HighlightColor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Pink => "pink",
            Self::Purple => "purple",
        }
    }

    fn from_db(value: &str) -> Result<Self> {
        match value {
            "yellow" => Ok(Self::Yellow),
            "green" => Ok(Self::Green),
            "blue" => Ok(Self::Blue),
            "pink" => Ok(Self::Pink),
            "purple" => Ok(Self::Purple),
            _ => bail!("unknown annotation color {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentFingerprint {
    pub algorithm: String,
    pub version: u32,
    pub bytes: Vec<u8>,
}

impl DocumentFingerprint {
    pub fn new(algorithm: impl Into<String>, version: u32, bytes: Vec<u8>) -> Result<Self> {
        let fingerprint = Self {
            algorithm: algorithm.into(),
            version,
            bytes,
        };
        fingerprint.validate()?;
        Ok(fingerprint)
    }

    fn validate(&self) -> Result<()> {
        if self.algorithm.trim().is_empty()
            || self.algorithm.len() > MAX_FINGERPRINT_ALGORITHM_BYTES
            || self.version == 0
            || self.bytes.is_empty()
            || self.bytes.len() > MAX_FINGERPRINT_BYTES
        {
            bail!("annotation fingerprint requires an algorithm, version, and bytes");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteSelector {
    pub original: Option<String>,
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

impl QuoteSelector {
    /// Build a selector from the selected text and its bounded surrounding text.
    pub fn new(selected: &str, before: &str, after: &str) -> Result<Self> {
        ensure_scalar_limit(selected, MAX_QUOTE_SCALARS, "annotation quote")?;
        ensure_scalar_limit(
            before,
            MAX_QUOTE_CONTEXT_INPUT_SCALARS,
            "annotation prefix input",
        )?;
        ensure_scalar_limit(
            after,
            MAX_QUOTE_CONTEXT_INPUT_SCALARS,
            "annotation suffix input",
        )?;
        let exact = normalize_quote_v1(selected);
        if exact.is_empty() {
            bail!("annotation quote must not be empty");
        }
        if exact.chars().count() > MAX_QUOTE_SCALARS {
            bail!("annotation quote exceeds {MAX_QUOTE_SCALARS} Unicode scalars");
        }
        Ok(Self {
            original: Some(selected.to_owned()),
            exact,
            prefix: quote_context_v1(before, ContextDirection::Prefix),
            suffix: quote_context_v1(after, ContextDirection::Suffix),
        })
    }

    fn validate(&self) -> Result<()> {
        if self.exact.is_empty()
            || self.exact.chars().count() > MAX_QUOTE_SCALARS
            || self.prefix.chars().count() > MAX_CONTEXT_SCALARS
            || self.suffix.chars().count() > MAX_CONTEXT_SCALARS
            || normalize_quote_v1(&self.exact) != self.exact
            || normalize_quote_v1(&self.prefix) != self.prefix
            || normalize_quote_v1(&self.suffix) != self.suffix
        {
            bail!("annotation quote selector is not normalized or exceeds its scalar limit");
        }
        if self.original.as_ref().is_some_and(|quote| {
            quote.chars().count() > MAX_QUOTE_SCALARS || normalize_quote_v1(quote) != self.exact
        }) {
            bail!("annotation original and normalized quote do not match");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl PageRect {
    pub fn new(left: f32, bottom: f32, right: f32, top: f32) -> Result<Self> {
        let rectangle = Self {
            left,
            bottom,
            right,
            top,
        };
        rectangle.validate()?;
        Ok(rectangle)
    }

    fn validate(&self) -> Result<()> {
        if ![self.left, self.bottom, self.right, self.top]
            .into_iter()
            .all(f32::is_finite)
            || self.left >= self.right
            || self.bottom >= self.top
        {
            bail!("PDF annotation rectangle must be finite and non-empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubAnchor {
    pub spine_occurrence: u32,
    pub resource_path: CanonicalEpubPath,
    pub scalar_start: u32,
    pub scalar_end: u32,
}

impl EpubAnchor {
    pub fn new(
        spine_occurrence: u32,
        resource_path: impl AsRef<str>,
        scalar_start: u32,
        scalar_end: u32,
    ) -> Result<Self> {
        if resource_path.as_ref().len() > MAX_EPUB_RESOURCE_PATH_BYTES {
            bail!("EPUB annotation resource path exceeds {MAX_EPUB_RESOURCE_PATH_BYTES} bytes");
        }
        let anchor = Self {
            spine_occurrence,
            resource_path: CanonicalEpubPath::new(resource_path.as_ref())
                .context("EPUB annotation requires a canonical resource path")?,
            scalar_start,
            scalar_end,
        };
        anchor.validate()?;
        Ok(anchor)
    }

    fn validate(&self) -> Result<()> {
        if self.resource_path.as_str().len() > MAX_EPUB_RESOURCE_PATH_BYTES {
            bail!("EPUB annotation resource path exceeds {MAX_EPUB_RESOURCE_PATH_BYTES} bytes");
        }
        if self.scalar_start >= self.scalar_end {
            bail!("EPUB annotation range must be non-empty and half-open");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdfAnchor {
    pub page: u32,
    pub character_range: Option<(u32, u32)>,
    pub rectangles: Vec<PageRect>,
}

impl PdfAnchor {
    pub fn new(
        page: u32,
        character_range: Option<(u32, u32)>,
        rectangles: Vec<PageRect>,
    ) -> Result<Self> {
        let anchor = Self {
            page,
            character_range,
            rectangles,
        };
        anchor.validate()?;
        Ok(anchor)
    }

    fn validate(&self) -> Result<()> {
        if self.rectangles.is_empty() || self.rectangles.len() > MAX_PDF_RECTANGLES {
            bail!("PDF annotation requires 1..={MAX_PDF_RECTANGLES} rectangles");
        }
        if self
            .character_range
            .is_some_and(|(start, end)| start >= end)
        {
            bail!("PDF annotation character range must be non-empty and half-open");
        }
        for rectangle in &self.rectangles {
            rectangle.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationTarget {
    Epub(EpubAnchor),
    Pdf(PdfAnchor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationResolution {
    Exact,
    Recovered,
    Ambiguous,
    Orphaned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTextAnchor {
    pub resolution: AnnotationResolution,
    pub range: Option<Range<usize>>,
}

#[derive(Debug, Error)]
pub(crate) enum TextAnchorResolutionError {
    #[error("text anchor resolution was cancelled")]
    Cancelled,
    #[error("text anchor resolution exceeded its work limit")]
    WorkLimit,
}

pub(crate) struct TextAnchorResolver<'a> {
    text: &'a str,
    scalar_bytes: Vec<usize>,
    normalized: MappedNormalizedText,
}

impl<'a> TextAnchorResolver<'a> {
    pub(crate) fn new(
        text: &'a str,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, TextAnchorResolutionError> {
        let scalar_bytes = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let normalized = mapped_normalized_quote_text(text, is_cancelled)?;
        Ok(Self {
            text,
            scalar_bytes,
            normalized,
        })
    }

    pub(crate) fn resolve(
        &self,
        stored_range: Range<usize>,
        quote: &QuoteSelector,
        remaining_work: &mut usize,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<ResolvedTextAnchor, TextAnchorResolutionError> {
        if stored_range.start < stored_range.end
            && stored_range.end < self.scalar_bytes.len()
            && normalize_quote_v1(
                &self.text
                    [self.scalar_bytes[stored_range.start]..self.scalar_bytes[stored_range.end]],
            ) == quote.exact
        {
            return Ok(ResolvedTextAnchor {
                resolution: AnnotationResolution::Exact,
                range: Some(stored_range),
            });
        }

        let mut candidate_count = 0;
        let mut only_candidate = None;
        let mut last_candidate = None;
        let mut contextual_candidate = None;
        let mut contextual_count = 0;
        for (index, start_byte) in self
            .normalized
            .byte_offsets
            .iter()
            .take(self.normalized.source_ranges.len())
            .enumerate()
        {
            if index % 1024 == 0 && is_cancelled() {
                return Err(TextAnchorResolutionError::Cancelled);
            }
            *remaining_work = remaining_work
                .checked_sub(1)
                .ok_or(TextAnchorResolutionError::WorkLimit)?;
            let Some(value) = self.normalized.text.get(*start_byte..) else {
                continue;
            };
            if !value.starts_with(&quote.exact) {
                continue;
            }
            let Some(end_byte) = start_byte.checked_add(quote.exact.len()) else {
                continue;
            };
            let Ok(end) = self.normalized.byte_offsets.binary_search(&end_byte) else {
                continue;
            };
            if index >= end {
                continue;
            }
            let source_range = self.normalized.source_ranges[index].start
                ..self.normalized.source_ranges[end - 1].end;
            if normalize_quote_v1(
                &self.text
                    [self.scalar_bytes[source_range.start]..self.scalar_bytes[source_range.end]],
            ) == quote.exact
            {
                if last_candidate.as_ref() == Some(&source_range) {
                    continue;
                }
                last_candidate = Some(source_range.clone());
                candidate_count += 1;
                if candidate_count == 1 {
                    only_candidate = Some(source_range.clone());
                }
                let matches_context = (quote.prefix.is_empty()
                    || self.normalized.text[..*start_byte]
                        .trim_end_matches(' ')
                        .ends_with(&quote.prefix))
                    && (quote.suffix.is_empty()
                        || self.normalized.text[end_byte..]
                            .trim_start_matches(' ')
                            .starts_with(&quote.suffix));
                if matches_context {
                    contextual_count += 1;
                    if contextual_count == 1 {
                        contextual_candidate = Some(source_range);
                    } else {
                        return Ok(ResolvedTextAnchor {
                            resolution: AnnotationResolution::Ambiguous,
                            range: None,
                        });
                    }
                }
            }
        }
        if candidate_count == 0 {
            return Ok(ResolvedTextAnchor {
                resolution: AnnotationResolution::Orphaned,
                range: None,
            });
        }

        let recovered = match contextual_count {
            1 => contextual_candidate,
            0 if candidate_count == 1 => only_candidate,
            _ => None,
        };
        Ok(ResolvedTextAnchor {
            resolution: if recovered.is_some() {
                AnnotationResolution::Recovered
            } else {
                AnnotationResolution::Ambiguous
            },
            range: recovered,
        })
    }
}

/// Resolve a persisted quote against the document's current Unicode-scalar
/// text. Stored offsets win when their normalized quote still matches; bounded
/// quote/context matching recovers a unique moved range without guessing when
/// repeated text remains ambiguous.
pub fn resolve_text_anchor(
    text: &str,
    stored_range: Range<usize>,
    quote: &QuoteSelector,
) -> ResolvedTextAnchor {
    let mut remaining_work = usize::MAX;
    TextAnchorResolver::new(text, &|| false)
        .and_then(|resolver| resolver.resolve(stored_range, quote, &mut remaining_work, &|| false))
        .expect("unlimited, uncancelled text resolution cannot fail")
}

struct MappedNormalizedText {
    text: String,
    byte_offsets: Vec<usize>,
    source_ranges: Vec<Range<usize>>,
}

fn mapped_normalized_quote_text(
    value: &str,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<MappedNormalizedText, TextAnchorResolutionError> {
    let mut source = value.chars().enumerate().peekable();
    let mut profile_input = String::new();
    let mut input_ranges = Vec::new();
    while let Some((index, character)) = source.next() {
        if index % 1024 == 0 && is_cancelled() {
            return Err(TextAnchorResolutionError::Cancelled);
        }
        if character == '\u{00ad}' {
            continue;
        }
        if character == '\r' {
            let end = if source.peek().is_some_and(|(_, next)| *next == '\n') {
                source.next();
                index + 2
            } else {
                index + 1
            };
            profile_input.push('\n');
            input_ranges.push(index..end);
        } else {
            profile_input.push(character);
            input_ranges.push(index..index + 1);
        }
    }
    let input_byte_offsets = profile_input
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(profile_input.len()))
        .collect::<Vec<_>>();
    let mut text = String::new();
    let mut source_ranges = Vec::new();
    let mut pending_space: Option<Range<usize>> = None;
    for (grapheme_index, (start_byte, grapheme)) in profile_input.grapheme_indices(true).enumerate()
    {
        if grapheme_index % 1024 == 0 && is_cancelled() {
            return Err(TextAnchorResolutionError::Cancelled);
        }
        let start = input_byte_offsets.binary_search(&start_byte).unwrap();
        let end = start + grapheme.chars().count();
        let source_range = input_ranges[start].start..input_ranges[end - 1].end;
        let normalized = grapheme.nfc().collect::<String>();
        for character in normalized.chars() {
            if quote_v1_whitespace(character) {
                if !text.is_empty() {
                    pending_space = Some(match pending_space {
                        Some(pending) => pending.start..source_range.end,
                        None => source_range.clone(),
                    });
                }
            } else {
                if let Some(pending) = pending_space.take() {
                    text.push(' ');
                    source_ranges.push(pending);
                }
                text.push(character);
                source_ranges.push(source_range.clone());
            }
        }
    }
    let byte_offsets = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .collect();
    Ok(MappedNormalizedText {
        text,
        byte_offsets,
        source_ranges,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProvenance {
    pub source_system: String,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewAnnotation {
    pub id: AnnotationId,
    pub book_id: Option<i64>,
    pub local_path: Option<String>,
    pub fingerprint: DocumentFingerprint,
    pub quote: Option<QuoteSelector>,
    pub target: AnnotationTarget,
    pub color: HighlightColor,
    pub body: Option<String>,
    pub provenance: Option<ImportProvenance>,
}

impl NewAnnotation {
    fn validate(&self) -> Result<()> {
        self.fingerprint.validate()?;
        if self
            .local_path
            .as_ref()
            .is_some_and(|path| path.is_empty() || path.len() > MAX_LOCAL_PATH_BYTES)
        {
            bail!("annotation local path is empty or exceeds {MAX_LOCAL_PATH_BYTES} bytes");
        }
        if let Some(body) = &self.body {
            ensure_scalar_limit(body, MAX_ANNOTATION_BODY_SCALARS, "annotation body")?;
        }
        if let Some(quote) = &self.quote {
            quote.validate()?;
        }
        match &self.target {
            AnnotationTarget::Epub(anchor) => {
                anchor.validate()?;
                if self.quote.is_none() {
                    bail!("EPUB annotations require a quote selector");
                }
            }
            AnnotationTarget::Pdf(anchor) => {
                anchor.validate()?;
                if anchor.character_range.is_some() != self.quote.is_some() {
                    bail!(
                        "PDF text ranges require quote selectors; geometry-only anchors require none"
                    );
                }
            }
        }
        if let Some(provenance) = &self.provenance
            && (provenance.source_system.trim().is_empty()
                || provenance.source_system.len() > MAX_PROVENANCE_SYSTEM_BYTES
                || provenance
                    .source_id
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || id.len() > MAX_PROVENANCE_ID_BYTES))
        {
            bail!("annotation provenance is empty or exceeds its byte limit");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub id: AnnotationId,
    pub book_id: Option<i64>,
    pub local_path: Option<String>,
    pub fingerprint: DocumentFingerprint,
    pub quote: Option<QuoteSelector>,
    pub target: AnnotationTarget,
    pub color: HighlightColor,
    pub body: Option<String>,
    pub provenance: Option<ImportProvenance>,
    pub created_at: String,
    pub modified_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AnnotationStore {
    pool: SqlitePool,
    #[cfg(test)]
    persistence_gate: Option<Arc<AnnotationPersistenceTestGate>>,
    #[cfg(test)]
    list_gate: Option<Arc<AnnotationPersistenceTestGate>>,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct AnnotationPersistenceTestGate {
    entered: Semaphore,
    release: Semaphore,
}

#[cfg(test)]
impl AnnotationPersistenceTestGate {
    pub(crate) fn new() -> Self {
        Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }

    pub(crate) async fn wait_until_entered(&self) {
        self.entered.acquire().await.unwrap().forget();
    }

    pub(crate) fn release(&self) {
        self.release.add_permits(1);
    }
}

impl AnnotationStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            #[cfg(test)]
            persistence_gate: None,
            #[cfg(test)]
            list_gate: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_test_gates(
        pool: SqlitePool,
        persistence_gate: Option<Arc<AnnotationPersistenceTestGate>>,
        list_gate: Option<Arc<AnnotationPersistenceTestGate>>,
    ) -> Self {
        Self {
            pool,
            persistence_gate,
            list_gate,
        }
    }

    #[cfg(test)]
    pub(crate) async fn execute_test_sql(&self, sql: &str) -> Result<()> {
        sqlx::query(sql).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn create_async(&self, annotation: &NewAnnotation) -> Result<Annotation> {
        annotation.validate()?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("failed to begin annotation insert")?;
        #[cfg(test)]
        if let Some(gate) = &self.persistence_gate {
            gate.entered.add_permits(1);
            gate.release.acquire().await.unwrap().forget();
        }
        let (format, epub, pdf) = match &annotation.target {
            AnnotationTarget::Epub(anchor) => ("epub", Some(anchor), None),
            AnnotationTarget::Pdf(anchor) => ("pdf", None, Some(anchor)),
        };
        let (char_start, char_end) = pdf
            .and_then(|anchor| anchor.character_range)
            .map_or((None, None), |(start, end)| {
                (Some(i64::from(start)), Some(i64::from(end)))
            });
        let quote = annotation.quote.as_ref();
        let provenance = annotation.provenance.as_ref();
        sqlx::query(
            "INSERT INTO annotations (
                id, book_id, local_path, format, anchor_version,
                fingerprint_algorithm, fingerprint_version, fingerprint,
                original_quote, normalization_profile, normalized_exact,
                normalized_prefix, normalized_suffix, color, body,
                source_system, source_id, epub_spine_occurrence,
                epub_resource_path, epub_scalar_start, epub_scalar_end,
                pdf_page, pdf_char_start, pdf_char_end)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(annotation.id.to_string())
        .bind(annotation.book_id)
        .bind(&annotation.local_path)
        .bind(format)
        .bind(i64::from(ANCHOR_VERSION))
        .bind(&annotation.fingerprint.algorithm)
        .bind(i64::from(annotation.fingerprint.version))
        .bind(&annotation.fingerprint.bytes)
        .bind(quote.and_then(|value| value.original.as_deref()))
        .bind(quote.map(|_| QUOTE_PROFILE_V1))
        .bind(quote.map(|value| value.exact.as_str()))
        .bind(quote.map(|value| value.prefix.as_str()))
        .bind(quote.map(|value| value.suffix.as_str()))
        .bind(annotation.color.as_str())
        .bind(&annotation.body)
        .bind(provenance.map(|value| value.source_system.as_str()))
        .bind(provenance.and_then(|value| value.source_id.as_deref()))
        .bind(epub.map(|anchor| i64::from(anchor.spine_occurrence)))
        .bind(epub.map(|anchor| anchor.resource_path.as_str()))
        .bind(epub.map(|anchor| i64::from(anchor.scalar_start)))
        .bind(epub.map(|anchor| i64::from(anchor.scalar_end)))
        .bind(pdf.map(|anchor| i64::from(anchor.page)))
        .bind(char_start)
        .bind(char_end)
        .execute(&mut *transaction)
        .await
        .context("failed to insert annotation")?;

        if let Some(anchor) = pdf {
            for (index, rectangle) in anchor.rectangles.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO annotation_pdf_rectangles
                        (annotation_id, rect_index, left, bottom, right, top)
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(annotation.id.to_string())
                .bind(i64::try_from(index).context("too many PDF rectangles")?)
                .bind(rectangle.left)
                .bind(rectangle.bottom)
                .bind(rectangle.right)
                .bind(rectangle.top)
                .execute(&mut *transaction)
                .await
                .context("failed to insert PDF annotation rectangle")?;
            }
        }
        ensure_annotation_snapshot_within_limits(&mut transaction, &annotation.id).await?;
        transaction
            .commit()
            .await
            .context("failed to commit annotation")?;
        self.get_async(&annotation.id, true)
            .await?
            .context("annotation missing after insert")
    }

    pub async fn get_async(
        &self,
        id: &AnnotationId,
        include_deleted: bool,
    ) -> Result<Option<Annotation>> {
        let row =
            sqlx::query("SELECT * FROM annotations WHERE id = ? AND (? OR deleted_at IS NULL)")
                .bind(id.to_string())
                .bind(include_deleted)
                .fetch_optional(&self.pool)
                .await
                .context("failed to get annotation")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let rectangles = self
            .load_pdf_rectangles(&row.try_get::<String, _>("id")?)
            .await?;
        row_to_annotation(row, rectangles).map(Some)
    }

    pub async fn list_for_book_async(&self, book_id: i64) -> Result<Vec<Annotation>> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("failed to begin annotation list snapshot")?;
        let rows = sqlx::query(
            "SELECT * FROM annotations
             WHERE book_id = ? AND deleted_at IS NULL
             ORDER BY created_at, id",
        )
        .bind(book_id)
        .fetch_all(&mut *transaction)
        .await
        .context("failed to list annotations for book")?;
        let mut annotations = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            let rectangle_rows = sqlx::query(
                "SELECT left, bottom, right, top FROM annotation_pdf_rectangles
                 WHERE annotation_id = ? ORDER BY rect_index LIMIT ?",
            )
            .bind(id)
            .bind(i64::try_from(MAX_PDF_RECTANGLES + 1).expect("rectangle limit fits in i64"))
            .fetch_all(&mut *transaction)
            .await
            .context("failed to load PDF annotation rectangles")?;
            annotations.push(row_to_annotation(row, rows_to_rectangles(rectangle_rows)?)?);
        }
        transaction
            .commit()
            .await
            .context("failed to finish annotation list snapshot")?;
        Ok(annotations)
    }

    /// List live annotations associated with an untracked device-local path.
    pub async fn list_for_local_path_async(&self, local_path: &str) -> Result<Vec<Annotation>> {
        self.list_for_local_document_async(local_path, None).await
    }

    pub(crate) async fn list_for_local_document_async(
        &self,
        local_path: &str,
        fingerprint: Option<&DocumentFingerprint>,
    ) -> Result<Vec<Annotation>> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("failed to begin annotation list snapshot")?;
        #[cfg(test)]
        if let Some(gate) = &self.list_gate {
            gate.entered.add_permits(1);
            gate.release.acquire().await.unwrap().forget();
        }
        let rows = sqlx::query(
            "SELECT * FROM annotations
             WHERE book_id IS NULL AND local_path = ? AND deleted_at IS NULL
               AND (? IS NULL OR (
                 fingerprint_algorithm = ? AND fingerprint_version = ? AND fingerprint = ?
               ))
             ORDER BY created_at, id LIMIT ?",
        )
        .bind(local_path)
        .bind(fingerprint.map(|value| value.algorithm.as_str()))
        .bind(fingerprint.map(|value| value.algorithm.as_str()))
        .bind(fingerprint.map(|value| i64::from(value.version)))
        .bind(fingerprint.map(|value| value.bytes.as_slice()))
        .bind(
            i64::try_from(MAX_ANNOTATIONS_PER_SNAPSHOT + 1)
                .expect("annotation snapshot limit fits in i64"),
        )
        .fetch_all(&mut *transaction)
        .await
        .context("failed to list annotations for local path")?;
        if rows.len() > MAX_ANNOTATIONS_PER_SNAPSHOT {
            return Err(AnnotationSnapshotLimit.into());
        }
        let mut annotations = Vec::with_capacity(rows.len());
        let mut rectangle_count = 0usize;
        for row in rows {
            let id: String = row.try_get("id")?;
            let rectangle_rows = sqlx::query(
                "SELECT left, bottom, right, top FROM annotation_pdf_rectangles WHERE annotation_id = ? ORDER BY rect_index LIMIT ?",
            )
            .bind(id)
            .bind(i64::try_from(MAX_PDF_RECTANGLES + 1).expect("rectangle limit fits in i64"))
            .fetch_all(&mut *transaction)
            .await
            .context("failed to load PDF annotation rectangles")?;
            rectangle_count = rectangle_count
                .checked_add(rectangle_rows.len())
                .ok_or(AnnotationSnapshotLimit)?;
            if rectangle_count > MAX_PDF_RECTANGLES_PER_SNAPSHOT {
                return Err(AnnotationSnapshotLimit.into());
            }
            annotations.push(row_to_annotation(row, rows_to_rectangles(rectangle_rows)?)?);
        }
        transaction
            .commit()
            .await
            .context("failed to finish annotation list snapshot")?;
        Ok(annotations)
    }

    pub async fn update_async(
        &self,
        id: &AnnotationId,
        color: HighlightColor,
        body: Option<&str>,
    ) -> Result<bool> {
        if let Some(body) = body {
            ensure_scalar_limit(body, MAX_ANNOTATION_BODY_SCALARS, "annotation body")?;
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("failed to begin annotation update")?;
        let result = sqlx::query(
            "UPDATE annotations
             SET color = ?, body = ?, modified_at =
                 CASE
                     WHEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') > modified_at
                     THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE strftime('%Y-%m-%dT%H:%M:%fZ', modified_at, '+0.001 seconds')
                 END
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(color.as_str())
        .bind(body)
        .bind(id.to_string())
        .execute(&mut *transaction)
        .await
        .context("failed to update annotation")?;
        if result.rows_affected() == 1 {
            ensure_annotation_snapshot_within_limits(&mut transaction, id).await?;
        }
        transaction
            .commit()
            .await
            .context("failed to commit annotation update")?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete_async(&self, id: &AnnotationId) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE annotations
             SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 modified_at =
                 CASE
                     WHEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') > modified_at
                     THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE strftime('%Y-%m-%dT%H:%M:%fZ', modified_at, '+0.001 seconds')
                 END
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .context("failed to delete annotation")?;
        Ok(result.rows_affected() == 1)
    }

    async fn load_pdf_rectangles(&self, annotation_id: &str) -> Result<Vec<PageRect>> {
        let rows = sqlx::query(
            "SELECT left, bottom, right, top FROM annotation_pdf_rectangles
             WHERE annotation_id = ? ORDER BY rect_index LIMIT ?",
        )
        .bind(annotation_id)
        .bind(i64::try_from(MAX_PDF_RECTANGLES + 1).expect("rectangle limit fits in i64"))
        .fetch_all(&self.pool)
        .await
        .context("failed to load PDF annotation rectangles")?;
        rows_to_rectangles(rows)
    }
}

async fn ensure_annotation_snapshot_within_limits(
    connection: &mut SqliteConnection,
    annotation_id: &AnnotationId,
) -> Result<()> {
    let usage = sqlx::query(
        "WITH target AS (
           SELECT book_id, local_path, fingerprint_algorithm, fingerprint_version, fingerprint
           FROM annotations WHERE id = ?
         )
         SELECT COUNT(*) AS annotation_count,
                COALESCE(SUM(
                  LENGTH(CAST(a.id AS BLOB)) +
                  LENGTH(CAST(COALESCE(a.body, '') AS BLOB)) +
                  LENGTH(CAST(COALESCE(a.original_quote, '') AS BLOB)) +
                  LENGTH(CAST(COALESCE(a.normalized_exact, '') AS BLOB)) +
                  LENGTH(CAST(COALESCE(a.normalized_prefix, '') AS BLOB)) +
                  LENGTH(CAST(COALESCE(a.normalized_suffix, '') AS BLOB))
                ), 0) AS string_bytes
         FROM annotations a, target t
         WHERE a.deleted_at IS NULL AND (
           (t.book_id IS NOT NULL AND a.book_id = t.book_id) OR
           (t.book_id IS NULL AND a.book_id IS NULL AND
            a.local_path = t.local_path AND
            a.fingerprint_algorithm = t.fingerprint_algorithm AND
            a.fingerprint_version = t.fingerprint_version AND
            a.fingerprint = t.fingerprint)
         )",
    )
    .bind(annotation_id.to_string())
    .fetch_one(&mut *connection)
    .await
    .context("failed to measure annotation snapshot")?;
    let annotation_count = usize::try_from(usage.try_get::<i64, _>("annotation_count")?)
        .map_err(|_| AnnotationSnapshotLimit)?;
    let string_bytes = usize::try_from(usage.try_get::<i64, _>("string_bytes")?)
        .map_err(|_| AnnotationSnapshotLimit)?;
    let rectangle_count = usize::try_from(
        sqlx::query_scalar::<_, i64>(
            "WITH target AS (
               SELECT book_id, local_path, fingerprint_algorithm, fingerprint_version, fingerprint
               FROM annotations WHERE id = ?
             )
             SELECT COUNT(*)
             FROM annotation_pdf_rectangles r
             JOIN annotations a ON a.id = r.annotation_id
             JOIN target t
             WHERE a.deleted_at IS NULL AND (
               (t.book_id IS NOT NULL AND a.book_id = t.book_id) OR
               (t.book_id IS NULL AND a.book_id IS NULL AND
                a.local_path = t.local_path AND
                a.fingerprint_algorithm = t.fingerprint_algorithm AND
                a.fingerprint_version = t.fingerprint_version AND
                a.fingerprint = t.fingerprint)
             )",
        )
        .bind(annotation_id.to_string())
        .fetch_one(&mut *connection)
        .await
        .context("failed to measure annotation rectangles")?,
    )
    .map_err(|_| AnnotationSnapshotLimit)?;
    let retained_bytes = annotation_count
        .checked_mul(ANNOTATION_SNAPSHOT_BASE_BYTES)
        .and_then(|base| base.checked_add(string_bytes))
        .and_then(|bytes| {
            rectangle_count
                .checked_mul(std::mem::size_of::<PageRect>())
                .and_then(|rectangles| bytes.checked_add(rectangles))
        })
        .ok_or(AnnotationSnapshotLimit)?;

    ensure_annotation_snapshot_usage(annotation_count, retained_bytes, rectangle_count)
}

fn ensure_annotation_snapshot_usage(
    annotation_count: usize,
    retained_bytes: usize,
    rectangle_count: usize,
) -> Result<()> {
    if annotation_count > MAX_ANNOTATIONS_PER_SNAPSHOT
        || retained_bytes > MAX_ANNOTATION_SNAPSHOT_BYTES
        || rectangle_count > MAX_PDF_RECTANGLES_PER_SNAPSHOT
    {
        return Err(AnnotationSnapshotLimit.into());
    }
    Ok(())
}

fn row_to_annotation(row: SqliteRow, rectangles: Vec<PageRect>) -> Result<Annotation> {
    let id_text: String = row.try_get("id")?;
    let id = AnnotationId::from_str(&id_text).context("invalid annotation ID in database")?;
    let anchor_version: i64 = row.try_get("anchor_version")?;
    if anchor_version != i64::from(ANCHOR_VERSION) {
        bail!("unsupported annotation anchor version {anchor_version}");
    }
    let fingerprint_version =
        positive_u32(row.try_get("fingerprint_version")?, "fingerprint version")?;
    let fingerprint = DocumentFingerprint::new(
        row.try_get::<String, _>("fingerprint_algorithm")?,
        fingerprint_version,
        row.try_get("fingerprint")?,
    )?;
    let profile: Option<String> = row.try_get("normalization_profile")?;
    let quote = match profile.as_deref() {
        None => None,
        Some(QUOTE_PROFILE_V1) => Some(QuoteSelector {
            original: row.try_get("original_quote")?,
            exact: row.try_get("normalized_exact")?,
            prefix: row.try_get("normalized_prefix")?,
            suffix: row.try_get("normalized_suffix")?,
        }),
        Some(profile) => bail!("unsupported annotation quote profile {profile:?}"),
    };
    if let Some(quote) = &quote {
        quote.validate()?;
    }
    let target = match row.try_get::<String, _>("format")?.as_str() {
        "epub" => AnnotationTarget::Epub(EpubAnchor::new(
            nonnegative_u32(
                row.try_get("epub_spine_occurrence")?,
                "EPUB spine occurrence",
            )?,
            row.try_get::<String, _>("epub_resource_path")?,
            nonnegative_u32(row.try_get("epub_scalar_start")?, "EPUB scalar start")?,
            nonnegative_u32(row.try_get("epub_scalar_end")?, "EPUB scalar end")?,
        )?),
        "pdf" => {
            let start: Option<i64> = row.try_get("pdf_char_start")?;
            let end: Option<i64> = row.try_get("pdf_char_end")?;
            let character_range = match (start, end) {
                (Some(start), Some(end)) => Some((
                    nonnegative_u32(start, "PDF character start")?,
                    nonnegative_u32(end, "PDF character end")?,
                )),
                (None, None) => None,
                _ => bail!("incomplete PDF character range in database"),
            };
            AnnotationTarget::Pdf(PdfAnchor::new(
                nonnegative_u32(row.try_get("pdf_page")?, "PDF page")?,
                character_range,
                rectangles,
            )?)
        }
        format => bail!("unknown annotation format {format:?}"),
    };
    let provenance = match row.try_get::<Option<String>, _>("source_system")? {
        Some(source_system) => Some(ImportProvenance {
            source_system,
            source_id: row.try_get("source_id")?,
        }),
        None => None,
    };
    let annotation = Annotation {
        id,
        book_id: row.try_get("book_id")?,
        local_path: row.try_get("local_path")?,
        fingerprint,
        quote,
        target,
        color: HighlightColor::from_db(&row.try_get::<String, _>("color")?)?,
        body: row.try_get("body")?,
        provenance,
        created_at: row.try_get("created_at")?,
        modified_at: row.try_get("modified_at")?,
        deleted_at: row.try_get("deleted_at")?,
    };
    NewAnnotation {
        id: annotation.id.clone(),
        book_id: annotation.book_id,
        local_path: annotation.local_path.clone(),
        fingerprint: annotation.fingerprint.clone(),
        quote: annotation.quote.clone(),
        target: annotation.target.clone(),
        color: annotation.color,
        body: annotation.body.clone(),
        provenance: annotation.provenance.clone(),
    }
    .validate()?;
    Ok(annotation)
}

fn rows_to_rectangles(rows: Vec<SqliteRow>) -> Result<Vec<PageRect>> {
    if rows.len() > MAX_PDF_RECTANGLES {
        bail!("PDF annotation exceeds {MAX_PDF_RECTANGLES} rectangles");
    }
    rows.into_iter()
        .map(|rectangle| {
            PageRect::new(
                rectangle.try_get("left")?,
                rectangle.try_get("bottom")?,
                rectangle.try_get("right")?,
                rectangle.try_get("top")?,
            )
        })
        .collect()
}

pub fn normalize_quote_v1(value: &str) -> String {
    let line_normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = line_normalized
        .chars()
        .filter(|character| *character != '\u{00ad}')
        .collect::<String>()
        .nfc()
        .collect::<String>();
    let mut result = String::new();
    let mut pending_space = false;
    for character in normalized.chars() {
        if quote_v1_whitespace(character) {
            pending_space = !result.is_empty();
        } else {
            if pending_space {
                result.push(' ');
                pending_space = false;
            }
            result.push(character);
        }
    }
    result
}

/// Convert a half-open Unicode-scalar range to the UTF-16 units required by EPUB CFI.
pub fn scalar_range_to_utf16(text: &str, range: Range<u32>) -> Result<Range<u32>> {
    if range.start > range.end {
        bail!("Unicode-scalar range is reversed");
    }
    let scalar_count = u32::try_from(text.chars().count()).context("text is too large")?;
    if range.end > scalar_count {
        bail!("Unicode-scalar range exceeds text length");
    }
    let mut utf16_start = None;
    let mut utf16_end = None;
    let mut utf16_offset = 0_u32;
    for (scalar_offset, character) in text.chars().enumerate() {
        let scalar_offset = u32::try_from(scalar_offset).context("text is too large")?;
        if scalar_offset == range.start {
            utf16_start = Some(utf16_offset);
        }
        if scalar_offset == range.end {
            utf16_end = Some(utf16_offset);
            break;
        }
        utf16_offset = utf16_offset
            .checked_add(character.len_utf16() as u32)
            .context("UTF-16 offset overflow")?;
    }
    if range.start == scalar_count {
        utf16_start = Some(utf16_offset);
    }
    if range.end == scalar_count {
        utf16_end = Some(utf16_offset);
    }
    Ok(utf16_start.context("missing UTF-16 range start")?
        ..utf16_end.context("missing UTF-16 range end")?)
}

#[derive(Clone, Copy)]
enum ContextDirection {
    Prefix,
    Suffix,
}

fn quote_context_v1(value: &str, direction: ContextDirection) -> String {
    let normalized = normalize_quote_v1(value);
    let graphemes = normalized.graphemes(true).collect::<Vec<_>>();
    match direction {
        ContextDirection::Prefix => {
            let mut scalars = 0;
            let start = graphemes
                .iter()
                .rposition(|grapheme| {
                    let next = scalars + grapheme.chars().count();
                    if next <= MAX_CONTEXT_SCALARS {
                        scalars = next;
                        false
                    } else {
                        true
                    }
                })
                .map_or(0, |index| index + 1);
            graphemes[start..].concat().trim_start().to_owned()
        }
        ContextDirection::Suffix => {
            let mut scalars = 0;
            let end = graphemes
                .iter()
                .position(|grapheme| {
                    let next = scalars + grapheme.chars().count();
                    if next <= MAX_CONTEXT_SCALARS {
                        scalars = next;
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(graphemes.len());
            graphemes[..end].concat().trim_end().to_owned()
        }
    }
}

fn quote_v1_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

fn nonnegative_u32(value: i64, field: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| anyhow!("invalid {field} in annotation database"))
}

fn positive_u32(value: i64, field: &str) -> Result<u32> {
    let value = nonnegative_u32(value, field)?;
    if value == 0 {
        bail!("invalid {field} in annotation database");
    }
    Ok(value)
}

fn ensure_scalar_limit(value: &str, limit: usize, field: &str) -> Result<()> {
    if value.chars().take(limit + 1).count() > limit {
        bail!("{field} exceeds {limit} Unicode scalars");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_anchor_resolution_distinguishes_exact_and_unique_recovery() {
        let original = "lead Cafe\u{301}\ttext tail";
        let quote = QuoteSelector::new("Cafe\u{301}\ttext", "lead ", " tail").unwrap();
        assert_eq!(
            resolve_text_anchor(original, 5..15, &quote),
            ResolvedTextAnchor {
                resolution: AnnotationResolution::Exact,
                range: Some(5..15),
            }
        );

        let changed = "inserted lead Café text tail";
        assert_eq!(
            resolve_text_anchor(changed, 5..15, &quote),
            ResolvedTextAnchor {
                resolution: AnnotationResolution::Recovered,
                range: Some(14..23),
            }
        );
    }

    #[test]
    fn text_anchor_resolution_uses_context_without_guessing_repeated_quotes() {
        let contextual = QuoteSelector::new("target", "alpha ", " omega").unwrap();
        assert_eq!(
            resolve_text_anchor("target noise alpha target omega", 1..7, &contextual),
            ResolvedTextAnchor {
                resolution: AnnotationResolution::Recovered,
                range: Some(19..25),
            }
        );

        let ambiguous = QuoteSelector::new("target", "", "").unwrap();
        assert_eq!(
            resolve_text_anchor("target target", 1..7, &ambiguous),
            ResolvedTextAnchor {
                resolution: AnnotationResolution::Ambiguous,
                range: None,
            }
        );
        let overlapping = QuoteSelector::new("aa", "", "").unwrap();
        assert_eq!(
            resolve_text_anchor("aaa", 1..2, &overlapping).resolution,
            AnnotationResolution::Ambiguous,
        );
    }

    #[test]
    fn text_anchor_resolution_reports_missing_quotes_as_orphaned() {
        let quote = QuoteSelector::new("missing", "", "").unwrap();
        assert_eq!(
            resolve_text_anchor("other text", 0..7, &quote),
            ResolvedTextAnchor {
                resolution: AnnotationResolution::Orphaned,
                range: None,
            }
        );
    }

    #[test]
    fn text_anchor_resolution_preserves_the_normalization_profile_at_source_boundaries() {
        let composed = QuoteSelector::new("é", "", "").unwrap();
        assert_eq!(
            resolve_text_anchor("xe\u{00ad}\u{0301}", 0..1, &composed),
            ResolvedTextAnchor {
                resolution: AnnotationResolution::Recovered,
                range: Some(1..4),
            }
        );

        let partial_cluster = QuoteSelector::new("👩", "", "").unwrap();
        let resolved = resolve_text_anchor("x👩‍💻", 0..1, &partial_cluster);
        assert_eq!(resolved.resolution, AnnotationResolution::Orphaned);
        assert!(resolved.range.is_none());
    }

    #[test]
    fn text_anchor_resolution_observes_cancellation_and_work_limits() {
        let checks = std::cell::Cell::new(0);
        assert!(matches!(
            TextAnchorResolver::new(&"x".repeat(2_048), &|| {
                checks.set(checks.get() + 1);
                checks.get() > 1
            }),
            Err(TextAnchorResolutionError::Cancelled)
        ));

        let resolver = TextAnchorResolver::new("different", &|| false).unwrap();
        let quote = QuoteSelector::new("missing", "", "").unwrap();
        assert!(matches!(
            resolver.resolve(0..1, &quote, &mut 0, &|| false),
            Err(TextAnchorResolutionError::WorkLimit)
        ));
    }

    #[test]
    fn aggregate_snapshot_usage_enforces_each_limit() {
        assert!(
            ensure_annotation_snapshot_usage(
                MAX_ANNOTATIONS_PER_SNAPSHOT,
                MAX_ANNOTATION_SNAPSHOT_BYTES,
                MAX_PDF_RECTANGLES_PER_SNAPSHOT,
            )
            .is_ok()
        );
        for usage in [
            (
                MAX_ANNOTATIONS_PER_SNAPSHOT + 1,
                MAX_ANNOTATION_SNAPSHOT_BYTES,
                MAX_PDF_RECTANGLES_PER_SNAPSHOT,
            ),
            (
                MAX_ANNOTATIONS_PER_SNAPSHOT,
                MAX_ANNOTATION_SNAPSHOT_BYTES + 1,
                MAX_PDF_RECTANGLES_PER_SNAPSHOT,
            ),
            (
                MAX_ANNOTATIONS_PER_SNAPSHOT,
                MAX_ANNOTATION_SNAPSHOT_BYTES,
                MAX_PDF_RECTANGLES_PER_SNAPSHOT + 1,
            ),
        ] {
            let error = ensure_annotation_snapshot_usage(usage.0, usage.1, usage.2).unwrap_err();
            assert!(error.is::<AnnotationSnapshotLimit>());
        }
    }
}
