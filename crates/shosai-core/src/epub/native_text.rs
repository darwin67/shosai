//! Renderer-neutral, book-local EPUB text shaping and rasterization.

use std::{collections::HashMap, ops::Range};

use anyhow::{Context, Result, bail};
use cosmic_text::{
    Align, Attrs, BidiParagraphs, Buffer, CacheKeyFlags, Color, FontSystem, Metrics, Shaping,
    SwashCache, SwashContent, Wrap,
    fontdb::{Database, Language, Stretch, Style, Weight},
};
use unicode_casefold::UnicodeCaseFold;
use unicode_segmentation::UnicodeSegmentation;

use super::{EpubFontBook, EpubFontFace, EpubFontStyle};
use crate::application::ResourceLimitError;

/// Hard ceiling for the sum of returned line bitmap pixels (64 MiB RGBA).
pub const EPUB_TEXT_MAX_PIXELS: usize = 16 * 1024 * 1024;
/// Hard ceiling for Unicode scalars shaped by one native request.
pub const EPUB_TEXT_MAX_SCALARS: usize = 64 * 1024;
/// Hard ceiling for retained half-grapheme endpoint geometry.
pub const EPUB_TEXT_MAX_ENDPOINTS: usize = 64 * 1024;
const EPUB_TEXT_MAX_PARAGRAPHS: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpubTextAlign {
    Left,
    Center,
    Right,
    Justified,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpubTextDirection {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EpubTextRun {
    pub text: String,
    pub family: Option<String>,
    pub monospace: bool,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub foreground: [u8; 4],
    pub link: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpubTextHighlight {
    pub scalars: Range<usize>,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub struct EpubTextRequest {
    pub runs: Vec<EpubTextRun>,
    pub max_width: f32,
    pub line_height: f32,
    pub scale: f32,
    pub align: EpubTextAlign,
    pub direction: EpubTextDirection,
    pub highlights: Vec<EpubTextHighlight>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EpubTextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
#[derive(Clone, Debug)]
pub struct EpubTextHit {
    pub rect: EpubTextRect,
    pub scalars: Range<usize>,
    pub link: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EpubTextEndpoint {
    pub rect: EpubTextRect,
    /// Shaper-derived caret coordinate; unlike the hit-zone center this is an
    /// actual cluster edge and preserves proportional widths and bidi.
    pub caret_x: f32,
    pub scalar: usize,
    pub scalar_start: usize,
    pub scalar_end: usize,
    /// Index into `EpubTextLayout::lines`, assigned by the shaper.
    pub visual_line: usize,
}
#[derive(Clone, Debug)]
pub struct EpubTextLine {
    pub top: f32,
    pub width: f32,
    pub rtl: bool,
    pub scalars: Range<usize>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct EpubTextLayout {
    pub width: f32,
    pub height: f32,
    pub lines: Vec<EpubTextLine>,
    pub links: Vec<EpubTextHit>,
    /// Half-grapheme hit zones derived from the exact shaped glyphs that paint
    /// this layout. Consumers can retain these and hit-test without re-entering
    /// the renderer.
    pub endpoints: Vec<EpubTextEndpoint>,
}

pub(super) struct NativeTextState {
    fonts: FontSystem,
    cache: SwashCache,
    aliases: HashMap<String, String>,
    styles: HashMap<String, Vec<Style>>,
    weights: HashMap<String, Vec<(Style, f32, f32)>>,
}

impl NativeTextState {
    pub(super) fn empty() -> Self {
        Self::new(&Database::new(), &[], &[])
    }
    pub(super) fn new(source: &Database, ids: &[fontdb::ID], faces: &[EpubFontFace]) -> Self {
        Self::new_cancellable(source, ids, faces, None).expect("uncancelled native text setup")
    }
    pub(super) fn new_cancellable(
        source: &Database,
        ids: &[fontdb::ID],
        faces: &[EpubFontFace],
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Self> {
        let check_cancelled = || -> Result<()> {
            if is_cancelled.is_some_and(|is_cancelled| is_cancelled()) {
                anyhow::bail!("import cancelled");
            }
            Ok(())
        };
        check_cancelled()?;
        let mut db = Database::new();
        let mut aliases = HashMap::new();
        let mut styles = HashMap::<String, Vec<Style>>::new();
        let mut weights = HashMap::<String, Vec<(Style, f32, f32)>>::new();
        if faces.is_empty() {
            // Native text is also the bridge renderer for ordinary EPUB text.
            // Keep its fallback deterministic in headless/mobile packages that
            // do not expose host fonts to the Rust process.
            db.load_font_data(super::pagination::math_layout::MATH_FONT_BYTES.to_vec());
        } else {
            db.load_system_fonts();
        }
        for (id, declared) in ids.iter().zip(faces) {
            check_cancelled()?;
            if let Some(mut info) = source.face(*id).cloned() {
                info.id = fontdb::ID::dummy();
                let folded = folded_family(&declared.family);
                let alias_index = aliases.len();
                let synthetic = aliases.entry(folded.clone()).or_insert_with(|| {
                    (alias_index..)
                        .map(|index| format!("\u{f0000}shosai-epub-family-{index}"))
                        .find(|candidate| {
                            !db.faces().any(|face| {
                                face.families.iter().any(|(family, _)| family == candidate)
                            })
                        })
                        .expect("synthetic EPUB family index space is inexhaustible")
                });
                info.families = vec![(synthetic.clone(), Language::English_UnitedStates)];
                info.style = match declared.style {
                    EpubFontStyle::Normal => Style::Normal,
                    EpubFontStyle::Italic => Style::Italic,
                    EpubFontStyle::Oblique => Style::Oblique,
                };
                styles.entry(folded.clone()).or_default().push(info.style);
                weights.entry(folded).or_default().push((
                    info.style,
                    declared.weight.min(),
                    declared.weight.max(),
                ));
                info.weight = Weight(
                    ((declared.weight.min() + declared.weight.max()) / 2.0)
                        .round()
                        .clamp(1.0, 1000.0) as u16,
                );
                info.stretch = Stretch::Normal;
                db.push_face_info(info);
            }
        }
        check_cancelled()?;
        Ok(Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), db),
            cache: SwashCache::new(),
            aliases,
            styles,
            weights,
        })
    }

    #[cfg(test)]
    pub(super) fn matched_postscript_name(&self, family: &str, style: Style) -> Option<&str> {
        let family = self.aliases.get(&folded_family(family))?;
        let id = self.fonts.db().query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            style,
            ..fontdb::Query::default()
        })?;
        self.fonts
            .db()
            .face(id)
            .map(|face| face.post_script_name.as_str())
    }

    #[cfg(test)]
    pub(super) fn retained_raster_image_count(&self) -> usize {
        0
    }
}

impl EpubFontBook {
    /// Shapes and rasterizes rich text without registering fonts globally.
    pub fn layout_text(&self, request: &EpubTextRequest) -> Result<EpubTextLayout> {
        self.layout_text_cancellable(request, &|| false)
    }

    pub(crate) fn layout_text_cancellable(
        &self,
        request: &EpubTextRequest,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<EpubTextLayout> {
        self.layout_text_inner(request, true, is_cancelled)
    }

    /// Shapes and measures rich text without allocating line bitmaps.
    pub fn measure_text(&self, request: &EpubTextRequest) -> Result<EpubTextLayout> {
        self.layout_text_inner(request, false, &|| false)
    }

    pub(crate) fn measure_text_cancellable(
        &self,
        request: &EpubTextRequest,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<EpubTextLayout> {
        self.layout_text_inner(request, false, is_cancelled)
    }

    fn layout_text_inner(
        &self,
        request: &EpubTextRequest,
        rasterize: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<EpubTextLayout> {
        check_cancelled(is_cancelled)?;
        validate(request)?;
        let visible_text: String = request.runs.iter().map(|run| run.text.as_str()).collect();
        let paragraphs = paragraph_ranges(&visible_text);
        if paragraphs.len() <= 1 {
            return self.layout_text_single(request, rasterize, is_cancelled);
        }

        let requests = paragraphs
            .iter()
            .map(|range| paragraph_request(request, range, &visible_text))
            .collect::<Result<Vec<_>>>()?;
        if rasterize {
            let pixels = requests.iter().try_fold(0_usize, |pixels, paragraph| {
                check_cancelled(is_cancelled)?;
                self.layout_text_single(paragraph, false, is_cancelled)?
                    .lines
                    .iter()
                    .try_fold(pixels, |pixels, line| {
                        pixels
                            .checked_add(line.pixel_width as usize * line.pixel_height as usize)
                            .context("EPUB text bitmap dimensions overflow")
                    })
            })?;
            if pixels > EPUB_TEXT_MAX_PIXELS {
                bail!("EPUB text output exceeds the {EPUB_TEXT_MAX_PIXELS}-pixel per-call ceiling");
            }
        }

        let mut result = EpubTextLayout {
            width: 0.0,
            height: 0.0,
            lines: Vec::new(),
            links: Vec::new(),
            endpoints: Vec::new(),
        };
        for (index, paragraph) in requests.iter().enumerate() {
            check_cancelled(is_cancelled)?;
            let range = &paragraphs[index];
            let scalar_start = visible_text[..range.start].chars().count();
            let separator_end = paragraphs
                .get(index + 1)
                .map_or(visible_text.len(), |next| next.start);
            let scalar_end = visible_text[..separator_end].chars().count();
            let mut layout = self.layout_text_single(paragraph, rasterize, is_cancelled)?;
            for line in &mut layout.lines {
                line.top += result.height;
                line.scalars = line.scalars.start + scalar_start..line.scalars.end + scalar_start;
            }
            if let Some(last) = layout.lines.last_mut() {
                last.scalars.end = scalar_end;
            }
            for hit in &mut layout.links {
                hit.rect.y += result.height;
                hit.scalars = hit.scalars.start + scalar_start..hit.scalars.end + scalar_start;
            }
            for endpoint in &mut layout.endpoints {
                endpoint.rect.y += result.height;
                endpoint.scalar += scalar_start;
                endpoint.scalar_start += scalar_start;
                endpoint.scalar_end += scalar_start;
                endpoint.visual_line += result.lines.len();
            }
            result.width = result.width.max(layout.width);
            result.height += layout.height;
            result.lines.extend(layout.lines);
            result.links.extend(layout.links);
            checked_extend_endpoints(&mut result.endpoints, layout.endpoints)?;
        }
        Ok(result)
    }

    fn layout_text_single(
        &self,
        request: &EpubTextRequest,
        rasterize: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<EpubTextLayout> {
        check_cancelled(is_cancelled)?;
        let mut state = self.native.lock().map_err(|_| {
            anyhow::anyhow!("EPUB text renderer lock is poisoned; discard and reopen this book")
        })?;
        check_cancelled(is_cancelled)?;
        #[cfg(test)]
        if let Some(entries) = self.renderer_entries.lock().unwrap().as_ref() {
            entries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let NativeTextState {
            fonts,
            cache,
            aliases,
            styles,
            weights,
        } = &mut *state;
        let default_size = request.runs.first().map_or(16.0, |r| r.font_size);
        let mut buffer = Buffer::new(fonts, Metrics::new(default_size, request.line_height));
        buffer.set_size(fonts, Some(request.max_width), None);
        buffer.set_wrap(fonts, Wrap::WordOrGlyph);
        let (isolate, pop_isolate) = match request.direction {
            EpubTextDirection::LeftToRight => ("\u{2066}", "\u{2069}"),
            EpubTextDirection::RightToLeft => ("\u{2067}", "\u{2069}"),
        };
        let control_attrs = Attrs::new()
            .metrics(Metrics::new(default_size, request.line_height))
            .color(Color::rgba(0, 0, 0, 0))
            .metadata(usize::MAX);
        let synthetic_bold = request
            .runs
            .iter()
            .map(|run| {
                run.bold
                    && run.family.as_deref().is_some_and(|family| {
                        let family = folded_family(family);
                        styles.get(&family).is_some_and(|available| {
                            let selected = select_style(available, run.italic).0;
                            !weights.get(&family).is_some_and(|faces| {
                                faces.iter().any(|(style, min, max)| {
                                    *style == selected && *min <= 700.0 && *max >= 700.0
                                })
                            })
                        })
                    })
            })
            .collect::<Vec<_>>();
        let attrs = std::iter::once((isolate, control_attrs.clone()))
            .chain(request.runs.iter().enumerate().map(|(i, run)| {
                let folded = run.family.as_deref().map(folded_family);
                let family = folded
                    .as_ref()
                    .and_then(|family| aliases.get(family))
                    .map_or_else(
                        || {
                            if run.monospace {
                                cosmic_text::Family::Monospace
                            } else {
                                cosmic_text::Family::SansSerif
                            }
                        },
                        |family| cosmic_text::Family::Name(family),
                    );
                let requested_style = if run.italic {
                    Style::Italic
                } else {
                    Style::Normal
                };
                let (style, cache_key_flags) = folded
                    .as_ref()
                    .and_then(|family| styles.get(family))
                    .map_or((requested_style, CacheKeyFlags::empty()), |available| {
                        select_style(available, run.italic)
                    });
                let a = Attrs::new()
                    .family(family)
                    .weight(if run.bold {
                        Weight::BOLD
                    } else {
                        Weight::NORMAL
                    })
                    .style(style)
                    .cache_key_flags(cache_key_flags)
                    .metrics(Metrics::new(run.font_size, request.line_height))
                    .color(Color::rgba(
                        run.foreground[0],
                        run.foreground[1],
                        run.foreground[2],
                        run.foreground[3],
                    ))
                    .metadata(i);
                (run.text.as_str(), a)
            }))
            .chain(std::iter::once((pop_isolate, control_attrs)));
        let align = match request.align {
            EpubTextAlign::Left => Align::Left,
            EpubTextAlign::Center => Align::Center,
            EpubTextAlign::Right => Align::Right,
            EpubTextAlign::Justified => Align::Justified,
        };
        buffer.set_rich_text(fonts, attrs, &Attrs::new(), Shaping::Advanced, Some(align));
        check_cancelled(is_cancelled)?;
        buffer.shape_until_scroll(fonts, false);
        check_cancelled(is_cancelled)?;

        let visible_text: String = request.runs.iter().map(|r| r.text.as_str()).collect();
        let scalar_boundaries = scalar_boundaries(&visible_text);
        let text = format!("{isolate}{visible_text}{pop_isolate}");
        let visible_bytes = isolate.len()..isolate.len() + visible_text.len();
        let paragraphs = paragraph_ranges(&text);
        let pw = (request.max_width * request.scale).ceil() as usize;
        let runs: Vec<_> = buffer.layout_runs().collect();
        let raw_ranges = runs
            .iter()
            .map(|run| {
                let paragraph = paragraphs
                    .get(run.line_i)
                    .context("EPUB shaper returned an unknown paragraph index")?;
                let start = run
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.metadata != usize::MAX)
                    .map(|glyph| paragraph.start + glyph.start)
                    .min();
                let end = run
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.metadata != usize::MAX)
                    .map(|glyph| paragraph.start + glyph.end)
                    .max();
                Ok::<_, anyhow::Error>(start.zip(end))
            })
            .collect::<Result<Vec<_>>>()?;
        let line_ranges = runs
            .iter()
            .enumerate()
            .map(|(index, run)| {
                let paragraph = &paragraphs[run.line_i];
                let partition_start = if index > 0 && runs[index - 1].line_i == run.line_i {
                    raw_ranges[index]
                        .as_ref()
                        .or(raw_ranges[index - 1].as_ref())
                        .map_or(paragraph.start, |r| r.0)
                } else {
                    paragraph.start
                };
                let partition_end = if runs
                    .get(index + 1)
                    .is_some_and(|next| next.line_i == run.line_i)
                {
                    raw_ranges[index + 1]
                        .as_ref()
                        .map_or(paragraph.end, |r| r.0)
                } else {
                    paragraphs
                        .get(run.line_i + 1)
                        .map_or(visible_bytes.end, |next| next.start)
                };
                checked_scalar_range(
                    &scalar_boundaries,
                    &visible_bytes,
                    partition_start,
                    partition_end,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let pixels = runs.iter().try_fold(0_usize, |pixels, run| {
            let height = (run.line_height * request.scale).ceil() as usize;
            pixels
                .checked_add(
                    pw.checked_mul(height)
                        .context("EPUB text bitmap dimensions overflow")?,
                )
                .context("EPUB text bitmap dimensions overflow")
        })?;
        if rasterize && pixels > EPUB_TEXT_MAX_PIXELS {
            bail!("EPUB text output exceeds the {EPUB_TEXT_MAX_PIXELS}-pixel per-call ceiling");
        }
        let mut lines = Vec::with_capacity(runs.len());
        let mut links = Vec::new();
        let mut endpoints = Vec::new();
        for (visual_line, (run, line_range)) in runs.into_iter().zip(line_ranges).enumerate() {
            check_cancelled(is_cancelled)?;
            let ph = (run.line_height * request.scale).ceil() as usize;
            let mut rgba = if rasterize {
                vec![
                    0;
                    pw.checked_mul(ph)
                        .and_then(|v| v.checked_mul(4))
                        .context("EPUB line bitmap size overflow")?
                ]
            } else {
                Vec::new()
            };
            let base = paragraphs[run.line_i].start;
            for glyph in run.glyphs {
                check_cancelled(is_cancelled)?;
                let start = (base + glyph.start).max(visible_bytes.start);
                let end = (base + glyph.end).min(visible_bytes.end);
                if start >= end || glyph.metadata == usize::MAX {
                    continue;
                }
                let local_start = start - visible_bytes.start;
                let local_end = end - visible_bytes.start;
                let scalars = checked_scalar_range(
                    &scalar_boundaries,
                    &(0..visible_text.len()),
                    local_start,
                    local_end,
                )?;
                let rect = EpubTextRect {
                    x: glyph.x,
                    y: run.line_top,
                    width: glyph.w.max(0.0),
                    height: run.line_height,
                };
                let cluster = &visible_text[local_start..local_end];
                let graphemes = cluster.graphemes(true).collect::<Vec<_>>();
                let grapheme_width = rect.width / graphemes.len().max(1) as f32;
                let mut cluster_scalar = scalars.start;
                let grapheme_count = graphemes.len();
                for (position, grapheme) in graphemes.into_iter().enumerate() {
                    check_cancelled(is_cancelled)?;
                    let after = cluster_scalar + grapheme.chars().count();
                    let (left, right) = if glyph.level.is_rtl() {
                        (after, cluster_scalar)
                    } else {
                        (cluster_scalar, after)
                    };
                    let physical_position = if glyph.level.is_rtl() {
                        grapheme_count - position - 1
                    } else {
                        position
                    };
                    let x = rect.x + physical_position as f32 * grapheme_width;
                    checked_push_endpoint(
                        &mut endpoints,
                        EpubTextEndpoint {
                            rect: EpubTextRect {
                                x,
                                y: rect.y,
                                width: grapheme_width / 2.0,
                                height: rect.height,
                            },
                            caret_x: x,
                            scalar: left,
                            scalar_start: cluster_scalar,
                            scalar_end: after,
                            visual_line,
                        },
                    )?;
                    checked_push_endpoint(
                        &mut endpoints,
                        EpubTextEndpoint {
                            rect: EpubTextRect {
                                x: x + grapheme_width / 2.0,
                                y: rect.y,
                                width: grapheme_width / 2.0,
                                height: rect.height,
                            },
                            caret_x: x + grapheme_width,
                            scalar: right,
                            scalar_start: cluster_scalar,
                            scalar_end: after,
                            visual_line,
                        },
                    )?;
                    cluster_scalar = after;
                }
                if rasterize {
                    for h in &request.highlights {
                        if h.scalars.start < scalars.end && scalars.start < h.scalars.end {
                            fill(
                                &mut rgba,
                                (pw, ph),
                                (
                                    (glyph.x * request.scale) as i32,
                                    0,
                                    (glyph.w * request.scale).ceil() as i32,
                                    ph as i32,
                                ),
                                h.color,
                            );
                        }
                    }
                }
                if let Some(link) = request
                    .runs
                    .get(glyph.metadata)
                    .and_then(|r| r.link.clone())
                {
                    links.push(EpubTextHit {
                        rect,
                        scalars: scalars.clone(),
                        link,
                    });
                }
                if rasterize {
                    let physical = glyph.physical((0.0, 0.0), request.scale);
                    let color = glyph.color_opt.unwrap_or(Color::rgb(0, 0, 0));
                    let synthetic_bold = synthetic_bold[glyph.metadata];
                    with_pixels_uncached(cache, fonts, physical.cache_key, color, |x, y, c| {
                        fill(
                            &mut rgba,
                            (pw, ph),
                            (
                                physical.x + x,
                                ((run.line_y - run.line_top) * request.scale) as i32
                                    + physical.y
                                    + y,
                                1,
                                1,
                            ),
                            [c.r(), c.g(), c.b(), c.a()],
                        );
                        if synthetic_bold {
                            fill(
                                &mut rgba,
                                (pw, ph),
                                (
                                    physical.x + x + 1,
                                    ((run.line_y - run.line_top) * request.scale) as i32
                                        + physical.y
                                        + y,
                                    1,
                                    1,
                                ),
                                [c.r(), c.g(), c.b(), c.a()],
                            );
                        }
                    });
                }
            }
            lines.push(EpubTextLine {
                top: run.line_top,
                width: run.line_w,
                rtl: request.direction == EpubTextDirection::RightToLeft,
                scalars: line_range,
                pixel_width: pw as u32,
                pixel_height: ph as u32,
                rgba,
            });
        }
        let width = lines.iter().map(|l| l.width).fold(0.0_f32, f32::max);
        let height = lines.last().map_or(0.0, |line| {
            line.top + line.pixel_height as f32 / request.scale
        });
        Ok(EpubTextLayout {
            width,
            height,
            lines,
            links,
            endpoints,
        })
    }
}

fn with_pixels_uncached(
    cache: &mut SwashCache,
    fonts: &mut FontSystem,
    key: cosmic_text::CacheKey,
    base: Color,
    mut draw: impl FnMut(i32, i32, Color),
) {
    let Some(image) = cache.get_image_uncached(fonts, key) else {
        return;
    };
    let x = image.placement.left;
    let y = -image.placement.top;
    match image.content {
        SwashContent::Mask => {
            for (index, alpha) in image.data.into_iter().enumerate() {
                let off_x = index as i32 % image.placement.width as i32;
                let off_y = index as i32 / image.placement.width as i32;
                draw(
                    x + off_x,
                    y + off_y,
                    Color((u32::from(alpha) << 24) | base.0 & 0x00ff_ffff),
                );
            }
        }
        SwashContent::Color => {
            for (index, pixel) in image.data.chunks_exact(4).enumerate() {
                let off_x = index as i32 % image.placement.width as i32;
                let off_y = index as i32 / image.placement.width as i32;
                draw(
                    x + off_x,
                    y + off_y,
                    Color::rgba(pixel[0], pixel[1], pixel[2], pixel[3]),
                );
            }
        }
        SwashContent::SubpixelMask => {}
    }
}

fn paragraph_request(
    request: &EpubTextRequest,
    range: &Range<usize>,
    visible_text: &str,
) -> Result<EpubTextRequest> {
    let mut runs = Vec::new();
    let mut offset = 0;
    for run in &request.runs {
        let run_range = offset..offset + run.text.len();
        let start = range.start.max(run_range.start);
        let end = range.end.min(run_range.end);
        if start < end {
            let mut sliced = run.clone();
            sliced.text = visible_text[start..end].to_owned();
            runs.push(sliced);
        }
        offset = run_range.end;
    }
    if runs.is_empty() {
        let empty = request.runs.first().map_or(
            EpubTextRun {
                text: String::new(),
                family: None,
                monospace: false,
                font_size: 16.0,
                bold: false,
                italic: false,
                foreground: [0, 0, 0, 0],
                link: None,
            },
            |run| EpubTextRun {
                text: String::new(),
                family: run.family.clone(),
                monospace: run.monospace,
                font_size: run.font_size,
                bold: run.bold,
                italic: run.italic,
                foreground: run.foreground,
                link: None,
            },
        );
        runs.push(empty);
    }
    let scalar_start = visible_text[..range.start].chars().count();
    let scalar_end = visible_text[..range.end].chars().count();
    let highlights = request
        .highlights
        .iter()
        .filter_map(|highlight| {
            let start = highlight.scalars.start.max(scalar_start);
            let end = highlight.scalars.end.min(scalar_end);
            (start < end).then(|| EpubTextHighlight {
                scalars: start - scalar_start..end - scalar_start,
                color: highlight.color,
            })
        })
        .collect();
    Ok(EpubTextRequest {
        runs,
        max_width: request.max_width,
        line_height: request.line_height,
        scale: request.scale,
        align: request.align,
        direction: request.direction,
        highlights,
    })
}

fn select_style(available: &[Style], italic: bool) -> (Style, CacheKeyFlags) {
    let requested = if italic { Style::Italic } else { Style::Normal };
    if available.contains(&requested) {
        (requested, CacheKeyFlags::empty())
    } else if italic && available.contains(&Style::Oblique) {
        (Style::Oblique, CacheKeyFlags::empty())
    } else if italic && available.contains(&Style::Normal) {
        (Style::Normal, CacheKeyFlags::FAKE_ITALIC)
    } else {
        (available[0], CacheKeyFlags::empty())
    }
}

fn folded_family(value: &str) -> String {
    value.case_fold().collect()
}

fn validate(r: &EpubTextRequest) -> Result<()> {
    if !r.max_width.is_finite()
        || r.max_width <= 0.0
        || !r.line_height.is_finite()
        || r.line_height <= 0.0
        || !r.scale.is_finite()
        || r.scale <= 0.0
    {
        bail!("EPUB text geometry must be finite and positive");
    }
    if r.max_width * r.scale > u32::MAX as f32 || r.line_height * r.scale > u32::MAX as f32 {
        bail!("EPUB text pixel geometry is out of range");
    }
    if r.runs
        .iter()
        .any(|x| !x.font_size.is_finite() || x.font_size <= 0.0)
    {
        bail!("EPUB font sizes must be finite and positive");
    }
    let scalars = r.runs.iter().try_fold(0_usize, |total, run| {
        total
            .checked_add(run.text.chars().count())
            .context("EPUB text length overflow")
    })?;
    if scalars > EPUB_TEXT_MAX_SCALARS {
        bail!("EPUB text exceeds the {EPUB_TEXT_MAX_SCALARS}-scalar per-request ceiling");
    }
    let endpoints = r
        .runs
        .iter()
        .flat_map(|run| run.text.graphemes(true))
        .try_fold(0_usize, |total, _| {
            total
                .checked_add(2)
                .context("EPUB text endpoint count overflow")
        })?;
    if endpoints > EPUB_TEXT_MAX_ENDPOINTS {
        return Err(ResourceLimitError(format!(
            "EPUB text exceeds the {EPUB_TEXT_MAX_ENDPOINTS}-endpoint retained geometry ceiling"
        ))
        .into());
    }
    let paragraphs = r
        .runs
        .iter()
        .flat_map(|run| run.text.chars())
        .filter(|character| is_bidi_paragraph_separator(*character))
        .count()
        .saturating_add(1);
    if paragraphs > EPUB_TEXT_MAX_PARAGRAPHS {
        bail!("EPUB text exceeds the {EPUB_TEXT_MAX_PARAGRAPHS}-paragraph per-request ceiling");
    }
    Ok(())
}

fn checked_push_endpoint(
    endpoints: &mut Vec<EpubTextEndpoint>,
    endpoint: EpubTextEndpoint,
) -> Result<()> {
    if endpoints.len() >= EPUB_TEXT_MAX_ENDPOINTS {
        return Err(ResourceLimitError(format!(
            "EPUB text exceeds the {EPUB_TEXT_MAX_ENDPOINTS}-endpoint retained geometry ceiling"
        ))
        .into());
    }
    endpoints.push(endpoint);
    Ok(())
}

fn checked_extend_endpoints(
    endpoints: &mut Vec<EpubTextEndpoint>,
    additional: Vec<EpubTextEndpoint>,
) -> Result<()> {
    if endpoints
        .len()
        .checked_add(additional.len())
        .is_none_or(|count| count > EPUB_TEXT_MAX_ENDPOINTS)
    {
        return Err(ResourceLimitError(format!(
            "EPUB text exceeds the {EPUB_TEXT_MAX_ENDPOINTS}-endpoint retained geometry ceiling"
        ))
        .into());
    }
    endpoints.extend(additional);
    Ok(())
}

fn check_cancelled(is_cancelled: &dyn Fn() -> bool) -> Result<()> {
    if is_cancelled() {
        anyhow::bail!("EPUB text operation cancelled");
    }
    Ok(())
}
fn is_bidi_paragraph_separator(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{001c}' | '\u{001d}' | '\u{001e}' | '\u{0085}' | '\u{2029}'
    )
}
fn paragraph_ranges(text: &str) -> Vec<Range<usize>> {
    let base = text.as_ptr() as usize;
    let mut ranges = BidiParagraphs::new(text)
        .map(|paragraph| {
            let start = paragraph.as_ptr() as usize - base;
            start..start + paragraph.len()
        })
        .collect::<Vec<_>>();
    if text.chars().last().is_some_and(is_bidi_paragraph_separator) {
        ranges.push(text.len()..text.len());
    }
    ranges
}
fn checked_scalar_range(
    scalar_boundaries: &[usize],
    visible_bytes: &Range<usize>,
    start: usize,
    end: usize,
) -> Result<Range<usize>> {
    let start = start.clamp(visible_bytes.start, visible_bytes.end) - visible_bytes.start;
    let end = end.clamp(visible_bytes.start, visible_bytes.end) - visible_bytes.start;
    let start = scalar_boundaries
        .binary_search(&start)
        .map_err(|_| anyhow::anyhow!("EPUB shaper returned a non-character source boundary"))?;
    let end = scalar_boundaries
        .binary_search(&end)
        .map_err(|_| anyhow::anyhow!("EPUB shaper returned a non-character source boundary"))?;
    Ok(start..end)
}
fn scalar_boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .collect()
}
fn fill(buf: &mut [u8], dimensions: (usize, usize), rect: (i32, i32, i32, i32), c: [u8; 4]) {
    let (w, h) = dimensions;
    let (x, y, ww, hh) = rect;
    for yy in y.max(0)..(y + hh).min(h as i32) {
        for xx in x.max(0)..(x + ww).min(w as i32) {
            let i = (yy as usize * w + xx as usize) * 4;
            let a = c[3] as u32;
            let da = buf[i + 3] as u32;
            let oa = a + da * (255 - a) / 255;
            if oa > 0 {
                for k in 0..3 {
                    buf[i + k] =
                        ((c[k] as u32 * a + buf[i + k] as u32 * da * (255 - a) / 255) / oa) as u8;
                }
            }
            buf[i + 3] = oa as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_oblique_precedes_synthesized_italic() {
        assert_eq!(
            select_style(&[Style::Normal, Style::Oblique], true),
            (Style::Oblique, CacheKeyFlags::empty())
        );
    }

    #[test]
    fn emitted_endpoint_limit_is_typed_at_push_and_combine_boundaries() {
        let endpoint = EpubTextEndpoint {
            rect: EpubTextRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            caret_x: 0.0,
            scalar: 0,
            scalar_start: 0,
            scalar_end: 1,
            visual_line: 0,
        };
        let mut endpoints = vec![endpoint; EPUB_TEXT_MAX_ENDPOINTS];
        let error = checked_push_endpoint(&mut endpoints, endpoint).unwrap_err();
        assert!(error.is::<ResourceLimitError>());

        let mut endpoints = vec![endpoint; EPUB_TEXT_MAX_ENDPOINTS];
        let error = checked_extend_endpoints(&mut endpoints, vec![endpoint]).unwrap_err();
        assert!(error.is::<ResourceLimitError>());
    }

    #[test]
    fn raster_loop_cancellation_is_observable() {
        let error = check_cancelled(&|| true).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }
}
