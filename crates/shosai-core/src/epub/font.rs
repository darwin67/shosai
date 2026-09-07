//! Book-local admission of author supplied EPUB fonts.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    io::Read,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    sync::{Arc, Mutex},
};

use anyhow::Result;
use fontdb::{Database, Source};
use lightningcss::{
    properties::font::{AbsoluteFontWeight, FontFamily, FontWeight},
    rules::{
        CssRule,
        font_face::{FontFaceProperty, FontFormat as CssFormat, Source as CssSource},
    },
    stylesheet::{ParserOptions, StyleSheet},
    traits::ToCss,
};
use unicode_casefold::UnicodeCaseFold;

use super::{CanonicalEpubPath, Chapter, EpubLimits, style::EpubStyles, types::StoredEpubResource};

const MAX_DIAGNOSTIC_LABEL_BYTES: usize = 256;
static NEXT_NATIVE_TEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EpubFontFormat {
    TrueType,
    OpenType,
    Woff,
    Woff2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EpubFontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Copy, Debug)]
pub struct EpubFontWeight {
    min: f32,
    max: f32,
}

impl EpubFontWeight {
    const NORMAL: Self = Self {
        min: 400.0,
        max: 400.0,
    };

    pub fn min(self) -> f32 {
        self.min
    }
    pub fn max(self) -> f32 {
        self.max
    }
}

impl PartialEq for EpubFontWeight {
    fn eq(&self, other: &Self) -> bool {
        self.min.to_bits() == other.min.to_bits() && self.max.to_bits() == other.max.to_bits()
    }
}

impl Eq for EpubFontWeight {}

impl Hash for EpubFontWeight {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.min.to_bits().hash(state);
        self.max.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EpubFontAttempt {
    Rejected {
        source: String,
        reason: String,
    },
    Loaded {
        path: CanonicalEpubPath,
        format: EpubFontFormat,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpubFontFace {
    pub family: String,
    pub style: EpubFontStyle,
    pub weight: EpubFontWeight,
    pub path: CanonicalEpubPath,
    pub format: EpubFontFormat,
    pub decoded_bytes: usize,
    pub attempts: Vec<EpubFontAttempt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpubRejectedFontFace {
    pub family: String,
    pub attempts: Vec<EpubFontAttempt>,
}

/// Fonts admitted for one EPUB. The backing database and its binary sources are
/// deliberately private, and are destroyed together with this value.
pub struct EpubFontBook {
    database: Database,
    registered_ids: Vec<fontdb::ID>,
    faces: Vec<EpubFontFace>,
    rejected_faces: Vec<EpubRejectedFontFace>,
    chapter_families: HashMap<String, HashSet<String>>,
    decoded_bytes: usize,
    native_text_id: u64,
    pub(super) native: Mutex<super::native_text::NativeTextState>,
    #[cfg(test)]
    pub(super) renderer_entries: Mutex<Option<Arc<AtomicU64>>>,
}

impl fmt::Debug for EpubFontBook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EpubFontBook")
            .field("faces", &self.faces)
            .field("rejected_faces", &self.rejected_faces)
            .field("decoded_bytes", &self.decoded_bytes)
            .finish_non_exhaustive()
    }
}

impl EpubFontBook {
    #[cfg(test)]
    fn observe_renderer_entries(&self, entries: Arc<AtomicU64>) {
        *self.renderer_entries.lock().unwrap() = Some(entries);
    }

    pub(crate) fn retained_decoded_bytes(&self) -> usize {
        self.decoded_bytes
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
    pub fn len(&self) -> usize {
        self.faces.len()
    }
    pub fn faces(&self) -> &[EpubFontFace] {
        &self.faces
    }
    pub fn rejected_faces(&self) -> &[EpubRejectedFontFace] {
        &self.rejected_faces
    }
    pub fn registered_face_count(&self) -> usize {
        self.database.len()
    }
    /// Stable per-book identity for renderer cache isolation.
    pub fn native_text_id(&self) -> u64 {
        self.native_text_id
    }
    /// Borrow one admitted decoded sfnt face without exposing the book-local database.
    pub fn with_face_data<T>(&self, index: usize, read: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
        self.database
            .with_face_data(*self.registered_ids.get(index)?, read)
    }
    pub fn contains_family(&self, family: &str) -> bool {
        let family = folded_family(family);
        self.faces
            .iter()
            .any(|face| folded_family(&face.family) == family)
    }
    pub(crate) fn contains_family_for_chapter(&self, chapter_path: &str, family: &str) -> bool {
        let family = folded_family(family);
        self.chapter_families
            .get(chapter_path)
            .is_some_and(|families| families.contains(&family))
    }

    #[cfg(test)]
    pub(crate) fn new(
        chapters: &[Chapter],
        styles: &EpubStyles,
        resources: &HashMap<CanonicalEpubPath, StoredEpubResource>,
        limits: &EpubLimits,
    ) -> Result<Self> {
        Self::new_cancellable(chapters, styles, resources, limits, None)
    }

    pub(crate) fn new_cancellable(
        chapters: &[Chapter],
        styles: &EpubStyles,
        resources: &HashMap<CanonicalEpubPath, StoredEpubResource>,
        limits: &EpubLimits,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self> {
        let mut book = Self {
            database: Database::new(),
            registered_ids: Vec::new(),
            faces: Vec::new(),
            rejected_faces: Vec::new(),
            chapter_families: HashMap::new(),
            decoded_bytes: 0,
            native_text_id: NEXT_NATIVE_TEXT_ID.fetch_add(1, AtomicOrdering::Relaxed),
            native: Mutex::new(super::native_text::NativeTextState::empty()),
            #[cfg(test)]
            renderer_entries: Mutex::new(None),
        };
        let mut inspected = HashMap::<Descriptor, bool>::new();
        let mut descriptor_limit_reported = false;
        for chapter in chapters {
            check_cancelled(is_cancelled)?;
            let Ok(normalized) =
                super::render::bounded_chapter_xhtml(&chapter.content, &chapter.path, limits)
            else {
                continue;
            };
            let options = super::render::xhtml_parsing_options(limits);
            let Ok(document) = roxmltree::Document::parse_with_options(&normalized, options) else {
                continue;
            };
            let base = chapter.path.rsplit_once('/').map_or("", |(dir, _)| dir);
            let css =
                styles.document_css_with_owner(&document, base, Some(&chapter.path), limits)?;
            let descriptors = parse_faces(&css, limits, &inspected)?;
            for descriptor in descriptors {
                check_cancelled(is_cancelled)?;
                if let Some(admitted) = inspected.get(&descriptor) {
                    if *admitted {
                        book.chapter_families
                            .entry(chapter.path.clone())
                            .or_default()
                            .insert(folded_family(&descriptor.family));
                    }
                    continue;
                }
                if inspected.len() >= limits.max_font_face_descriptors_per_book {
                    if !descriptor_limit_reported {
                        book.rejected_faces.push(EpubRejectedFontFace {
                            family: bounded_diagnostic(&descriptor.family),
                            attempts: vec![EpubFontAttempt::Rejected {
                                source: "@font-face".into(),
                                reason: "per-book font descriptor inspection limit is exhausted"
                                    .into(),
                            }],
                        });
                        descriptor_limit_reported = true;
                    }
                    continue;
                }
                if descriptor.sources.len() > limits.max_font_sources_per_face {
                    book.rejected_faces.push(EpubRejectedFontFace {
                        family: bounded_diagnostic(&descriptor.family),
                        attempts: vec![EpubFontAttempt::Rejected {
                            source: "@font-face".into(),
                            reason: "font source inspection limit is exhausted".into(),
                        }],
                    });
                    inspected.insert(descriptor, false);
                    continue;
                }
                if descriptor.family.len() > limits.max_css_font_family_name_bytes {
                    book.rejected_faces.push(EpubRejectedFontFace {
                        family: bounded_diagnostic(&descriptor.family),
                        attempts: vec![EpubFontAttempt::Rejected {
                            source: "@font-face".into(),
                            reason: "font family name exceeds the byte limit".into(),
                        }],
                    });
                    inspected.insert(descriptor, false);
                    continue;
                }
                if book.faces.len() >= limits.max_font_faces_per_book {
                    book.rejected_faces.push(EpubRejectedFontFace {
                        family: bounded_diagnostic(&descriptor.family),
                        attempts: vec![EpubFontAttempt::Rejected {
                            source: "@font-face".into(),
                            reason: "per-book admitted font face limit is exhausted".into(),
                        }],
                    });
                    inspected.insert(descriptor, false);
                    continue;
                }
                let family = descriptor.family.clone();
                let admitted =
                    book.load_descriptor(descriptor.clone(), resources, limits, is_cancelled)?;
                inspected.insert(descriptor, admitted);
                if admitted {
                    book.chapter_families
                        .entry(chapter.path.clone())
                        .or_default()
                        .insert(folded_family(&family));
                }
            }
        }
        check_cancelled(is_cancelled)?;
        let native = super::native_text::NativeTextState::new_cancellable(
            &book.database,
            &book.registered_ids,
            &book.faces,
            is_cancelled,
        )?;
        check_cancelled(is_cancelled)?;
        book.native = Mutex::new(native);
        Ok(book)
    }

    fn load_descriptor(
        &mut self,
        descriptor: Descriptor,
        resources: &HashMap<CanonicalEpubPath, StoredEpubResource>,
        limits: &EpubLimits,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<bool> {
        let mut attempts = Vec::new();
        for source in &descriptor.sources {
            check_cancelled(is_cancelled)?;
            let (reference, format, technology) = match source {
                SourceDescriptor::Local(_) => {
                    reject(&mut attempts, source.label(), "local fonts are disabled");
                    continue;
                }
                SourceDescriptor::Rejected { label, reason } => {
                    reject(&mut attempts, label, reason);
                    continue;
                }
                SourceDescriptor::Url {
                    reference,
                    format,
                    technology,
                } => (reference, format, technology),
            };
            if *technology {
                reject(
                    &mut attempts,
                    reference,
                    "font technology descriptor is unsupported",
                );
                continue;
            }
            let declared = match format {
                FormatHint::Absent => None,
                FormatHint::Supported(v) => Some(*v),
                FormatHint::Unsupported => {
                    reject(
                        &mut attempts,
                        reference,
                        "font format descriptor is unsupported",
                    );
                    continue;
                }
            };
            let reference_path = match CanonicalEpubPath::from_protocol_uri(reference) {
                Ok(value) if value.fragment.is_none() => value.path,
                Ok(_) => {
                    reject(
                        &mut attempts,
                        reference,
                        "font references cannot contain fragments",
                    );
                    continue;
                }
                Err(error) => {
                    reject(&mut attempts, reference, error.to_string());
                    continue;
                }
            };
            let Some(resource) = resources.get(&reference_path) else {
                reject(
                    &mut attempts,
                    reference_path.as_str(),
                    "font resource is missing",
                );
                continue;
            };
            let (format, decoded) =
                match decode_font(&resource.bytes, declared, limits, is_cancelled) {
                    Ok(value) => value,
                    Err(error) => {
                        check_cancelled(is_cancelled)?;
                        reject(&mut attempts, reference_path.as_str(), error);
                        continue;
                    }
                };
            check_cancelled(is_cancelled)?;
            if self
                .decoded_bytes
                .checked_add(decoded.len())
                .is_none_or(|n| n > limits.max_total_decoded_font_bytes)
            {
                reject(
                    &mut attempts,
                    reference_path.as_str(),
                    "per-book decoded font budget is exhausted",
                );
                continue;
            }
            let decoded_bytes = decoded.len();
            let ids = self
                .database
                .load_font_source(Source::Binary(Arc::new(decoded)));
            check_cancelled(is_cancelled)?;
            if ids.is_empty() {
                reject(
                    &mut attempts,
                    reference_path.as_str(),
                    "fontdb rejected the decoded font",
                );
                continue;
            }
            if ids.len() != 1 {
                for id in ids {
                    self.database.remove_face(id);
                }
                reject(
                    &mut attempts,
                    reference_path.as_str(),
                    "font collections are unsupported",
                );
                continue;
            }
            attempts.push(EpubFontAttempt::Loaded {
                path: reference_path.clone(),
                format,
            });
            self.decoded_bytes += decoded_bytes;
            self.registered_ids.push(ids[0]);
            self.faces.push(EpubFontFace {
                family: descriptor.family,
                style: descriptor.style,
                weight: descriptor.weight,
                path: reference_path,
                format,
                decoded_bytes,
                attempts,
            });
            return Ok(true);
        }
        self.rejected_faces.push(EpubRejectedFontFace {
            family: descriptor.family,
            attempts,
        });
        Ok(false)
    }
}

fn check_cancelled(is_cancelled: Option<&dyn Fn() -> bool>) -> Result<()> {
    if is_cancelled.is_some_and(|is_cancelled| is_cancelled()) {
        anyhow::bail!("import cancelled");
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Descriptor {
    family: String,
    style: EpubFontStyle,
    weight: EpubFontWeight,
    sources: Vec<SourceDescriptor>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SourceDescriptor {
    Local(String),
    Rejected {
        label: String,
        reason: String,
    },
    Url {
        reference: String,
        format: FormatHint,
        technology: bool,
    },
}
impl SourceDescriptor {
    fn label(&self) -> String {
        match self {
            Self::Local(v) => format!("local({v})"),
            Self::Rejected { label, .. } => label.clone(),
            Self::Url { reference, .. } => reference.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FormatHint {
    Absent,
    Supported(EpubFontFormat),
    Unsupported,
}

fn text<T: ToCss>(value: &T) -> Option<String> {
    value.to_css_string(Default::default()).ok()
}

fn folded_family(value: &str) -> String {
    value.case_fold().collect()
}

fn absolute_weight(value: &FontWeight) -> Option<f32> {
    let value = match value {
        FontWeight::Absolute(AbsoluteFontWeight::Weight(value)) => *value,
        FontWeight::Absolute(AbsoluteFontWeight::Normal) => 400.0,
        FontWeight::Absolute(AbsoluteFontWeight::Bold) => 700.0,
        FontWeight::Bolder | FontWeight::Lighter => return None,
    };
    (value.is_finite() && (1.0..=1_000.0).contains(&value)).then_some(value)
}

fn bounded_diagnostic(value: &str) -> String {
    if value.len() <= MAX_DIAGNOSTIC_LABEL_BYTES {
        return value.to_owned();
    }
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= MAX_DIAGNOSTIC_LABEL_BYTES - 3)
        .last()
        .unwrap_or(0);
    format!("{}...", &value[..end])
}

fn parse_faces(
    css: &str,
    limits: &EpubLimits,
    globally_inspected: &HashMap<Descriptor, bool>,
) -> Result<Vec<Descriptor>> {
    let Ok(sheet) = StyleSheet::parse(css, ParserOptions::default()) else {
        return Ok(Vec::new());
    };
    super::computed_style::validate_stylesheet_complexity(&sheet.rules.0, limits)?;
    fn collect(
        rules: &[CssRule<'_>],
        faces: &mut Vec<Descriptor>,
        seen: &mut HashSet<Descriptor>,
        globally_inspected: &HashMap<Descriptor, bool>,
        remaining_new: &mut usize,
        limits: &EpubLimits,
    ) {
        for rule in rules {
            if let CssRule::Media(media) = rule {
                if super::computed_style::screen_media_matches(&media.query) {
                    collect(
                        &media.rules.0,
                        faces,
                        seen,
                        globally_inspected,
                        remaining_new,
                        limits,
                    );
                }
                continue;
            }
            let CssRule::FontFace(rule) = rule else {
                continue;
            };
            let Some(descriptor) = (|| -> Option<Descriptor> {
                let (mut family, mut style, mut weight, mut sources) = (
                    None,
                    EpubFontStyle::Normal,
                    EpubFontWeight::NORMAL,
                    Vec::new(),
                );
                for property in &rule.properties {
                    match property {
                        FontFaceProperty::FontFamily(FontFamily::FamilyName(v)) => {
                            family = super::computed_style::decoded_family_name(v)
                        }
                        FontFaceProperty::FontStyle(v) => {
                            style = match text(v)?.to_ascii_lowercase().as_str() {
                                "italic" => EpubFontStyle::Italic,
                                v if v.starts_with("oblique") => EpubFontStyle::Oblique,
                                _ => EpubFontStyle::Normal,
                            }
                        }
                        FontFaceProperty::FontWeight(v) => {
                            if let (Some(min), Some(max)) =
                                (absolute_weight(&v.0), absolute_weight(&v.1))
                                && min <= max
                            {
                                weight = EpubFontWeight { min, max };
                            }
                        }
                        FontFaceProperty::Source(values) => {
                            sources = values
                                .iter()
                                .map(|source| match source {
                                    CssSource::Local(v) => SourceDescriptor::Local(match v {
                                        FontFamily::FamilyName(name) => bounded_diagnostic(
                                            &super::computed_style::decoded_family_name(name)
                                                .unwrap_or_default(),
                                        ),
                                        FontFamily::Generic(_) => {
                                            bounded_diagnostic(&text(v).unwrap_or_default())
                                        }
                                    }),
                                    CssSource::Url(v) => {
                                        let reference = v.url.url.to_string();
                                        if reference.len() > limits.max_font_source_reference_bytes
                                        {
                                            SourceDescriptor::Rejected {
                                                label: bounded_diagnostic(&reference),
                                                reason:
                                                    "font source reference exceeds the byte limit"
                                                        .into(),
                                            }
                                        } else {
                                            SourceDescriptor::Url {
                                                reference,
                                                format: v.format.as_ref().map_or(
                                                    FormatHint::Absent,
                                                    |f| match f {
                                                        CssFormat::TrueType => {
                                                            FormatHint::Supported(
                                                                EpubFontFormat::TrueType,
                                                            )
                                                        }
                                                        CssFormat::OpenType => {
                                                            FormatHint::Supported(
                                                                EpubFontFormat::OpenType,
                                                            )
                                                        }
                                                        CssFormat::WOFF => FormatHint::Supported(
                                                            EpubFontFormat::Woff,
                                                        ),
                                                        CssFormat::WOFF2 => FormatHint::Supported(
                                                            EpubFontFormat::Woff2,
                                                        ),
                                                        _ => FormatHint::Unsupported,
                                                    },
                                                ),
                                                technology: !v.tech.is_empty(),
                                            }
                                        }
                                    }
                                })
                                .collect()
                        }
                        _ => {}
                    }
                }
                let family = family?;
                Some(Descriptor {
                    family,
                    style,
                    weight,
                    sources,
                })
            })() else {
                continue;
            };
            if seen.insert(descriptor.clone())
                && (globally_inspected.contains_key(&descriptor) || *remaining_new > 0)
            {
                if !globally_inspected.contains_key(&descriptor) {
                    *remaining_new -= 1;
                }
                faces.push(descriptor);
            }
        }
    }
    let mut faces = Vec::new();
    let mut seen = HashSet::new();
    let mut remaining_new = limits
        .max_font_face_descriptors_per_book
        .saturating_sub(globally_inspected.len())
        .saturating_add(1);
    collect(
        &sheet.rules.0,
        &mut faces,
        &mut seen,
        globally_inspected,
        &mut remaining_new,
        limits,
    );
    Ok(faces)
}

fn reject(
    attempts: &mut Vec<EpubFontAttempt>,
    source: impl Into<String>,
    reason: impl Into<String>,
) {
    attempts.push(EpubFontAttempt::Rejected {
        source: bounded_diagnostic(&source.into()),
        reason: bounded_diagnostic(&reason.into()),
    });
}
fn read_u16(b: &[u8], o: usize) -> std::result::Result<usize, String> {
    b.get(o..o + 2)
        .map(|v| u16::from_be_bytes(v.try_into().unwrap()) as usize)
        .ok_or_else(|| "compressed font header is truncated".into())
}
fn read_u32(b: &[u8], o: usize) -> std::result::Result<usize, String> {
    b.get(o..o + 4)
        .map(|v| u32::from_be_bytes(v.try_into().unwrap()) as usize)
        .ok_or_else(|| "compressed font header is truncated".into())
}
fn aligned(v: usize) -> std::result::Result<usize, String> {
    v.checked_add(3)
        .map(|v| v & !3)
        .ok_or_else(|| "font size arithmetic overflowed".into())
}
fn header(bytes: &[u8], limits: &EpubLimits) -> std::result::Result<usize, String> {
    if bytes.get(4..8) == Some(b"ttcf") {
        return Err("font collections are unsupported".into());
    }
    let n = read_u16(bytes, 12)?;
    if n == 0 || n > limits.max_font_tables {
        return Err("font table count exceeds the limit".into());
    }
    if read_u32(bytes, 16)? > limits.max_decoded_font_bytes {
        return Err("font declares an oversized decoded payload".into());
    }
    Ok(n)
}
fn base_size(n: usize) -> std::result::Result<usize, String> {
    16usize
        .checked_mul(n)
        .and_then(|v| 12usize.checked_add(v))
        .ok_or_else(|| "font size arithmetic overflowed".into())
}
fn preflight_woff(
    bytes: &[u8],
    limits: &EpubLimits,
    is_cancelled: Option<&dyn Fn() -> bool>,
) -> std::result::Result<(), String> {
    let n = header(bytes, limits)?;
    if 20usize
        .checked_mul(n)
        .and_then(|directory| 44usize.checked_add(directory))
        .is_none_or(|v| v > bytes.len())
    {
        return Err("WOFF table directory is truncated".into());
    }
    let mut total = base_size(n)?;
    for i in 0..n {
        check_cancelled(is_cancelled).map_err(|error| error.to_string())?;
        let e = 44 + i * 20;
        let o = read_u32(bytes, e + 4)?;
        let c = read_u32(bytes, e + 8)?;
        let raw = read_u32(bytes, e + 12)?;
        if o.checked_add(c).is_none_or(|v| v > bytes.len()) {
            return Err("WOFF table data is outside the input".into());
        }
        total = total
            .checked_add(aligned(if c < raw { raw } else { c })?)
            .ok_or_else(|| "font size arithmetic overflowed".to_owned())?;
        if total > limits.max_decoded_font_bytes {
            return Err("WOFF table data exceeds the output limit".into());
        }
    }
    Ok(())
}
fn base128(b: &[u8], c: &mut usize) -> std::result::Result<usize, String> {
    let mut v = 0usize;
    for i in 0..5 {
        let x = *b
            .get(*c)
            .ok_or_else(|| "WOFF2 table directory is truncated".to_owned())?;
        *c += 1;
        if i == 0 && x == 128 {
            return Err("WOFF2 length is not canonical".into());
        }
        v = v
            .checked_mul(128)
            .and_then(|v| v.checked_add((x & 127) as usize))
            .ok_or_else(|| "font size arithmetic overflowed".to_owned())?;
        if x & 128 == 0 {
            return Ok(v);
        }
    }
    Err("WOFF2 length is too large".into())
}
fn preflight_woff2(
    b: &[u8],
    l: &EpubLimits,
    is_cancelled: Option<&dyn Fn() -> bool>,
) -> std::result::Result<(), String> {
    let n = header(b, l)?;
    let (mut c, mut out, mut encoded) = (48, base_size(n)?, 0usize);
    for _ in 0..n {
        check_cancelled(is_cancelled).map_err(|error| error.to_string())?;
        let flags = *b
            .get(c)
            .ok_or_else(|| "WOFF2 table directory is truncated".to_owned())?;
        c += 1;
        let tag = if flags & 63 == 63 {
            let t = b
                .get(c..c + 4)
                .ok_or_else(|| "WOFF2 table directory is truncated".to_owned())?;
            c += 4;
            Some(t)
        } else {
            None
        };
        let raw = base128(b, &mut c)?;
        let glyf = matches!(flags & 63, 10 | 11) || matches!(tag, Some(b"glyf") | Some(b"loca"));
        let transformed = if glyf {
            flags >> 6 == 0
        } else {
            flags >> 6 != 0
        };
        let enc = if transformed {
            base128(b, &mut c)?
        } else {
            raw
        };
        out = out
            .checked_add(aligned(raw)?)
            .ok_or_else(|| "font size arithmetic overflowed".to_owned())?;
        encoded = encoded
            .checked_add(enc)
            .ok_or_else(|| "font size arithmetic overflowed".to_owned())?;
        if out > l.max_decoded_font_bytes || encoded > l.max_decoded_font_bytes {
            return Err("WOFF2 table data exceeds the output limit".into());
        }
    }
    Ok(())
}
fn bounded(
    mut reader: impl Read,
    expected: usize,
    limit: usize,
    is_cancelled: Option<&dyn Fn() -> bool>,
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    if expected > limit {
        return Err("decoder output exceeds the limit".into());
    }
    let mut out = Vec::with_capacity(expected);
    let mut buffer = [0_u8; 64 * 1024];
    while out.len() <= expected {
        check_cancelled(is_cancelled)?;
        let remaining = expected.saturating_add(1).saturating_sub(out.len());
        let read = reader.by_ref().take(remaining as u64).read(&mut buffer)?;
        if read == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..read]);
    }
    if out.len() != expected {
        return Err("decoder output length does not match the table directory".into());
    }
    Ok(out)
}
fn sniff(b: &[u8]) -> std::result::Result<EpubFontFormat, String> {
    match b.get(..4) {
        Some([0, 1, 0, 0]) | Some(b"true") => Ok(EpubFontFormat::TrueType),
        Some(b"OTTO") => Ok(EpubFontFormat::OpenType),
        Some(b"wOFF") => Ok(EpubFontFormat::Woff),
        Some(b"wOF2") => Ok(EpubFontFormat::Woff2),
        _ => Err("unsupported font signature".into()),
    }
}
fn decode_font(
    b: &[u8],
    declared: Option<EpubFontFormat>,
    l: &EpubLimits,
    is_cancelled: Option<&dyn Fn() -> bool>,
) -> std::result::Result<(EpubFontFormat, Vec<u8>), String> {
    check_cancelled(is_cancelled).map_err(|error| error.to_string())?;
    if b.len() as u64 > l.max_font_bytes {
        return Err("encoded font exceeds the input limit".into());
    }
    let f = sniff(b)?;
    if declared.is_some_and(|d| d != f) {
        return Err("font signature does not match its format descriptor".into());
    }
    let out = match f {
        EpubFontFormat::TrueType | EpubFontFormat::OpenType => {
            let mut out = Vec::with_capacity(b.len());
            for chunk in b.chunks(64 * 1024) {
                check_cancelled(is_cancelled).map_err(|error| error.to_string())?;
                out.extend_from_slice(chunk);
            }
            out
        }
        EpubFontFormat::Woff => {
            preflight_woff(b, l, is_cancelled)?;
            wuff::decompress_woff1_with_custom_z(b, &mut |c, n| {
                bounded(
                    flate2::read::ZlibDecoder::new(c),
                    n,
                    l.max_decoded_font_bytes,
                    is_cancelled,
                )
            })
            .map_err(|_| "WOFF decoding failed".to_owned())?
        }
        EpubFontFormat::Woff2 => {
            preflight_woff2(b, l, is_cancelled)?;
            wuff::decompress_woff2_with_custom_brotli(b, &mut |c, n| {
                bounded(
                    brotli_decompressor::Decompressor::new(c, 4096),
                    n,
                    l.max_decoded_font_bytes,
                    is_cancelled,
                )
            })
            .map_err(|_| "WOFF2 decoding failed".to_owned())?
        }
    };
    check_cancelled(is_cancelled).map_err(|error| error.to_string())?;
    if out.len() > l.max_decoded_font_bytes {
        return Err("decoded font exceeds the output limit".into());
    }
    if !matches!(
        sniff(&out)?,
        EpubFontFormat::TrueType | EpubFontFormat::OpenType
    ) {
        return Err("decoder did not produce an sfnt font".into());
    }
    let tables = read_u16(&out, 4)?;
    if tables == 0 || tables > l.max_font_tables {
        return Err("font table count exceeds the limit".into());
    }
    Ok((f, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    const BOOK_A_TTF: &[u8] = include_bytes!("../../../shosai-app/tests/fonts/epub/book-a.ttf");
    const BOOK_A_OTF: &[u8] = include_bytes!("../../../shosai-app/tests/fonts/epub/book-a.otf");
    const BOOK_A_WOFF: &[u8] = include_bytes!("../../../shosai-app/tests/fonts/epub/book-a.woff");
    const BOOK_A_WOFF2: &[u8] = include_bytes!("../../../shosai-app/tests/fonts/epub/book-a.woff2");
    const BOOK_B_TTF: &[u8] = include_bytes!("../../../shosai-app/tests/fonts/epub/book-b.ttf");
    const INTER: &[u8] = include_bytes!("../../../../assets/fonts/InterVariable.ttf");
    const INTER_ITALIC: &[u8] =
        include_bytes!("../../../shosai-app/tests/fonts/InterVariable-Italic.ttf");
    const FAMILY: &str = "Shosai EPUB Fixture";

    fn font_book(
        css: &str,
        resources: &[(&str, &[u8])],
        limits: EpubLimits,
        chapters: usize,
    ) -> EpubFontBook {
        let styles = EpubStyles::parse([("OPS/styles/book.css", css)]);
        let chapters = (0..chapters)
            .map(|index| Chapter {
                index,
                title: None,
                path: format!("OPS/Text/chapter-{index}.xhtml"),
                content: r#"<html><head><link rel="stylesheet" href="../styles/book.css"/></head><body/></html>"#.into(),
            })
            .collect::<Vec<_>>();
        let resources = resources
            .iter()
            .map(|(path, bytes)| {
                (
                    CanonicalEpubPath::new(path).unwrap(),
                    StoredEpubResource {
                        media_type: "application/octet-stream".into(),
                        bytes: bytes.to_vec(),
                    },
                )
            })
            .collect();
        EpubFontBook::new(&chapters, &styles, &resources, &limits).unwrap()
    }

    fn one_face(path: &str, format: &str) -> String {
        format!(
            r#"@font-face {{ font-family: "{FAMILY}"; font-style: italic; font-weight: 700; src: url("../fonts/{path}") format("{format}"); }}"#
        )
    }

    #[test]
    fn dtd_chapter_discovers_external_font_face() {
        let css = one_face("book.ttf", "truetype");
        let styles = EpubStyles::parse([("OPS/styles/book.css", css.as_str())]);
        let chapters = vec![Chapter {
            index: 0,
            title: None,
            path: "OPS/Text/chapter.xhtml".into(),
            content: r#"<!DOCTYPE html [<!ENTITY label "Book">]><html><head><link rel="stylesheet" href="../styles/book.css"/></head><body>&label;</body></html>"#.into(),
        }];
        let resources = HashMap::from([(
            CanonicalEpubPath::new("OPS/fonts/book.ttf").unwrap(),
            StoredEpubResource {
                media_type: "font/ttf".into(),
                bytes: INTER.to_vec(),
            },
        )]);

        let book = EpubFontBook::new(&chapters, &styles, &resources, &EpubLimits::default())
            .expect("DTD chapter font discovery should succeed");
        assert!(book.contains_family_for_chapter("OPS/Text/chapter.xhtml", FAMILY));
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn native_layout_is_book_local_bounded_and_retains_unicode_hits() {
        use super::super::{
            EpubTextAlign, EpubTextDirection, EpubTextHighlight, EpubTextRequest, EpubTextRun,
        };

        let css = r#"@font-face { font-family: "Straße"; src: url("../fonts/book.ttf") format("truetype"); }"#;
        let a = font_book(
            css,
            &[("OPS/fonts/book.ttf", INTER)],
            EpubLimits::default(),
            1,
        );
        let b = font_book(
            css,
            &[("OPS/fonts/book.ttf", INTER_ITALIC)],
            EpubLimits::default(),
            1,
        );
        assert_ne!(a.native_text_id(), b.native_text_id());
        let a_name = a
            .native
            .lock()
            .unwrap()
            .matched_postscript_name("Straße", fontdb::Style::Normal)
            .unwrap()
            .to_owned();
        let b_name = b
            .native
            .lock()
            .unwrap()
            .matched_postscript_name("Straße", fontdb::Style::Normal)
            .unwrap()
            .to_owned();
        assert_ne!(a_name, b_name);
        let variants = font_book(
            r#"
                @font-face { font-family: "Book"; src: url("../fonts/regular.ttf"); }
                @font-face { font-family: "book"; font-style: italic; src: url("../fonts/italic.ttf"); }
            "#,
            &[
                ("OPS/fonts/regular.ttf", INTER),
                ("OPS/fonts/italic.ttf", INTER_ITALIC),
            ],
            EpubLimits::default(),
            1,
        );
        assert_eq!(
            variants
                .native
                .lock()
                .unwrap()
                .matched_postscript_name("BOOK", fontdb::Style::Italic),
            Some("InterVariableItalic")
        );
        let request = EpubTextRequest {
            runs: vec![
                EpubTextRun {
                    text: "AB\nAB ".into(),
                    family: Some("STRASSE".into()),
                    monospace: false,
                    font_size: 28.0,
                    bold: false,
                    italic: false,
                    foreground: [10, 20, 30, 255],
                    link: Some("chapter.xhtml#target".into()),
                },
                EpubTextRun {
                    text: "é שלום".into(),
                    family: None,
                    monospace: false,
                    font_size: 28.0,
                    bold: false,
                    italic: false,
                    foreground: [10, 20, 30, 255],
                    link: Some("chapter.xhtml#unicode".into()),
                },
            ],
            max_width: 300.0,
            line_height: 36.0,
            scale: 1.0,
            align: EpubTextAlign::Left,
            direction: EpubTextDirection::RightToLeft,
            highlights: vec![EpubTextHighlight {
                scalars: 6..7,
                color: [255, 255, 0, 255],
            }],
        };
        let first = a.layout_text(&request).unwrap();
        assert!(
            first
                .lines
                .iter()
                .any(|line| line.rgba.iter().any(|v| *v != 0))
        );
        assert!(first.links.iter().any(|hit| hit.scalars.start == 0));
        assert!(first.links.iter().all(|hit| hit.scalars.end <= 12));
        assert_eq!(first.lines.len(), 2);
        assert_eq!(first.lines[0].scalars, 0..3);
        assert_eq!(first.lines[1].scalars, 3..12);
        assert!(
            first
                .lines
                .iter()
                .all(|line| line.rgba.iter().any(|value| *value != 0)),
            "every visual line must rasterize into its own local bitmap"
        );
        assert!(first.lines.iter().any(|line| {
            line.rgba
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 200 && pixel[1] > 200 && pixel[3] > 0)
        }));
        drop(a);
        let second = b.layout_text(&request).unwrap();
        assert!(second.lines[0].rgba.iter().any(|value| *value != 0));
        assert_ne!(
            first.lines[0].rgba, second.lines[0].rgba,
            "same CSS alias in two books must retain each book's distinct glyph raster"
        );

        let mut separator = request.clone();
        separator.runs = vec![EpubTextRun {
            text: "A\u{2029}B".into(),
            family: Some("STRASSE".into()),
            monospace: false,
            font_size: 28.0,
            bold: false,
            italic: false,
            foreground: [10, 20, 30, 255],
            link: None,
        }];
        separator.direction = EpubTextDirection::LeftToRight;
        separator.highlights.clear();
        let separated = b.layout_text(&separator).unwrap();
        assert_eq!(
            separated
                .lines
                .iter()
                .map(|line| line.scalars.clone())
                .collect::<Vec<_>>(),
            vec![0..2, 2..3]
        );
        assert!(
            separated
                .lines
                .iter()
                .all(|line| line.rgba.iter().any(|value| *value != 0))
        );

        let mut invalid = request.clone();
        invalid.scale = f32::NAN;
        assert!(b.layout_text(&invalid).is_err());
        invalid.scale = 1.0;
        invalid.max_width = 1_000_000.0;
        invalid.line_height = 1_000_000.0;
        assert!(b.layout_text(&invalid).is_err());
    }

    #[test]
    fn native_raster_cache_does_not_grow_across_requests() {
        use super::super::{EpubTextAlign, EpubTextDirection, EpubTextRequest, EpubTextRun};

        let book = font_book(
            r#"@font-face { font-family: "Book"; src: url("../fonts/book.ttf"); }"#,
            &[("OPS/fonts/book.ttf", INTER)],
            EpubLimits::default(),
            1,
        );
        for size in 10..40 {
            book.layout_text(&EpubTextRequest {
                runs: vec![EpubTextRun {
                    text: "Retained glyph bitmap".into(),
                    family: Some("Book".into()),
                    monospace: false,
                    font_size: size as f32,
                    bold: false,
                    italic: false,
                    foreground: [0, 0, 0, 255],
                    link: None,
                }],
                max_width: 500.0,
                line_height: 48.0,
                scale: 1.0,
                align: EpubTextAlign::Left,
                direction: EpubTextDirection::LeftToRight,
                highlights: Vec::new(),
            })
            .unwrap();
        }
        assert_eq!(
            book.native.lock().unwrap().retained_raster_image_count(),
            0,
            "per-book state must not retain glyph images across raster requests"
        );
    }

    #[test]
    fn native_layout_bounds_input_and_forces_each_paragraph_direction() {
        use super::super::native_text::EPUB_TEXT_MAX_ENDPOINTS;
        use super::super::{
            EPUB_TEXT_MAX_SCALARS, EpubTextAlign, EpubTextDirection, EpubTextRequest, EpubTextRun,
        };

        let book = font_book(
            r#"@font-face { font-family: "Book"; src: url("../fonts/book.ttf"); }"#,
            &[("OPS/fonts/book.ttf", INTER)],
            EpubLimits::default(),
            1,
        );
        let request = |text: String| EpubTextRequest {
            runs: vec![EpubTextRun {
                text,
                family: Some("Book".into()),
                monospace: false,
                font_size: 20.0,
                bold: false,
                italic: false,
                foreground: [0, 0, 0, 255],
                link: Some("target".into()),
            }],
            max_width: 400.0,
            line_height: 28.0,
            scale: 1.0,
            align: EpubTextAlign::Left,
            direction: EpubTextDirection::RightToLeft,
            highlights: Vec::new(),
        };
        let layout = book.layout_text(&request("אבג\nABC".into())).unwrap();
        assert_eq!(layout.lines.len(), 2);
        assert!(layout.lines.iter().all(|line| line.rtl));
        assert_eq!(layout.lines[0].scalars, 0..4);
        assert_eq!(layout.lines[1].scalars, 4..7);
        assert!(layout.links.iter().all(|hit| hit.scalars.end <= 7));

        assert!(
            book.measure_text(&request("x".repeat(EPUB_TEXT_MAX_SCALARS + 1)))
                .is_err()
        );
        assert!(
            book.measure_text(&request("\n".repeat(4 * 1024 + 1)))
                .is_err(),
            "paragraph splitting must be bounded before allocating child requests"
        );
        assert!(
            book.measure_text(&request("x".repeat(EPUB_TEXT_MAX_ENDPOINTS / 2 + 1)))
                .is_err(),
            "retained endpoint geometry must have an independent hard ceiling"
        );

        let rtl_cluster = book.measure_text(&request("لا".into())).unwrap();
        let mut endpoints = rtl_cluster.endpoints;
        endpoints.sort_by(|left, right| left.rect.x.total_cmp(&right.rect.x));
        assert_eq!(endpoints.len(), 4);
        assert_eq!(endpoints[0].scalar_start, 1);
        assert_eq!(endpoints[2].scalar_start, 0);
    }

    #[test]
    fn native_measurement_rechecks_cancellation_after_waiting_for_renderer() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        use super::super::{EpubTextAlign, EpubTextDirection, EpubTextRequest, EpubTextRun};

        let book = Arc::new(font_book(
            r#"@font-face { font-family: "Book"; src: url("../fonts/book.ttf"); }"#,
            &[("OPS/fonts/book.ttf", INTER)],
            EpubLimits::default(),
            1,
        ));
        let request = EpubTextRequest {
            runs: vec![EpubTextRun {
                text: "measurement must not start after cancellation".into(),
                family: Some("Book".into()),
                monospace: false,
                font_size: 20.0,
                bold: false,
                italic: false,
                foreground: [0, 0, 0, 255],
                link: None,
            }],
            max_width: 400.0,
            line_height: 28.0,
            scale: 1.0,
            align: EpubTextAlign::Left,
            direction: EpubTextDirection::LeftToRight,
            highlights: Vec::new(),
        };
        let renderer = book.native.lock().unwrap();
        let checks = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let renderer_entries = Arc::new(AtomicU64::new(0));
        book.observe_renderer_entries(Arc::clone(&renderer_entries));
        let (waiting_tx, waiting_rx) = std::sync::mpsc::sync_channel(1);
        let worker_book = Arc::clone(&book);
        let worker_checks = Arc::clone(&checks);
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_request = request.clone();
        let worker = std::thread::spawn(move || {
            worker_book.measure_text_cancellable(&worker_request, &|| {
                let is_cancelled = worker_cancelled.load(Ordering::SeqCst);
                if worker_checks.fetch_add(1, Ordering::SeqCst) == 1 {
                    waiting_tx.send(()).unwrap();
                }
                is_cancelled
            })
        });
        waiting_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("measurement must reach the renderer lock");
        cancelled.store(true, Ordering::SeqCst);
        drop(renderer);

        let error = worker.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(checks.load(Ordering::SeqCst) >= 3);
        assert_eq!(renderer_entries.load(AtomicOrdering::SeqCst), 0);

        cancelled.store(false, Ordering::SeqCst);
        book.measure_text_cancellable(&request, &|| cancelled.load(Ordering::SeqCst))
            .unwrap();
        assert_eq!(renderer_entries.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn static_regular_face_synthesizes_requested_bold() {
        use super::super::{EpubTextAlign, EpubTextDirection, EpubTextRequest, EpubTextRun};

        let book = font_book(
            r#"@font-face { font-family: "Book"; font-weight: 400; src: url("../fonts/book.ttf"); }"#,
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        let request = |bold| EpubTextRequest {
            runs: vec![EpubTextRun {
                text: "Static face".into(),
                family: Some("Book".into()),
                monospace: false,
                font_size: 32.0,
                bold,
                italic: false,
                foreground: [0, 0, 0, 255],
                link: None,
            }],
            max_width: 300.0,
            line_height: 40.0,
            scale: 1.0,
            align: EpubTextAlign::Left,
            direction: EpubTextDirection::LeftToRight,
            highlights: Vec::new(),
        };
        let regular = book.layout_text(&request(false)).unwrap();
        let bold = book.layout_text(&request(true)).unwrap();
        let opacity = |layout: &super::super::EpubTextLayout| {
            layout.lines[0]
                .rgba
                .chunks_exact(4)
                .map(|pixel| u64::from(pixel[3]))
                .sum::<u64>()
        };
        assert!(opacity(&bold) > opacity(&regular));
    }

    #[test]
    fn ttf_otf_woff_and_woff2_are_admitted_into_book_local_databases() {
        use super::super::{EpubTextAlign, EpubTextDirection, EpubTextRequest, EpubTextRun};

        for (path, format, bytes, expected) in [
            (
                "book-a.ttf",
                "truetype",
                BOOK_A_TTF,
                EpubFontFormat::TrueType,
            ),
            (
                "book-a.otf",
                "opentype",
                BOOK_A_OTF,
                EpubFontFormat::OpenType,
            ),
            ("book-a.woff", "woff", BOOK_A_WOFF, EpubFontFormat::Woff),
            ("book-a.woff2", "woff2", BOOK_A_WOFF2, EpubFontFormat::Woff2),
        ] {
            let resource_path = format!("OPS/fonts/{path}");
            let book = font_book(
                &one_face(path, format),
                &[(resource_path.as_str(), bytes)],
                EpubLimits::default(),
                1,
            );
            assert_eq!(book.len(), 1, "{path}: {:?}", book.rejected_faces());
            assert_eq!(book.registered_face_count(), 1, "{path}");
            assert_eq!(book.faces()[0].format, expected, "{path}");
            assert_eq!(book.faces()[0].style, EpubFontStyle::Italic, "{path}");
            assert_eq!(book.faces()[0].weight.min(), 700.0, "{path}");
            assert_eq!(book.faces()[0].weight.max(), 700.0, "{path}");
            assert!(book.with_face_data(0, |data, _| !data.is_empty()).unwrap());
            let layout = book
                .layout_text(&EpubTextRequest {
                    runs: vec![EpubTextRun {
                        text: "AB".into(),
                        family: Some(FAMILY.into()),
                        monospace: false,
                        font_size: 28.0,
                        bold: true,
                        italic: true,
                        foreground: [20, 30, 40, 255],
                        link: None,
                    }],
                    max_width: 120.0,
                    line_height: 36.0,
                    scale: 1.0,
                    align: EpubTextAlign::Left,
                    direction: EpubTextDirection::LeftToRight,
                    highlights: Vec::new(),
                })
                .unwrap_or_else(|error| panic!("{path} failed native rendering: {error:#}"));
            assert!(
                layout
                    .lines
                    .iter()
                    .any(|line| line.rgba.iter().any(|value| *value != 0)),
                "{path} produced no embedded-font pixels"
            );
        }
    }

    #[test]
    fn source_fallback_rejects_local_remote_missing_and_unsupported_sources() {
        let css = format!(
            r#"@font-face {{ font-family: "{FAMILY}"; src:
                local("System Font"),
                url("https://example.invalid/remote.ttf") format("truetype"),
                url("../fonts/missing.ttf") format("truetype"),
                url("../fonts/unsupported.ttf") format("svg"),
                url("../fonts/technology.ttf") format("truetype") tech(variations),
                url("../fonts/book-a.ttf") format("truetype"); }}"#
        );
        let book = font_book(
            &css,
            &[("OPS/fonts/book-a.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );

        assert_eq!(book.len(), 1);
        assert_eq!(book.faces()[0].attempts.len(), 6);
        assert!(matches!(
            book.faces()[0].attempts.last(),
            Some(EpubFontAttempt::Loaded { path, .. }) if path.as_str() == "OPS/fonts/book-a.ttf"
        ));
    }

    #[test]
    fn later_src_descriptor_replaces_earlier_sources() {
        let css = format!(
            r#"@font-face {{
                font-family: "{FAMILY}";
                src: url("../fonts/old.ttf") format("truetype");
                src: url("../fonts/current.ttf") format("truetype");
            }}"#
        );
        let book = font_book(
            &css,
            &[
                ("OPS/fonts/old.ttf", BOOK_A_TTF),
                ("OPS/fonts/current.ttf", BOOK_B_TTF),
            ],
            EpubLimits::default(),
            1,
        );

        assert_eq!(book.len(), 1);
        assert_eq!(book.faces()[0].path.as_str(), "OPS/fonts/current.ttf");
        assert_eq!(book.faces()[0].attempts.len(), 1);
    }

    #[test]
    fn matching_member_of_mixed_media_list_admits_font_face() {
        let css = format!(
            r#"@media screen, (min-width: 1px) {{ {} }}"#,
            one_face("book.ttf", "truetype")
        );
        let book = font_book(
            &css,
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );

        assert_eq!(book.len(), 1, "{:?}", book.rejected_faces());
    }

    #[test]
    fn family_aliases_preserve_decoded_names_and_use_unicode_case_folding() {
        let numeric_alias = "123 Font";
        let numeric_css = format!(
            r#"@font-face {{ font-family: "{numeric_alias}"; src: url("../fonts/book.ttf"); }}"#
        );
        let numeric = font_book(
            &numeric_css,
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        assert_eq!(numeric.faces()[0].family, numeric_alias);

        let unicode_css = r#"@font-face { font-family: "Straße"; src: url("../fonts/book.ttf"); }
            p { font-family: "STRASSE"; }"#;
        let styles = EpubStyles::parse([("OPS/styles/book.css", unicode_css)]);
        let chapter = Chapter {
            index: 0,
            title: None,
            path: "OPS/Text/chapter.xhtml".into(),
            content: r#"<html><head><link rel="stylesheet" href="../styles/book.css"/></head><body><p>Folded</p></body></html>"#.into(),
        };
        let resources = HashMap::from([(
            CanonicalEpubPath::new("OPS/fonts/book.ttf").unwrap(),
            StoredEpubResource {
                media_type: "font/ttf".into(),
                bytes: BOOK_A_TTF.to_vec(),
            },
        )]);
        let limits = EpubLimits::default();
        let fonts = EpubFontBook::new(std::slice::from_ref(&chapter), &styles, &resources, &limits)
            .unwrap();
        let nodes = super::super::render::parse_chapter_xhtml_at_path_with_limits(
            &chapter.content,
            &chapter.path,
            &styles,
            &fonts,
            &limits,
        )
        .unwrap();
        let super::super::render::ContentNode::Paragraph(spans, _) = &nodes[0] else {
            panic!("fixture paragraph must be retained");
        };
        assert_eq!(spans[0].font_family.as_deref(), Some("STRASSE"));
    }

    #[test]
    fn family_metadata_and_diagnostic_labels_are_bounded_before_amplification() {
        let parse = |css: &str, paragraph: &str, limits: EpubLimits| {
            let styles = EpubStyles::parse([("OPS/styles/book.css", css)]);
            let chapter = Chapter {
                index: 0,
                title: None,
                path: "OPS/Text/chapter.xhtml".into(),
                content: format!(
                    r#"<html><head><link rel="stylesheet" href="../styles/book.css"/></head><body>{paragraph}</body></html>"#
                ),
            };
            let fonts = EpubFontBook::new(&[], &styles, &HashMap::new(), &limits).unwrap();
            super::super::render::parse_chapter_xhtml_at_path_with_limits(
                &chapter.content,
                &chapter.path,
                &styles,
                &fonts,
                &limits,
            )
        };

        let long_family = "x".repeat(1_024);
        let css = format!(r#"p {{ font-family: "{long_family}"; }}"#);
        let limits = EpubLimits::default();
        let error = parse(&css, "<p>Bounded</p>", limits)
            .expect_err("oversized family metadata must stop before style-tree inheritance");
        assert!(error.to_string().contains("font family"));

        let inline = format!(r#"<p style="font-family: '{long_family}'">Inline</p>"#);
        assert!(parse("", &inline, limits).is_err());
        assert!(
            parse(
                r#"p { font-family: "One", "Two"; }"#,
                "<p>Count</p>",
                EpubLimits {
                    max_css_font_families_per_declaration: 1,
                    ..limits
                },
            )
            .is_err()
        );
        assert!(
            parse(
                r#"p { font-family: "Four", "Bytes"; }"#,
                "<p>Bytes</p>",
                EpubLimits {
                    max_css_font_family_bytes_per_declaration: 8,
                    ..limits
                },
            )
            .is_err()
        );

        let long_local = "y".repeat(1_024);
        let face_css =
            format!(r#"@font-face {{ font-family: "Bounded"; src: local("{long_local}"); }}"#);
        let rejected = font_book(&face_css, &[], limits, 1);
        let EpubFontAttempt::Rejected { source, .. } = &rejected.rejected_faces()[0].attempts[0]
        else {
            panic!("local source must be rejected");
        };
        assert!(source.len() <= 256);

        let long_url = "z".repeat(4_096);
        let url_css =
            format!(r#"@font-face {{ font-family: "Bounded"; src: url("{long_url}"); }}"#);
        let rejected = font_book(&url_css, &[], limits, 1);
        assert!(matches!(
            rejected.rejected_faces()[0].attempts.as_slice(),
            [EpubFontAttempt::Rejected { source, reason }]
                if source.len() <= 256
                    && reason == "font source reference exceeds the byte limit"
        ));
    }

    #[test]
    fn weight_ranges_are_retained_and_invalid_weights_use_the_initial_value() {
        let ranged = font_book(
            &format!(
                r#"@font-face {{ font-family: "{FAMILY}"; font-weight: 300.5 700.25; src: url("../fonts/book.ttf"); }}"#
            ),
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged.faces()[0].weight.min(), 300.5);
        assert_eq!(ranged.faces()[0].weight.max(), 700.25);

        let invalid = font_book(
            &format!(
                r#"@font-face {{ font-family: "{FAMILY}"; font-weight: 2000; src: url("../fonts/book.ttf"); }}"#
            ),
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid.faces()[0].weight.min(), 400.0);
        assert_eq!(invalid.faces()[0].weight.max(), 400.0);

        let unsupported_style_range = font_book(
            &format!(
                r#"@font-face {{ font-family: "{FAMILY}"; font-style: oblique 10deg 20deg; src: url("../fonts/book.ttf"); }}"#
            ),
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        assert_eq!(unsupported_style_range.len(), 1);
        assert_eq!(
            unsupported_style_range.faces()[0].style,
            EpubFontStyle::Oblique,
            "M2a retains the supported style category without claiming angle-range metadata"
        );
    }

    #[test]
    fn apple_true_sfnt_signature_is_admitted_as_truetype() {
        let mut apple = BOOK_A_TTF.to_vec();
        apple[..4].copy_from_slice(b"true");
        let book = font_book(
            &one_face("apple.ttf", "truetype"),
            &[("OPS/fonts/apple.ttf", apple.as_slice())],
            EpubLimits::default(),
            1,
        );

        assert_eq!(book.len(), 1, "{:?}", book.rejected_faces());
        assert_eq!(book.faces()[0].format, EpubFontFormat::TrueType);
    }

    #[test]
    fn zero_font_limits_disable_admission_without_rejecting_font_free_books() {
        let disabled = EpubLimits {
            max_font_faces_per_book: 0,
            max_font_face_descriptors_per_book: 0,
            max_font_sources_per_face: 0,
            max_decoded_font_bytes: 0,
            max_total_decoded_font_bytes: 0,
            max_font_tables: 0,
            ..EpubLimits::default()
        };
        let empty = font_book("", &[], disabled, 1);
        assert!(empty.is_empty());

        let rejected = font_book(
            &one_face("book.ttf", "truetype"),
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            disabled,
            1,
        );
        assert!(rejected.is_empty());
        assert_eq!(rejected.rejected_faces().len(), 1);
    }

    #[test]
    fn rejected_face_does_not_consume_admitted_face_budget() {
        let css = format!(
            r#"@font-face {{ font-family: "Missing"; src: url("../fonts/missing.ttf"); }}
                {}"#,
            one_face("book.ttf", "truetype")
        );
        let book = font_book(
            &css,
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits {
                max_font_faces_per_book: 1,
                ..EpubLimits::default()
            },
            1,
        );

        assert_eq!(book.len(), 1, "{:?}", book.rejected_faces());
        assert_eq!(book.faces()[0].family, FAMILY);
    }

    #[test]
    fn descriptor_source_and_admitted_face_work_are_independently_bounded() {
        let descriptors = (0..4)
            .map(|index| {
                format!(
                    r#"@font-face {{ font-family: "Missing {index}"; src: url("missing-{index}.ttf"); }}"#
                )
            })
            .collect::<String>();
        let bounded_descriptors = font_book(
            &descriptors,
            &[],
            EpubLimits {
                max_font_face_descriptors_per_book: 1,
                ..EpubLimits::default()
            },
            1,
        );
        assert_eq!(bounded_descriptors.rejected_faces().len(), 2);
        assert!(bounded_descriptors.rejected_faces().iter().any(|face| {
            matches!(
                face.attempts.as_slice(),
                [EpubFontAttempt::Rejected { reason, .. }]
                    if reason == "per-book font descriptor inspection limit is exhausted"
            )
        }));

        let bounded_sources = font_book(
            &format!(
                r#"@font-face {{ font-family: "{FAMILY}"; src: url("one.ttf"), url("two.ttf"); }}"#
            ),
            &[],
            EpubLimits {
                max_font_sources_per_face: 1,
                ..EpubLimits::default()
            },
            1,
        );
        assert!(matches!(
            bounded_sources.rejected_faces()[0].attempts.as_slice(),
            [EpubFontAttempt::Rejected { reason, .. }]
                if reason == "font source inspection limit is exhausted"
        ));

        let two_faces = format!(
            r#"{}
                @font-face {{ font-family: "Second"; src: url("../fonts/book-b.ttf"); }}"#,
            one_face("book-a.ttf", "truetype")
        );
        let bounded_faces = font_book(
            &two_faces,
            &[
                ("OPS/fonts/book-a.ttf", BOOK_A_TTF),
                ("OPS/fonts/book-b.ttf", BOOK_B_TTF),
            ],
            EpubLimits {
                max_font_faces_per_book: 1,
                ..EpubLimits::default()
            },
            1,
        );
        assert_eq!(bounded_faces.len(), 1);
        assert!(bounded_faces.rejected_faces().iter().any(|face| {
            matches!(
                face.attempts.as_slice(),
                [EpubFontAttempt::Rejected { reason, .. }]
                    if reason == "per-book admitted font face limit is exhausted"
            )
        }));
    }

    #[test]
    fn stylesheet_complexity_is_validated_before_font_decoding() {
        let css = one_face("book.ttf", "truetype");
        let styles = EpubStyles::parse([("OPS/styles/book.css", css.as_str())]);
        let chapter = Chapter {
            index: 0,
            title: None,
            path: "OPS/Text/chapter.xhtml".into(),
            content: r#"<html><head><link rel="stylesheet" href="../styles/book.css"/></head><body/></html>"#.into(),
        };
        let resources = HashMap::from([(
            CanonicalEpubPath::new("OPS/fonts/book.ttf").unwrap(),
            StoredEpubResource {
                media_type: "font/ttf".into(),
                bytes: BOOK_A_TTF.to_vec(),
            },
        )]);

        let error = EpubFontBook::new(
            &[chapter],
            &styles,
            &resources,
            &EpubLimits {
                max_css_rules_per_document: 0,
                ..EpubLimits::default()
            },
        )
        .expect_err("font admission must share the document CSS rule boundary");
        assert!(error.to_string().contains("CSS rule limit"));
    }

    #[test]
    fn admitted_aliases_remain_scoped_to_their_chapter_stylesheets() {
        let declaring_css = one_face("book.ttf", "truetype");
        let other_css = format!(r#"p {{ font-family: "{FAMILY}"; }}"#);
        let styles = EpubStyles::parse([
            ("OPS/styles/declaring.css", declaring_css.as_str()),
            ("OPS/styles/other.css", other_css.as_str()),
        ]);
        let chapters = [
            Chapter {
                index: 0,
                title: None,
                path: "OPS/Text/declaring.xhtml".into(),
                content: r#"<html><head><link rel="stylesheet" href="../styles/declaring.css"/></head><body><p>Declared</p></body></html>"#.into(),
            },
            Chapter {
                index: 1,
                title: None,
                path: "OPS/Text/other.xhtml".into(),
                content: r#"<html><head><link rel="stylesheet" href="../styles/other.css"/></head><body><p>Fallback</p></body></html>"#.into(),
            },
        ];
        let resources = HashMap::from([(
            CanonicalEpubPath::new("OPS/fonts/book.ttf").unwrap(),
            StoredEpubResource {
                media_type: "font/ttf".into(),
                bytes: BOOK_A_TTF.to_vec(),
            },
        )]);
        let limits = EpubLimits::default();
        let fonts = EpubFontBook::new(&chapters, &styles, &resources, &limits).unwrap();
        let nodes = super::super::render::parse_chapter_xhtml_at_path_with_limits(
            &chapters[1].content,
            &chapters[1].path,
            &styles,
            &fonts,
            &limits,
        )
        .unwrap();
        let super::super::render::ContentNode::Paragraph(spans, _) = &nodes[0] else {
            panic!("fixture paragraph must be retained");
        };

        assert_eq!(spans[0].font_family, None);
    }

    #[test]
    fn malformed_mismatched_format_and_decoded_limits_fail_closed() {
        let mismatch = font_book(
            &one_face("book-a.ttf", "opentype"),
            &[("OPS/fonts/book-a.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        assert!(mismatch.is_empty());
        assert!(mismatch.rejected_faces()[0].attempts.iter().any(|attempt| {
            matches!(attempt, EpubFontAttempt::Rejected { reason, .. } if reason.contains("signature does not match"))
        }));

        let alias = font_book(
            &one_face("book-a.ttf", "truetype"),
            &[("OPS/fonts/book-a.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            1,
        );
        assert_eq!(
            alias.len(),
            1,
            "@font-face family is an author-defined alias"
        );

        let bounded = font_book(
            &one_face("book-a.woff2", "woff2"),
            &[("OPS/fonts/book-a.woff2", BOOK_A_WOFF2)],
            EpubLimits {
                max_decoded_font_bytes: 1,
                ..EpubLimits::default()
            },
            1,
        );
        assert!(bounded.is_empty());
    }

    #[test]
    fn encoded_table_and_corrupt_font_limits_report_rejection_reasons() {
        for (limits, bytes, expected) in [
            (
                EpubLimits {
                    max_font_bytes: (BOOK_A_TTF.len() - 1) as u64,
                    ..EpubLimits::default()
                },
                BOOK_A_TTF,
                "encoded font exceeds the input limit",
            ),
            (
                EpubLimits {
                    max_font_tables: 1,
                    ..EpubLimits::default()
                },
                BOOK_A_TTF,
                "font table count exceeds the limit",
            ),
            (
                EpubLimits::default(),
                b"corrupt font",
                "unsupported font signature",
            ),
        ] {
            let book = font_book(
                &one_face("book.ttf", "truetype"),
                &[("OPS/fonts/book.ttf", bytes)],
                limits,
                1,
            );
            assert!(book.is_empty());
            assert!(matches!(
                book.rejected_faces()[0].attempts.as_slice(),
                [EpubFontAttempt::Rejected { reason, .. }] if reason == expected
            ));
        }
    }

    #[test]
    fn faces_are_deduplicated_bounded_and_isolated_per_book() {
        let css = one_face("book.ttf", "truetype");
        let first = font_book(
            &css,
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits::default(),
            2,
        );
        let second = font_book(
            &css,
            &[("OPS/fonts/book.ttf", BOOK_B_TTF)],
            EpubLimits::default(),
            2,
        );
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first.registered_face_count(), 1);
        assert_eq!(second.registered_face_count(), 1);
        assert!(first.contains_family(FAMILY));
        assert!(second.contains_family(FAMILY));
        let first_hash = first
            .with_face_data(0, |bytes, _| sha2::Sha256::digest(bytes))
            .unwrap();
        let second_hash = second
            .with_face_data(0, |bytes, _| sha2::Sha256::digest(bytes))
            .unwrap();
        assert_ne!(first_hash, second_hash);

        let first_source = match &first
            .database
            .face(first.registered_ids[0])
            .expect("registered face must remain in its book database")
            .source
        {
            Source::Binary(bytes) => Arc::downgrade(bytes),
            _ => panic!("book fonts must use in-memory binary sources"),
        };
        drop(first);
        assert!(
            first_source.upgrade().is_none(),
            "dropping one book must release its decoded font source"
        );
        assert_eq!(second.registered_face_count(), 1);
        assert_eq!(
            second
                .with_face_data(0, |bytes, _| sha2::Sha256::digest(bytes))
                .unwrap(),
            second_hash,
            "dropping one book must not disturb another book's same-family face"
        );

        let exhausted = font_book(
            &css,
            &[("OPS/fonts/book.ttf", BOOK_A_TTF)],
            EpubLimits {
                max_total_decoded_font_bytes: 1,
                ..EpubLimits::default()
            },
            1,
        );
        assert!(exhausted.is_empty());
        assert!(exhausted.rejected_faces()[0].attempts.iter().any(|attempt| {
            matches!(attempt, EpubFontAttempt::Rejected { reason, .. } if reason.contains("budget"))
        }));
    }

    #[test]
    fn native_spans_select_the_first_admitted_family_then_fall_back() {
        let css = format!(
            r#"{}
                p {{ font-family: "Missing Family", "{FAMILY}", serif; }}"#,
            one_face("book.ttf", "truetype")
        );
        let styles = EpubStyles::parse([("OPS/styles/book.css", css.as_str())]);
        let chapter = Chapter {
            index: 0,
            title: None,
            path: "OPS/Text/chapter.xhtml".into(),
            content: r#"<html><head><link rel="stylesheet" href="../styles/book.css"/></head><body><p>Embedded</p></body></html>"#.into(),
        };
        let resources = HashMap::from([(
            CanonicalEpubPath::new("OPS/fonts/book.ttf").unwrap(),
            StoredEpubResource {
                media_type: "font/ttf".into(),
                bytes: BOOK_A_TTF.to_vec(),
            },
        )]);
        let limits = EpubLimits::default();
        let fonts = EpubFontBook::new(std::slice::from_ref(&chapter), &styles, &resources, &limits)
            .unwrap();
        let nodes = super::super::render::parse_chapter_xhtml_at_path_with_limits(
            &chapter.content,
            &chapter.path,
            &styles,
            &fonts,
            &limits,
        )
        .unwrap();
        let super::super::render::ContentNode::Paragraph(spans, _) = &nodes[0] else {
            panic!("fixture paragraph must be retained");
        };
        assert_eq!(spans[0].font_family.as_deref(), Some(FAMILY));

        let no_fonts = EpubFontBook::new(
            std::slice::from_ref(&chapter),
            &styles,
            &HashMap::new(),
            &limits,
        )
        .unwrap();
        let fallback = super::super::render::parse_chapter_xhtml_at_path_with_limits(
            &chapter.content,
            &chapter.path,
            &styles,
            &no_fonts,
            &limits,
        )
        .unwrap();
        let super::super::render::ContentNode::Paragraph(spans, _) = &fallback[0] else {
            panic!("fixture paragraph must be retained");
        };
        assert_eq!(spans[0].font_family, None);
    }

    #[test]
    fn admitted_family_keeps_requested_bold_italic_for_native_synthesis() {
        let css = format!(
            r#"@font-face {{ font-family: "{FAMILY}"; font-style: normal; font-weight: 400; src: url("../fonts/book.ttf") format("truetype"); }}
                p {{ font-family: "{FAMILY}"; font-style: italic; font-weight: bold; }}"#
        );
        let styles = EpubStyles::parse([("OPS/styles/book.css", css.as_str())]);
        let chapter = Chapter {
            index: 0,
            title: None,
            path: "OPS/Text/chapter.xhtml".into(),
            content: r#"<html><head><link rel="stylesheet" href="../styles/book.css"/></head><body><p>Synthesized</p></body></html>"#.into(),
        };
        let resources = HashMap::from([(
            CanonicalEpubPath::new("OPS/fonts/book.ttf").unwrap(),
            StoredEpubResource {
                media_type: "font/ttf".into(),
                bytes: BOOK_A_TTF.to_vec(),
            },
        )]);
        let limits = EpubLimits::default();
        let fonts = EpubFontBook::new(std::slice::from_ref(&chapter), &styles, &resources, &limits)
            .unwrap();
        let nodes = super::super::render::parse_chapter_xhtml_at_path_with_limits(
            &chapter.content,
            &chapter.path,
            &styles,
            &fonts,
            &limits,
        )
        .unwrap();
        let super::super::render::ContentNode::Paragraph(spans, _) = &nodes[0] else {
            panic!("fixture paragraph must be retained");
        };
        assert_eq!(spans[0].font_family.as_deref(), Some(FAMILY));
        assert!(spans[0].bold);
        assert!(spans[0].italic);
    }
}
