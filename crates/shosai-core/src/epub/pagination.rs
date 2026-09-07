//! EPUB-specific pagination models and renderer-neutral page geometry helpers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::render::{ContentNode, TableRow, TableRowGroup};
use super::{
    EpubDoc, EpubFontBook, EpubTextAlign, EpubTextDirection, EpubTextLayout, EpubTextRequest,
    EpubTextRun,
};

pub mod math_layout;

pub const BLOCKQUOTE_SPACING: f32 = 8.0;
pub const TEXT_LINE_HEIGHT: f32 = 1.2;
pub const AVERAGE_CHARACTER_WIDTH: f32 = 0.55;
pub const MAX_CHARACTERS_PER_LINE: usize = 72;
pub const PAGE_NUMBER_SIZE: f32 = 11.0;
pub const MAX_EPUB_PAGES: usize = 10_000;
pub const MIN_EPUB_TABLE_WIDTH: f32 = 360.0;
pub const EPUB_TABLE_CELL_PADDING: f32 = 6.0;
pub const EPUB_TABLE_CELL_SPACING: f32 = 4.0;
pub const EPUB_TABLE_ROW_SPACING: f32 = 8.0;
pub const INLINE_MATH_WRAP_SPACING: f32 = 0.25;
pub const MAX_INLINE_MATH_FLOW_ITEMS: usize = 256;
const MAX_INLINE_MATH_LINE_HEIGHTS: f32 = 3.0;
const MIN_EPUB_TABLE_CELL_WIDTH: f32 = 120.0;
const MAX_EPUB_TABLE_WIDTH: f32 = 4_096.0;
const MAX_EPUB_TABLE_COLUMNS: usize = 256;
const EPUB_PAGINATION_SHAPE_CHUNK: usize = 4 * 1024;
const EPUB_PAGINATION_LOOP_CHUNK: usize = 64;

#[cfg(test)]
thread_local! {
    static TABLE_PLACEMENT_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TABLE_CELL_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TABLE_CANCEL_AFTER_VISITS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static TABLE_CELL_INTERNAL_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TABLE_CANCEL_AFTER_INTERNAL_VISITS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn table_cell_checkpoint(
    cancellation: Option<&EpubPaginationCancellation>,
    cell_index: usize,
) -> bool {
    #[cfg(test)]
    TABLE_CELL_VISITS.with(|visits| {
        let visited = visits.get() + 1;
        visits.set(visited);
        TABLE_CANCEL_AFTER_VISITS.with(|limit| {
            if limit.get().is_some_and(|limit| visited >= limit)
                && let Some(cancellation) = cancellation
            {
                cancellation.cancel();
            }
        });
    });
    !cell_index.is_multiple_of(EPUB_PAGINATION_LOOP_CHUNK)
        || cancellation.is_none_or(|cancellation| !cancellation.is_cancelled())
}

fn table_row_checkpoint(
    cancellation: Option<&EpubPaginationCancellation>,
    row_index: usize,
) -> bool {
    !row_index.is_multiple_of(EPUB_PAGINATION_LOOP_CHUNK)
        || cancellation.is_none_or(|cancellation| !cancellation.is_cancelled())
}

fn table_cell_internal_checkpoint(
    cancellation: Option<&EpubPaginationCancellation>,
    index: usize,
) -> bool {
    #[cfg(test)]
    TABLE_CELL_INTERNAL_VISITS.with(|visits| {
        let visited = visits.get() + 1;
        visits.set(visited);
        TABLE_CANCEL_AFTER_INTERNAL_VISITS.with(|limit| {
            if limit.get().is_some_and(|limit| visited >= limit)
                && let Some(cancellation) = cancellation
            {
                cancellation.cancel();
            }
        });
    });
    !index.is_multiple_of(EPUB_PAGINATION_LOOP_CHUNK)
        || cancellation.is_none_or(|cancellation| !cancellation.is_cancelled())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSize {
    pub width: f32,
    pub height: f32,
}

impl LayoutSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

type Size = LayoutSize;

pub struct EpubPaginationBudget {
    remaining_page_breaks: usize,
    cancellation: Option<EpubPaginationCancellation>,
}

#[derive(Debug, Clone, Default)]
pub struct EpubPaginationCancellation(Arc<AtomicBool>);

impl EpubPaginationCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for EpubPaginationBudget {
    fn default() -> Self {
        Self {
            remaining_page_breaks: MAX_EPUB_PAGES - 1,
            cancellation: None,
        }
    }
}

impl EpubPaginationBudget {
    pub fn for_document(chapters: usize) -> Self {
        Self {
            remaining_page_breaks: MAX_EPUB_PAGES.saturating_sub(chapters),
            cancellation: None,
        }
    }

    pub fn with_cancellation(mut self, cancellation: EpubPaginationCancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(EpubPaginationCancellation::is_cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct PageNode {
    pub node: ContentNode,
    pub text_offset: usize,
    /// Pagination-owned block geometry. Adjacent authored margins are resolved
    /// against the original chapter, never against a page-local fragment list.
    pub block_before: f32,
    pub block_after: f32,
}

pub type PageNodes = Vec<PageNode>;

#[derive(Debug, Clone)]
pub struct Page {
    pub chapter: usize,
    pub title: Option<String>,
    pub nodes: PageNodes,
}

pub fn page_size(
    available: LayoutSize,
    spread: bool,
    gutter: f32,
    font_size: f32,
    line_spacing: f32,
) -> LayoutSize {
    let page_count = if spread { 2.0 } else { 1.0 };
    let available_text_width =
        ((available.width - gutter * (page_count - 1.0)) / page_count - 40.0).max(120.0);
    let readable_text_width = font_size * AVERAGE_CHARACTER_WIDTH * MAX_CHARACTERS_PER_LINE as f32;
    let footer_height = PAGE_NUMBER_SIZE * TEXT_LINE_HEIGHT + font_size * line_spacing;
    LayoutSize::new(
        available_text_width.min(readable_text_width),
        (available.height - 40.0 - footer_height).max(120.0),
    )
}

pub fn spread_start(page: usize, page_count: usize, spread: bool) -> usize {
    let page = page.min(page_count.saturating_sub(1));
    if spread { page - page % 2 } else { page }
}

pub fn epub_node_block_sides(
    node: &ContentNode,
    font_size: f32,
    default_spacing: f32,
) -> (f32, f32) {
    let Some(style) = node.style() else {
        return (0.0, default_spacing.max(0.0));
    };
    if style.block_before_em.is_none() && style.block_after_em.is_none() {
        return (0.0, default_spacing.max(0.0));
    }
    (
        style.block_before_em.unwrap_or(0.0) * font_size,
        style.block_after_em.unwrap_or(0.0) * font_size,
    )
}

/// Collapsed spacing at a node boundary. The outer boundaries retain the
/// first node's before and last node's after margin.
pub fn epub_node_boundary_spacing(
    nodes: &[ContentNode],
    boundary: usize,
    font_size: f32,
    default_spacing: f32,
) -> f32 {
    match boundary {
        0 => nodes.first().map_or(0.0, |node| {
            epub_node_block_sides(node, font_size, default_spacing).0
        }),
        boundary if boundary >= nodes.len() => nodes.last().map_or(0.0, |node| {
            epub_node_block_sides(node, font_size, default_spacing).1
        }),
        boundary => {
            let after = epub_node_block_sides(&nodes[boundary - 1], font_size, default_spacing).1;
            let before = epub_node_block_sides(&nodes[boundary], font_size, default_spacing).0;
            after.max(before)
        }
    }
}

pub fn epub_node_list_spacing(nodes: &[ContentNode], font_size: f32, default_spacing: f32) -> f32 {
    (0..=nodes.len())
        .map(|boundary| epub_node_boundary_spacing(nodes, boundary, font_size, default_spacing))
        .sum()
}

pub fn epub_fragment_boundary_spacing(
    nodes: &[ContentNode],
    boundary: usize,
    font_size: f32,
    default_spacing: f32,
    style: &shosai_core::epub::render::NodeStyle,
) -> f32 {
    if boundary == 0 && style.fragment_before || boundary >= nodes.len() && style.fragment_after {
        0.0
    } else {
        epub_node_boundary_spacing(nodes, boundary, font_size, default_spacing)
    }
}

fn epub_fragment_list_spacing(
    nodes: &[ContentNode],
    font_size: f32,
    default_spacing: f32,
    style: &shosai_core::epub::render::NodeStyle,
) -> f32 {
    (0..=nodes.len())
        .map(|boundary| {
            epub_fragment_boundary_spacing(nodes, boundary, font_size, default_spacing, style)
        })
        .sum()
}

#[derive(Debug, Clone, Copy)]
pub struct EpubImageLayout {
    pub width: f32,
    pub height: f32,
    pub caption_height: f32,
    pub caption_gap: f32,
}

impl EpubImageLayout {
    fn total_height(self) -> f32 {
        self.height + self.caption_height + self.caption_gap
    }
}

pub fn epub_image_margin_left(
    style: &shosai_core::epub::render::NodeStyle,
    font_size: f32,
    available_width: f32,
) -> f32 {
    (style.margin_left_em.unwrap_or(0.0).max(0.0) * font_size).min((available_width - 1.0).max(0.0))
}

pub fn epub_image_layout(
    node: &ContentNode,
    font_size: f32,
    available_width: f32,
    percentage_height_basis: Option<f32>,
    maximum_height: Option<f32>,
    fonts: Option<&EpubFontBook>,
) -> Option<EpubImageLayout> {
    let ContentNode::Image {
        alt,
        style,
        caption,
        caption_style,
        intrinsic_size,
        ..
    } = node
    else {
        return None;
    };
    let containing_width = if available_width.is_finite() {
        available_width.max(1.0)
    } else {
        1.0
    };
    let available_width =
        (containing_width - epub_image_margin_left(style, font_size, containing_width)).max(1.0);
    let percentage_height_basis =
        percentage_height_basis.filter(|height| height.is_finite() && *height > 0.0);
    let maximum_height = maximum_height.filter(|height| height.is_finite() && *height > 0.0);
    let estimate_caption_text_height = |width: f32, size: f32| {
        let span_scale = spans_font_scale(caption);
        let per_line = (width / (size * span_scale * AVERAGE_CHARACTER_WIDTH))
            .floor()
            .max(1.0) as usize;
        let caption_text = caption
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        let lines = caption_text
            .split('\n')
            .map(|line| line.chars().count().div_ceil(per_line).max(1))
            .sum::<usize>()
            .max(1);
        lines as f32 * size * span_scale * TEXT_LINE_HEIGHT
    };
    let Some(intrinsic_size) = intrinsic_size else {
        let caption_size = font_size
            * caption_style
                .as_ref()
                .and_then(|style| style.font_size_multiplier)
                .unwrap_or(1.0);
        let caption_height = if caption.is_empty() {
            0.0
        } else {
            measure_epub_spans(
                fonts,
                caption,
                caption_size,
                available_width,
                caption_style
                    .as_ref()
                    .map_or(Default::default(), |style| style.direction),
                caption_style.as_ref().and_then(|style| style.text_align),
            )
            .map_or_else(
                || estimate_caption_text_height(available_width, caption_size),
                |layout| layout.height,
            ) + inline_math_height_reserve(
                caption,
                caption_size,
                available_width,
                maximum_height.unwrap_or(f32::MAX),
            )
        };
        let caption_gap = if caption.is_empty() {
            0.0
        } else {
            font_size * 0.5
        };
        let estimated_height = (alt.chars().count() + 8).div_ceil(
            (available_width / (font_size * AVERAGE_CHARACTER_WIDTH))
                .floor()
                .max(1.0) as usize,
        ) as f32
            * font_size
            * TEXT_LINE_HEIGHT;
        let height = maximum_height.map_or(estimated_height, |maximum_height| {
            estimated_height.min((maximum_height - caption_height - caption_gap).max(0.0))
        });
        let layout = EpubImageLayout {
            width: available_width,
            height,
            caption_height,
            caption_gap,
        };
        return [
            layout.width,
            layout.height,
            layout.caption_height,
            layout.caption_gap,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value >= 0.0)
        .then_some(layout);
    };
    let intrinsic_width = intrinsic_size.width as f32;
    let intrinsic_height = intrinsic_size.height as f32;
    let resolve_height = |dimension: shosai_core::epub::render::NodeWidth| match dimension {
        shosai_core::epub::render::NodeWidth::Percent(value) => {
            percentage_height_basis.map(|height| value * height)
        }
        shosai_core::epub::render::NodeWidth::Pixels(value) => Some(value),
    };
    let requested_width = match style.width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => value * containing_width,
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value,
        None => style
            .height
            .and_then(resolve_height)
            .map_or(intrinsic_width, |height| {
                height * intrinsic_width / intrinsic_height.max(1.0)
            }),
    };
    let maximum_width = match style.max_width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => value * containing_width,
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value,
        None => available_width,
    };
    let requested_width = if requested_width.is_finite() {
        requested_width
    } else {
        intrinsic_width
    };
    let maximum_width = if maximum_width.is_finite() {
        maximum_width
    } else {
        available_width
    };
    let mut width = requested_width.clamp(1.0, maximum_width.min(available_width).max(1.0));
    // Two authored dimensions intentionally define the replaced-element rectangle;
    // otherwise retain the admitted resource's aspect ratio.
    let mut height = if style.width.is_some() {
        style.height.and_then(resolve_height)
    } else {
        None
    }
    .filter(|height| height.is_finite() && *height > 0.0)
    .unwrap_or(intrinsic_height * width / intrinsic_width.max(1.0))
    .max(1.0);
    if let Some(available_height) = maximum_height
        && height > available_height
    {
        let scale = available_height / height;
        width *= scale;
        height = available_height;
    }
    let caption_size = font_size
        * caption_style
            .as_ref()
            .and_then(|style| style.font_size_multiplier)
            .unwrap_or(1.0);
    let measure_caption = |width| {
        measure_epub_spans(
            fonts,
            caption,
            caption_size,
            width,
            caption_style
                .as_ref()
                .map_or(Default::default(), |style| style.direction),
            caption_style.as_ref().and_then(|style| style.text_align),
        )
        .map_or_else(
            || estimate_caption_text_height(width, caption_size),
            |layout| layout.height,
        ) + inline_math_height_reserve(
            caption,
            caption_size,
            width,
            maximum_height.unwrap_or(f32::MAX),
        )
    };
    let mut caption_height = if caption.is_empty() {
        0.0
    } else {
        measure_caption(width)
    };
    let caption_gap = if caption.is_empty() {
        0.0
    } else {
        font_size * 0.5
    };
    if let Some(available_height) = maximum_height {
        for _ in 0..3 {
            let maximum_image_height = (available_height - caption_height - caption_gap).max(1.0);
            if height <= maximum_image_height {
                break;
            }
            let scale = maximum_image_height / height;
            width *= scale;
            height = maximum_image_height;
            if !caption.is_empty() {
                caption_height = measure_caption(width);
            }
        }
    }
    if ![width, height, caption_height, caption_gap]
        .into_iter()
        .all(|value| value.is_finite() && value >= 0.0)
    {
        return None;
    }
    Some(EpubImageLayout {
        width,
        height,
        caption_height,
        caption_gap,
    })
}

fn split_epub_image_caption(
    node: &ContentNode,
    font_size: f32,
    width: f32,
    maximum_height: f32,
    fonts: Option<&EpubFontBook>,
) -> Option<(ContentNode, ContentNode, usize)> {
    let ContentNode::Image {
        alt,
        style: image_style,
        caption,
        caption_style,
        ..
    } = node
    else {
        return None;
    };
    let caption_len = spans_text_len(caption);
    if caption_len == 0
        || epub_image_layout(
            node,
            font_size,
            width,
            Some(maximum_height),
            Some(maximum_height),
            fonts,
        )
        .is_some_and(|layout| layout.total_height() <= maximum_height)
    {
        return None;
    }

    let mut low = 1;
    let mut high = caption_len;
    let mut fitting = None;
    while low < high {
        let take = low + (high - low) / 2;
        let mut prefix = node.clone();
        let ContentNode::Image { caption, .. } = &mut prefix else {
            unreachable!();
        };
        *caption = slice_epub_spans(caption, 0, take);
        if epub_image_layout(
            &prefix,
            font_size,
            width,
            Some(maximum_height),
            Some(maximum_height),
            fonts,
        )
        .is_some_and(|layout| layout.total_height() <= maximum_height)
        {
            fitting = Some((take, prefix));
            low = take + 1;
        } else {
            high = take;
        }
    }
    let fitted = fitting.or_else(|| {
        let mut prefix = node.clone();
        let ContentNode::Image { caption, .. } = &mut prefix else {
            unreachable!();
        };
        *caption = slice_epub_spans(caption, 0, 1);
        epub_image_layout(
            &prefix,
            font_size,
            width,
            Some(maximum_height),
            Some(maximum_height),
            fonts,
        )
        .is_some_and(|layout| layout.total_height() <= maximum_height)
        .then_some((1, prefix))
    });
    let (take, prefix) = fitted.unwrap_or_else(|| {
        let mut prefix = node.clone();
        let ContentNode::Image { caption, .. } = &mut prefix else {
            unreachable!();
        };
        caption.clear();
        (0, prefix)
    });
    let mut style = caption_style.clone().unwrap_or_default();
    style.block_before_em = Some(0.0);
    style.block_after_em = Some(0.0);
    let detached_layout = epub_image_layout(
        &prefix,
        font_size,
        width,
        Some(maximum_height),
        Some(maximum_height),
        fonts,
    );
    style.width =
        detached_layout.map(|layout| shosai_core::epub::render::NodeWidth::Pixels(layout.width));
    style.margin_left_em = detached_layout.map(|layout| {
        let margin = epub_image_margin_left(image_style, font_size, width);
        let post_margin_width = (width - margin).max(1.0);
        (margin + ((post_margin_width - layout.width) / 2.0).max(0.0)) / font_size
    });
    Some((
        prefix,
        ContentNode::Paragraph(slice_epub_spans(caption, take, caption_len - take), style),
        alt.chars().count() + 1 + take,
    ))
}

pub fn visible_pages(page: usize, page_count: usize, spread: bool) -> Vec<usize> {
    if page_count == 0 {
        return Vec::new();
    }
    let start = spread_start(page, page_count, spread);
    let end = if spread {
        (start + 1).min(page_count - 1)
    } else {
        start
    };
    (start..=end).collect()
}

pub fn paginate_epub_chapter(
    nodes: &[ContentNode],
    title: Option<&str>,
    font_size: f32,
    line_spacing: f32,
    page_size: LayoutSize,
) -> Vec<PageNodes> {
    paginate_epub_chapter_with_budget(
        nodes,
        title,
        font_size,
        line_spacing,
        page_size,
        None,
        &mut EpubPaginationBudget::default(),
    )
}

pub fn paginate_epub_chapter_with_budget(
    nodes: &[ContentNode],
    title: Option<&str>,
    font_size: f32,
    line_spacing: f32,
    page_size: LayoutSize,
    fonts: Option<&EpubFontBook>,
    budget: &mut EpubPaginationBudget,
) -> Vec<PageNodes> {
    let chars_per_line = (page_size.width / (font_size * AVERAGE_CHARACTER_WIDTH).max(1.0))
        .floor()
        .max(12.0) as usize;
    let default_block_spacing = (font_size * line_spacing).max(1.0);
    let lines_per_page = (page_size.height / default_block_spacing).floor().max(4.0) as usize;
    let first_page_has_title = title.is_some();
    let title_height = title
        .map(|title| {
            let title_chars_per_line = scaled_characters_per_line(chars_per_line, 1.5);
            title.chars().count().div_ceil(title_chars_per_line).max(1) as f32
                * font_size
                * 1.5
                * TEXT_LINE_HEIGHT
                + default_block_spacing
        })
        .unwrap_or(0.0);
    let mut pages = vec![Vec::new()];
    let mut remaining = (page_size.height - title_height).max(0.0);
    let mut text_offset = 0;

    for (node_index, node) in nodes.iter().enumerate() {
        if budget.is_cancelled() {
            return Vec::new();
        }
        let block_before = if node_index == 0 {
            epub_node_boundary_spacing(nodes, 0, font_size, default_block_spacing)
        } else {
            0.0
        };
        remaining = (remaining - block_before).max(0.0);
        let block_spacing =
            epub_node_boundary_spacing(nodes, node_index + 1, font_size, default_block_spacing);
        let keep_with_next = match node {
            ContentNode::Heading { .. } => true,
            ContentNode::Paragraph(spans, _) => spans.iter().any(|span| span.link.is_some()),
            _ => false,
        };
        if page_has_content(&pages, first_page_has_title)
            && keep_with_next
            && let Some(ContentNode::BlockQuote { children, .. }) = nodes.get(node_index + 1)
            && let Some(first_child) = children.first()
        {
            let node_height =
                measured_epub_compact_node_height(fonts, node, font_size, page_size.width)
                    .map(|height| height + font_size * line_spacing)
                    .unwrap_or_else(|| {
                        estimated_epub_node_height(
                            node,
                            chars_per_line,
                            lines_per_page,
                            font_size,
                            line_spacing,
                        )
                    });
            let first_child_height =
                measured_epub_compact_node_height(fonts, first_child, font_size, page_size.width)
                    .unwrap_or_else(|| {
                        estimated_epub_compact_node_height(
                            first_child,
                            chars_per_line,
                            lines_per_page,
                            font_size,
                        )
                    });
            if node_height + first_child_height > remaining && push_epub_page(&mut pages, budget) {
                remaining = (page_size.height - block_before).max(0.0);
            }
        }
        if page_has_content(&pages, first_page_has_title)
            && matches!(node, ContentNode::Paragraph(..))
            && let Some(ContentNode::Math { content, style, .. }) = nodes.get(node_index + 1)
            && content.expression.is_some()
        {
            let label_height =
                measured_epub_compact_node_height(fonts, node, font_size, page_size.width)
                    .map(|height| height + block_spacing)
                    .unwrap_or_else(|| {
                        estimated_epub_node_height(
                            node,
                            chars_per_line,
                            lines_per_page,
                            font_size,
                            line_spacing,
                        )
                    });
            let math_height = measured_epub_compact_node_height_bounded(
                fonts,
                &nodes[node_index + 1],
                font_size,
                page_size.width,
                page_size.height,
            )
            .map(|height| height + block_spacing)
            .unwrap_or_else(|| {
                estimated_epub_node_height(
                    &nodes[node_index + 1],
                    chars_per_line,
                    lines_per_page,
                    font_size,
                    line_spacing,
                )
            });
            // Default-font paragraph heights are estimated. Keep one scaled math line in reserve so
            // measurement drift cannot clip an atomic native widget at the bottom of the page.
            let fit_reserve =
                font_size * TEXT_LINE_HEIGHT * style.font_size_multiplier.unwrap_or(1.0);
            if label_height + math_height + fit_reserve > remaining
                && push_epub_page(&mut pages, budget)
            {
                remaining = (page_size.height - block_before).max(0.0);
            }
        }
        let text_len = content_node_text_len(node);
        match node {
            ContentNode::Paragraph(spans, style) => {
                let base_size = font_size * style.font_size_multiplier.unwrap_or(1.0);
                let effective_width = paragraph_width(page_size.width, font_size, style);
                let pagination_spans = pagination_inline_spans(
                    spans,
                    base_size,
                    effective_width,
                    page_size.height,
                    style.direction,
                    style.text_align,
                );
                let spans = pagination_spans.as_slice();
                let measure = |spans: &[shosai_core::epub::render::TextSpan]| {
                    measure_epub_spans(
                        fonts,
                        spans,
                        base_size,
                        effective_width,
                        style.direction,
                        style.text_align,
                    )
                };
                if !spans.iter().any(|span| span.math.is_some())
                    && fonts.is_some_and(|fonts| uses_native_fonts(fonts, spans))
                {
                    let saved_page_count = pages.len();
                    let saved_last_page_nodes = pages.last().map_or(0, Vec::len);
                    let saved_remaining = remaining;
                    let saved_page_breaks = budget.remaining_page_breaks;
                    if paginate_measured_paragraph(
                        spans,
                        style,
                        &measure,
                        text_offset,
                        block_spacing,
                        page_size.height,
                        first_page_has_title,
                        &mut pages,
                        &mut remaining,
                        budget,
                    ) {
                        text_offset += text_len + 1;
                        continue;
                    }
                    pages.truncate(saved_page_count);
                    pages
                        .last_mut()
                        .expect("EPUB pagination always retains one page")
                        .truncate(saved_last_page_nodes);
                    remaining = saved_remaining;
                    budget.remaining_page_breaks = saved_page_breaks;
                }
                let mut cursor = EpubSpanCursor::new(spans);
                let style_scale =
                    style.font_size_multiplier.unwrap_or(1.0) * spans_font_scale(spans);
                let text_line_height = font_size * TEXT_LINE_HEIGHT * style_scale;
                let paragraph_chars_per_line = (effective_width
                    / (font_size * AVERAGE_CHARACTER_WIDTH * style_scale).max(1.0))
                .floor()
                .max(1.0) as usize;
                while cursor.remaining() > 0 {
                    if budget.is_cancelled() {
                        return Vec::new();
                    }
                    let mut at_page_limit = false;
                    if remaining < text_line_height + block_spacing
                        && page_has_content(&pages, first_page_has_title)
                    {
                        if push_epub_page(&mut pages, budget) {
                            remaining = (page_size.height
                                - if cursor.consumed() == 0 {
                                    block_before
                                } else {
                                    0.0
                                })
                            .max(0.0);
                        } else {
                            at_page_limit = true;
                        }
                    }
                    let mut available_lines = ((remaining - block_spacing).max(text_line_height)
                        / text_line_height)
                        .floor()
                        .max(1.0) as usize;
                    let (take, chunk_height) = loop {
                        let available_chars = if at_page_limit {
                            cursor.remaining()
                        } else {
                            paragraph_chars_per_line * available_lines
                        };
                        let Some(take) = cursor.split_length(available_chars, budget) else {
                            return Vec::new();
                        };
                        let mut preview = cursor.clone();
                        let Some(chunk) = preview.take(take, budget) else {
                            return Vec::new();
                        };
                        let trailing_spacing = if take == cursor.remaining() {
                            block_spacing
                        } else {
                            0.0
                        };
                        let chunk_height = take.div_ceil(paragraph_chars_per_line).max(1) as f32
                            * text_line_height
                            + inline_math_height_reserve_for_context(
                                &chunk,
                                base_size,
                                effective_width,
                                page_size.height,
                                style.direction,
                                style.text_align,
                            )
                            + trailing_spacing;
                        if chunk_height <= remaining || available_lines == 1 || at_page_limit {
                            break (take, chunk_height);
                        }
                        available_lines -= 1;
                    };
                    if chunk_height > remaining
                        && page_has_content(&pages, first_page_has_title)
                        && push_epub_page(&mut pages, budget)
                    {
                        remaining = (page_size.height
                            - if cursor.consumed() == 0 {
                                block_before
                            } else {
                                0.0
                            })
                        .max(0.0);
                        continue;
                    }
                    let consumed = cursor.consumed();
                    let Some(chunk) = cursor.take(take, budget) else {
                        return Vec::new();
                    };
                    let mut fragment_style = style.clone();
                    if consumed > 0 {
                        fragment_style.block_before_em = Some(0.0);
                    }
                    if cursor.remaining() > 0 {
                        fragment_style.block_after_em = Some(0.0);
                    }
                    pages.last_mut().unwrap().push(PageNode {
                        node: ContentNode::Paragraph(chunk, fragment_style),
                        text_offset: text_offset + consumed,
                        block_before: 0.0,
                        block_after: 0.0,
                    });
                    remaining = (remaining - chunk_height).max(0.0);
                }
            }
            ContentNode::CodeBlock { code, language } => {
                let mut consumed = 0;
                let mut consumed_bytes = 0;
                let code_line_height = font_size * TEXT_LINE_HEIGHT * 0.85;
                let code_padding = 24.0;
                while consumed < text_len {
                    let mut at_page_limit = false;
                    if remaining < code_line_height + code_padding + block_spacing
                        && page_has_content(&pages, first_page_has_title)
                    {
                        if push_epub_page(&mut pages, budget) {
                            remaining = (page_size.height
                                - if consumed == 0 { block_before } else { 0.0 })
                            .max(0.0);
                        } else {
                            at_page_limit = true;
                        }
                    }
                    let available_lines = ((remaining - code_padding - block_spacing)
                        .max(code_line_height)
                        / code_line_height)
                        .floor()
                        .max(1.0) as usize;
                    let remaining_code = &code[consumed_bytes..];
                    let chunk = if at_page_limit {
                        remaining_code.to_string()
                    } else {
                        remaining_code
                            .split_inclusive('\n')
                            .take(available_lines)
                            .collect::<String>()
                    };
                    let chunk_len = chunk.chars().count();
                    consumed_bytes += chunk.len();
                    let chunk_height = chunk.lines().count().max(1) as f32 * code_line_height
                        + code_padding
                        + block_spacing;
                    pages.last_mut().unwrap().push(PageNode {
                        node: ContentNode::CodeBlock {
                            code: chunk,
                            language: language.clone(),
                        },
                        text_offset: text_offset + consumed,
                        block_before: 0.0,
                        block_after: 0.0,
                    });
                    remaining = (remaining - chunk_height).max(0.0);
                    consumed += chunk_len;
                }
            }
            ContentNode::UnorderedList(items) => {
                if !paginate_epub_list(
                    items,
                    None,
                    text_offset,
                    chars_per_line,
                    font_size,
                    line_spacing,
                    page_size.height,
                    page_size.width,
                    fonts,
                    first_page_has_title,
                    &mut pages,
                    &mut remaining,
                    budget,
                ) {
                    return Vec::new();
                }
            }
            ContentNode::OrderedList { items, start } => {
                if !paginate_epub_list(
                    items,
                    Some(*start),
                    text_offset,
                    chars_per_line,
                    font_size,
                    line_spacing,
                    page_size.height,
                    page_size.width,
                    fonts,
                    first_page_has_title,
                    &mut pages,
                    &mut remaining,
                    budget,
                ) {
                    return Vec::new();
                }
            }
            ContentNode::BlockQuote { children, style } => {
                let node_height =
                    measured_epub_compact_node_height(fonts, node, font_size, page_size.width)
                        .map(|height| height + block_spacing)
                        .unwrap_or_else(|| {
                            estimated_epub_node_height(
                                node,
                                chars_per_line,
                                lines_per_page,
                                font_size,
                                line_spacing,
                            ) - default_block_spacing
                                + block_spacing
                        });
                let follows_linked_label = nodes
                    .get(..node_index)
                    .and_then(|previous| previous.last())
                    .is_some_and(|previous| match previous {
                        ContentNode::Heading { .. } => true,
                        ContentNode::Paragraph(spans, _) => {
                            spans.iter().any(|span| span.link.is_some())
                        }
                        _ => false,
                    });
                let split_after_label = follows_linked_label
                    && node_height > remaining
                    && page_has_content(&pages, first_page_has_title);
                if node_height <= page_size.height && !split_after_label {
                    if node_height > remaining
                        && page_has_content(&pages, first_page_has_title)
                        && push_epub_page(&mut pages, budget)
                    {
                        remaining = (page_size.height - block_before).max(0.0);
                    }
                    pages.last_mut().unwrap().push(PageNode {
                        node: node.clone(),
                        text_offset,
                        block_before: 0.0,
                        block_after: 0.0,
                    });
                    remaining = (remaining - node_height).max(0.0);
                } else {
                    let available_height = (remaining - block_spacing).max(0.0);
                    let (prefix, remaining_children, prefix_height, prefix_text_len) =
                        if !page_has_content(&pages, first_page_has_title) {
                            (Vec::new(), children.to_vec(), 0.0, 0)
                        } else {
                            match split_epub_blockquote_prefix(
                                children,
                                available_height,
                                lines_per_page,
                                font_size,
                                blockquote_width(page_size.width, font_size, style),
                                fonts,
                                budget,
                            ) {
                                Some(split) => split,
                                None => return Vec::new(),
                            }
                        };
                    if !prefix.is_empty() {
                        let mut fragment_style = style.clone();
                        fragment_style.fragment_after = !remaining_children.is_empty();
                        pages.last_mut().unwrap().push(PageNode {
                            node: ContentNode::BlockQuote {
                                children: prefix,
                                style: fragment_style,
                            },
                            text_offset,
                            block_before: 0.0,
                            block_after: 0.0,
                        });
                        remaining = (remaining - prefix_height - block_spacing).max(0.0);
                    }

                    if !remaining_children.is_empty() {
                        let follows_prefix = prefix_text_len > 0;
                        if page_has_content(&pages, first_page_has_title) {
                            let _ = push_epub_page(&mut pages, budget);
                        }
                        if budget.remaining_page_breaks == 0 {
                            let mut fragment_style = style.clone();
                            fragment_style.fragment_before = follows_prefix;
                            pages.last_mut().unwrap().push(PageNode {
                                node: ContentNode::BlockQuote {
                                    children: remaining_children,
                                    style: fragment_style,
                                },
                                text_offset: text_offset + prefix_text_len,
                                block_before: 0.0,
                                block_after: 0.0,
                            });
                        } else {
                            let child_pages = paginate_epub_chapter_with_budget(
                                &remaining_children,
                                None,
                                font_size,
                                line_spacing,
                                blockquote_continuation_page_size(page_size, font_size, style),
                                fonts,
                                budget,
                            );
                            if budget.is_cancelled() {
                                return Vec::new();
                            }
                            let child_page_count = child_pages.len();
                            for (index, child_page) in child_pages.into_iter().enumerate() {
                                if index > 0 {
                                    pages.push(Vec::new());
                                }
                                let child_offset =
                                    child_page.first().map_or(0, |node| node.text_offset);
                                let mut fragment_style = style.clone();
                                fragment_style.fragment_before = follows_prefix || index > 0;
                                fragment_style.fragment_after = index + 1 < child_page_count;
                                pages.last_mut().unwrap().push(PageNode {
                                    node: ContentNode::BlockQuote {
                                        children: child_page
                                            .into_iter()
                                            .map(|node| node.node)
                                            .collect(),
                                        style: fragment_style,
                                    },
                                    text_offset: text_offset + prefix_text_len + child_offset,
                                    block_before: 0.0,
                                    block_after: 0.0,
                                });
                            }
                        }
                        remaining = 0.0;
                    }
                }
            }
            ContentNode::Figure { children, style } => {
                let figure_width = epub_figure_content_width(style, page_size.width, font_size);
                let maximum_content_height =
                    (page_size.height - block_before - block_spacing).max(1.0);
                let node_height = measured_epub_compact_node_height_bounded(
                    fonts,
                    node,
                    font_size,
                    page_size.width,
                    maximum_content_height,
                )
                .unwrap_or_else(|| {
                    estimated_epub_compact_node_height_bounded(
                        node,
                        chars_per_line,
                        lines_per_page,
                        font_size,
                        page_size.width,
                        maximum_content_height,
                        Some(page_size.height),
                    )
                }) + block_spacing;
                if node_height <= page_size.height - block_before {
                    if node_height > remaining
                        && page_has_content(&pages, first_page_has_title)
                        && push_epub_page(&mut pages, budget)
                    {
                        remaining = (page_size.height - block_before).max(0.0);
                    }
                    pages.last_mut().unwrap().push(PageNode {
                        node: node.clone(),
                        text_offset,
                        block_before: 0.0,
                        block_after: 0.0,
                    });
                    remaining = (remaining - node_height).max(0.0);
                } else {
                    let available_height = (remaining - block_spacing).max(0.0);
                    let (prefix, remaining_children, prefix_height, prefix_text_len) =
                        if !page_has_content(&pages, first_page_has_title) {
                            (Vec::new(), children.to_vec(), 0.0, 0)
                        } else {
                            match split_epub_blockquote_prefix(
                                children,
                                available_height,
                                lines_per_page,
                                font_size,
                                figure_width,
                                fonts,
                                budget,
                            ) {
                                Some(split) => split,
                                None => return Vec::new(),
                            }
                        };
                    if !prefix.is_empty() {
                        let mut fragment_style = style.clone();
                        fragment_style.fragment_after = !remaining_children.is_empty();
                        pages.last_mut().unwrap().push(PageNode {
                            node: ContentNode::Figure {
                                children: prefix,
                                style: fragment_style,
                            },
                            text_offset,
                            block_before: 0.0,
                            block_after: 0.0,
                        });
                        remaining = (remaining - prefix_height - block_spacing).max(0.0);
                    }

                    if !remaining_children.is_empty() {
                        let follows_prefix = prefix_text_len > 0;
                        if page_has_content(&pages, first_page_has_title) {
                            let _ = push_epub_page(&mut pages, budget);
                        }
                        if budget.remaining_page_breaks == 0 {
                            let mut fragment_style = style.clone();
                            fragment_style.fragment_before = follows_prefix;
                            pages.last_mut().unwrap().push(PageNode {
                                node: ContentNode::Figure {
                                    children: remaining_children,
                                    style: fragment_style,
                                },
                                text_offset: text_offset + prefix_text_len,
                                block_before: 0.0,
                                block_after: 0.0,
                            });
                        } else {
                            let child_pages = paginate_epub_chapter_with_budget(
                                &remaining_children,
                                None,
                                font_size,
                                line_spacing,
                                Size::new(figure_width, page_size.height),
                                fonts,
                                budget,
                            );
                            if budget.is_cancelled() {
                                return Vec::new();
                            }
                            let child_page_count = child_pages.len();
                            for (index, child_page) in child_pages.into_iter().enumerate() {
                                if index > 0 {
                                    pages.push(Vec::new());
                                }
                                let child_offset =
                                    child_page.first().map_or(0, |node| node.text_offset);
                                let mut fragment_style = style.clone();
                                fragment_style.fragment_before = follows_prefix || index > 0;
                                fragment_style.fragment_after = index + 1 < child_page_count;
                                pages.last_mut().unwrap().push(PageNode {
                                    node: ContentNode::Figure {
                                        children: child_page
                                            .into_iter()
                                            .map(|node| node.node)
                                            .collect(),
                                        style: fragment_style,
                                    },
                                    text_offset: text_offset + prefix_text_len + child_offset,
                                    block_before: 0.0,
                                    block_after: 0.0,
                                });
                            }
                        }
                        remaining = 0.0;
                    }
                }
            }
            ContentNode::Image { .. } => {
                let maximum_height = (page_size.height - block_before - block_spacing).max(1.0);
                if let Some((image, caption_remainder, consumed_text)) = split_epub_image_caption(
                    node,
                    font_size,
                    page_size.width,
                    maximum_height,
                    fonts,
                ) {
                    if page_has_content(&pages, first_page_has_title) {
                        let _ = push_epub_page(&mut pages, budget);
                    }
                    let caption_offset = text_offset + consumed_text;
                    pages.last_mut().unwrap().push(PageNode {
                        node: image,
                        text_offset,
                        block_before: 0.0,
                        block_after: 0.0,
                    });

                    if push_epub_page(&mut pages, budget) {
                        let caption_pages = paginate_epub_chapter_with_budget(
                            std::slice::from_ref(&caption_remainder),
                            None,
                            font_size,
                            line_spacing,
                            page_size,
                            fonts,
                            budget,
                        );
                        for (page_index, caption_page) in caption_pages.into_iter().enumerate() {
                            if page_index > 0 {
                                pages.push(Vec::new());
                            }
                            pages
                                .last_mut()
                                .unwrap()
                                .extend(caption_page.into_iter().map(|mut page_node| {
                                    page_node.text_offset += caption_offset;
                                    page_node
                                }));
                        }
                    } else {
                        pages.last_mut().unwrap().push(PageNode {
                            node: caption_remainder,
                            text_offset: caption_offset,
                            block_before: 0.0,
                            block_after: 0.0,
                        });
                    }
                    remaining = 0.0;
                    text_offset += text_len + 1;
                    continue;
                }
                let node_height = epub_image_layout(
                    node,
                    font_size,
                    page_size.width,
                    Some(page_size.height),
                    Some((page_size.height - block_before - block_spacing).max(1.0)),
                    fonts,
                )
                .map_or(page_size.height * 0.5, EpubImageLayout::total_height)
                    + block_spacing;
                if node_height > remaining
                    && page_has_content(&pages, first_page_has_title)
                    && push_epub_page(&mut pages, budget)
                {
                    remaining = (page_size.height - block_before).max(0.0);
                }
                pages.last_mut().unwrap().push(PageNode {
                    node: node.clone(),
                    text_offset,
                    block_before: 0.0,
                    block_after: 0.0,
                });
                remaining = (remaining - node_height).max(0.0);
            }
            ContentNode::Table { .. } => {
                if !paginate_epub_table(
                    node,
                    text_offset,
                    chars_per_line,
                    lines_per_page,
                    font_size,
                    page_size.width,
                    page_size.height,
                    block_before,
                    block_spacing,
                    first_page_has_title,
                    &mut pages,
                    &mut remaining,
                    fonts,
                    budget,
                ) {
                    return Vec::new();
                }
            }
            _ => {
                let node_height = measured_epub_compact_node_height_bounded(
                    fonts,
                    node,
                    font_size,
                    page_size.width,
                    page_size.height,
                )
                .map(|height| height + block_spacing)
                .unwrap_or_else(|| {
                    estimated_epub_node_height(
                        node,
                        chars_per_line,
                        lines_per_page,
                        font_size,
                        line_spacing,
                    ) - default_block_spacing
                        + block_spacing
                });
                if node_height > remaining
                    && page_has_content(&pages, first_page_has_title)
                    && push_epub_page(&mut pages, budget)
                {
                    remaining = (page_size.height - block_before).max(0.0);
                }
                pages.last_mut().unwrap().push(PageNode {
                    node: node.clone(),
                    text_offset,
                    block_before: 0.0,
                    block_after: 0.0,
                });
                remaining = (remaining - node_height).max(0.0);
            }
        }
        text_offset += text_len + 1;
    }

    if pages.len() > 1 && pages.last().is_some_and(Vec::is_empty) {
        pages.pop();
    }
    if budget.is_cancelled()
        || !assign_paginated_block_geometry(
            nodes,
            font_size,
            default_block_spacing,
            &mut pages,
            budget,
        )
    {
        return Vec::new();
    }
    pages
}

pub fn paginate_document(
    document: &EpubDoc,
    font_size: f32,
    line_spacing: f32,
    page_size: LayoutSize,
) -> Vec<Page> {
    let mut pages = Vec::new();
    let chapters = document.presentation().chapters();
    let mut budget = EpubPaginationBudget::for_document(chapters.len());
    for chapter in 0..chapters.len() {
        if pages.len() >= MAX_EPUB_PAGES {
            break;
        }
        pages.extend(paginate_document_chapter(
            document,
            chapter,
            font_size,
            line_spacing,
            page_size,
            &mut budget,
        ));
    }
    pages
}

pub fn paginate_document_cancellable(
    document: &EpubDoc,
    font_size: f32,
    line_spacing: f32,
    page_size: LayoutSize,
    cancellation: EpubPaginationCancellation,
) -> Vec<Page> {
    let mut pages = Vec::new();
    let chapters = document.presentation().chapters();
    let mut budget =
        EpubPaginationBudget::for_document(chapters.len()).with_cancellation(cancellation);
    for chapter in 0..chapters.len() {
        if budget.is_cancelled() || pages.len() >= MAX_EPUB_PAGES {
            break;
        }
        pages.extend(paginate_document_chapter(
            document,
            chapter,
            font_size,
            line_spacing,
            page_size,
            &mut budget,
        ));
    }
    pages
}

pub fn paginate_document_chapter(
    document: &EpubDoc,
    chapter: usize,
    font_size: f32,
    line_spacing: f32,
    page_size: LayoutSize,
    budget: &mut EpubPaginationBudget,
) -> Vec<Page> {
    let Some(presentation) = document.presentation().chapter(chapter) else {
        return Vec::new();
    };
    let nodes = presentation.nodes();
    let source = document
        .chapter(chapter)
        .expect("presentation chapters match source chapters");
    let title = source
        .title
        .as_deref()
        .filter(|title| !content_starts_with_heading(nodes, title));
    paginate_epub_chapter_with_budget(
        nodes,
        title,
        font_size,
        line_spacing,
        page_size,
        Some(document.fonts()),
        budget,
    )
    .into_iter()
    .enumerate()
    .map(|(page, nodes)| Page {
        chapter,
        title: (page == 0).then(|| title.map(str::to_owned)).flatten(),
        nodes,
    })
    .collect()
}

/// Assign each original boundary exactly once. A boundary whose two nodes land
/// on different pages is truncated, matching CSS fragmentation rather than
/// duplicating the margin at either page edge. Split-node internal boundaries
/// always remain zero.
fn assign_paginated_block_geometry(
    nodes: &[ContentNode],
    font_size: f32,
    default_spacing: f32,
    pages: &mut [PageNodes],
    budget: &EpubPaginationBudget,
) -> bool {
    let mut starts = Vec::with_capacity(nodes.len());
    let mut offset = 0;
    for (index, node) in nodes.iter().enumerate() {
        if index % EPUB_PAGINATION_LOOP_CHUNK == 0 && budget.is_cancelled() {
            return false;
        }
        starts.push(offset);
        offset += content_node_text_len(node) + 1;
    }
    let mut fragments = vec![Vec::<(usize, usize)>::new(); nodes.len()];
    for (page_index, page) in pages.iter_mut().enumerate() {
        for (fragment_index, fragment) in page.iter_mut().enumerate() {
            if fragment_index % EPUB_PAGINATION_LOOP_CHUNK == 0 && budget.is_cancelled() {
                return false;
            }
            fragment.block_before = 0.0;
            fragment.block_after = 0.0;
            let original = starts
                .partition_point(|start| *start <= fragment.text_offset)
                .saturating_sub(1)
                .min(nodes.len().saturating_sub(1));
            if let Some(entries) = fragments.get_mut(original) {
                entries.push((page_index, fragment_index));
            }
        }
    }
    if let Some(&(page, fragment)) = fragments.first().and_then(|items| items.first()) {
        pages[page][fragment].block_before =
            epub_node_boundary_spacing(nodes, 0, font_size, default_spacing);
    }
    for boundary in 1..nodes.len() {
        if boundary % EPUB_PAGINATION_LOOP_CHUNK == 0 && budget.is_cancelled() {
            return false;
        }
        let Some(&(left_page, left_fragment)) = fragments[boundary - 1].last() else {
            continue;
        };
        let Some(&(right_page, _)) = fragments[boundary].first() else {
            continue;
        };
        if left_page == right_page {
            pages[left_page][left_fragment].block_after =
                epub_node_boundary_spacing(nodes, boundary, font_size, default_spacing);
        }
    }
    if let Some(&(page, fragment)) = fragments.last().and_then(|items| items.last()) {
        pages[page][fragment].block_after =
            epub_node_boundary_spacing(nodes, nodes.len(), font_size, default_spacing);
    }
    !budget.is_cancelled()
}

fn scaled_characters_per_line(chars_per_line: usize, scale: f32) -> usize {
    ((chars_per_line as f32 / scale.max(0.1)).floor() as usize).max(1)
}

fn push_epub_page(pages: &mut Vec<PageNodes>, budget: &mut EpubPaginationBudget) -> bool {
    if budget.is_cancelled() || budget.remaining_page_breaks == 0 {
        return false;
    }
    budget.remaining_page_breaks -= 1;
    pages.push(Vec::new());
    true
}

fn page_has_content(pages: &[PageNodes], first_page_has_title: bool) -> bool {
    pages.last().is_some_and(|page| !page.is_empty()) || (first_page_has_title && pages.len() == 1)
}

#[allow(clippy::too_many_arguments)]
fn paginate_epub_table(
    table: &ContentNode,
    text_offset: usize,
    chars_per_line: usize,
    lines_per_page: usize,
    font_size: f32,
    page_width: f32,
    page_height: f32,
    leading_spacing: f32,
    trailing_spacing: f32,
    first_page_has_title: bool,
    pages: &mut Vec<PageNodes>,
    remaining: &mut f32,
    fonts: Option<&EpubFontBook>,
    budget: &mut EpubPaginationBudget,
) -> bool {
    let ContentNode::Table {
        caption,
        caption_style,
        row_groups,
        style,
    } = table
    else {
        unreachable!("table pagination requires a table node");
    };
    if row_groups.is_empty() {
        if caption.is_empty() {
            return true;
        }
        if page_has_content(pages, first_page_has_title) && push_epub_page(pages, budget) {
            *remaining = (page_height - leading_spacing).max(0.0);
        }
        let mut caption_fragment_style = caption_style.clone().unwrap_or_default();
        caption_fragment_style.block_before_em = Some(0.0);
        caption_fragment_style.block_after_em = Some(0.0);
        let table_width = epub_table_layout_width(row_groups, style, page_width);
        let content_width = epub_table_content_width(style, table_width, page_width, font_size);
        caption_fragment_style.width =
            Some(shosai_core::epub::render::NodeWidth::Pixels(content_width));
        caption_fragment_style.margin_left_em = Some(
            epub_table_margin_left(style, font_size, page_width, table_width, content_width)
                / font_size,
        );
        let caption_pages = paginate_epub_chapter_with_budget(
            &[ContentNode::Paragraph(
                caption.clone(),
                caption_fragment_style,
            )],
            None,
            font_size,
            0.0,
            Size::new(page_width, (page_height - leading_spacing).max(1.0)),
            fonts,
            budget,
        );
        for (page_index, caption_page) in caption_pages.into_iter().enumerate() {
            if page_index > 0 {
                pages.push(Vec::new());
            }
            pages
                .last_mut()
                .unwrap()
                .extend(caption_page.into_iter().map(|mut page_node| {
                    page_node.text_offset += text_offset;
                    page_node
                }));
        }
        *remaining = 0.0;
        return !budget.is_cancelled();
    }
    let mut fragment_offset = text_offset;
    let mut include_caption = !caption.is_empty();
    let mut pending = None;
    let mut pending_height = 0.0;
    let mut pending_source_group = None;
    let mut fragment_capacity = *remaining;
    let mut page_budget_exhausted = false;
    let mut bands = Vec::new();
    for (group_index, group) in row_groups.iter().enumerate() {
        let Some(group_bands) = table_row_bands(&group.rows, budget) else {
            return false;
        };
        bands.extend(
            group_bands
                .into_iter()
                .map(|rows| (group_index, group.kind, rows)),
        );
    }
    let cancellation = budget.cancellation.clone();
    let compact_height = |node: &ContentNode, maximum_height: f32| -> Option<f32> {
        let ContentNode::Table {
            caption,
            caption_style,
            row_groups,
            style,
        } = node
        else {
            return Some(estimated_epub_compact_node_height_bounded(
                node,
                chars_per_line,
                lines_per_page,
                font_size,
                page_width,
                maximum_height,
                Some(page_height),
            ));
        };
        let table_width = epub_table_layout_width(row_groups, style, page_width);
        let content_width = epub_table_content_width(style, table_width, page_width, font_size);
        let placements = epub_table_cell_placements_cancellable(row_groups, cancellation.as_ref())?;
        let column_widths = epub_table_column_widths_from_placements_cancellable(
            row_groups,
            content_width,
            &placements,
            cancellation.as_ref(),
        )?;
        let caption_height = epub_table_caption_height(
            fonts,
            caption,
            caption_style.as_ref(),
            font_size,
            content_width,
            maximum_height,
        );
        let caption_gap = EPUB_TABLE_ROW_SPACING
            * usize::from(!caption.is_empty() && !row_groups.is_empty()) as f32;
        let geometry = epub_table_geometry_bounded_from_placements_cancellable(
            row_groups,
            &placements,
            &column_widths,
            lines_per_page,
            font_size,
            (maximum_height - caption_height - caption_gap).max(1.0),
            fonts,
            cancellation.as_ref(),
        );
        Some(caption_height + geometry?.height + caption_gap)
    };

    if !caption.is_empty() {
        let table_width = epub_table_layout_width(row_groups, style, page_width);
        let content_width = epub_table_content_width(style, table_width, page_width, font_size);
        let maximum_height = (page_height - leading_spacing - trailing_spacing).max(1.0);
        let minimum_caption_height = font_size
            * caption_style
                .as_ref()
                .and_then(|style| style.font_size_multiplier)
                .unwrap_or(1.0)
            * spans_font_scale(caption)
            * TEXT_LINE_HEIGHT;
        let row_height = if let Some((_, kind, rows)) = bands.first().copied() {
            let row = ContentNode::Table {
                caption: Vec::new(),
                caption_style: None,
                row_groups: vec![TableRowGroup {
                    kind,
                    rows: rows.to_vec(),
                }],
                style: style.clone(),
            };
            let Some(height) = compact_height(
                &row,
                (maximum_height - minimum_caption_height - EPUB_TABLE_ROW_SPACING).max(1.0),
            ) else {
                return false;
            };
            height
        } else {
            0.0
        };
        let caption_gap = EPUB_TABLE_ROW_SPACING * usize::from(!bands.is_empty()) as f32;
        let maximum_caption_height = (maximum_height - row_height - caption_gap).max(1.0);
        if let Some((caption_prefix, caption_suffix)) = split_epub_caption_suffix(
            caption,
            caption_style.as_ref(),
            font_size,
            content_width,
            maximum_caption_height,
            fonts,
        ) {
            if page_has_content(pages, first_page_has_title) && push_epub_page(pages, budget) {
                *remaining = (page_height - leading_spacing).max(0.0);
            }
            let mut prefix_style = caption_style.clone().unwrap_or_default();
            prefix_style.block_before_em = Some(0.0);
            prefix_style.block_after_em = Some(0.0);
            prefix_style.width = Some(shosai_core::epub::render::NodeWidth::Pixels(content_width));
            prefix_style.margin_left_em = Some(
                epub_table_margin_left(style, font_size, page_width, table_width, content_width)
                    / font_size,
            );
            let prefix = ContentNode::Paragraph(caption_prefix.clone(), prefix_style);
            let prefix_pages = paginate_epub_chapter_with_budget(
                std::slice::from_ref(&prefix),
                None,
                font_size,
                0.0,
                Size::new(page_width, (page_height - leading_spacing).max(1.0)),
                fonts,
                budget,
            );
            for (page_index, prefix_page) in prefix_pages.into_iter().enumerate() {
                if page_index > 0 {
                    pages.push(Vec::new());
                }
                pages
                    .last_mut()
                    .unwrap()
                    .extend(prefix_page.into_iter().map(|mut page_node| {
                        page_node.text_offset += text_offset;
                        page_node
                    }));
            }
            *remaining = 0.0;
            if push_epub_page(pages, budget) {
                *remaining = page_height;
            }
            let mut suffix_table = table.clone();
            let ContentNode::Table { caption, .. } = &mut suffix_table else {
                unreachable!();
            };
            *caption = caption_suffix;
            let suffix_offset = text_offset
                + spans_text_len(&caption_prefix)
                + usize::from(caption.is_empty() && !row_groups.is_empty());
            let _ = paginate_epub_table(
                &suffix_table,
                suffix_offset,
                chars_per_line,
                lines_per_page,
                font_size,
                page_width,
                page_height,
                0.0,
                trailing_spacing,
                false,
                pages,
                remaining,
                fonts,
                budget,
            );
            return !budget.is_cancelled();
        }
    }

    for (band_index, (group_index, kind, rows)) in bands.iter().copied().enumerate() {
        if budget.is_cancelled() {
            return false;
        }
        let is_final_band = band_index + 1 == bands.len();
        let band = ContentNode::Table {
            caption: if include_caption {
                caption.clone()
            } else {
                Vec::new()
            },
            caption_style: if include_caption {
                caption_style.clone()
            } else {
                None
            },
            row_groups: vec![TableRowGroup {
                kind,
                rows: rows.to_vec(),
            }],
            style: style.clone(),
        };
        include_caption = false;

        if page_budget_exhausted {
            append_table_band(
                pending
                    .as_mut()
                    .expect("exhausted table pagination retains a pending fragment"),
                kind,
                rows,
                pending_source_group == Some(group_index),
            );
            pending_height = fragment_capacity;
            pending_source_group = Some(group_index);
            continue;
        }

        let mut candidate = pending.clone().unwrap_or_else(|| band.clone());
        if pending.is_some() {
            append_table_band(
                &mut candidate,
                kind,
                rows,
                pending_source_group == Some(group_index),
            );
        }
        let candidate_maximum_height = (page_height
            - if fragment_offset == text_offset {
                leading_spacing
            } else {
                0.0
            }
            - if is_final_band { trailing_spacing } else { 0.0 })
        .max(1.0);
        let Some(candidate_height) = compact_height(&candidate, candidate_maximum_height) else {
            return false;
        };
        let required_height = candidate_height + if is_final_band { trailing_spacing } else { 0.0 };

        if pending.is_some() && required_height > fragment_capacity && !page_budget_exhausted {
            if budget.remaining_page_breaks == 0 {
                page_budget_exhausted = true;
                pending = Some(candidate);
                pending_height = fragment_capacity;
                pending_source_group = Some(group_index);
                continue;
            }
            let fragment = pending.take().expect("pending table fragment must exist");
            let fragment_len = content_node_text_len(&fragment);
            pages.last_mut().unwrap().push(PageNode {
                node: fragment,
                text_offset: fragment_offset,
                block_before: 0.0,
                block_after: 0.0,
            });
            fragment_offset += fragment_len;
            *remaining = (fragment_capacity - pending_height).max(0.0);
            let _ = push_epub_page(pages, budget);
            *remaining = page_height;
            fragment_capacity = *remaining;
            let band_maximum_height =
                (page_height - if is_final_band { trailing_spacing } else { 0.0 }).max(1.0);
            let Some(height) = compact_height(&band, band_maximum_height) else {
                return false;
            };
            pending_height = height;
            pending = Some(band);
            pending_source_group = Some(group_index);
            continue;
        }

        if pending.is_none()
            && required_height > *remaining
            && page_has_content(pages, first_page_has_title)
            && push_epub_page(pages, budget)
        {
            *remaining = (page_height - leading_spacing).max(0.0);
            fragment_capacity = *remaining;
        }
        pending = Some(candidate);
        pending_height = candidate_height;
        pending_source_group = Some(group_index);
    }

    if let Some(fragment) = pending {
        pages.last_mut().unwrap().push(PageNode {
            node: fragment,
            text_offset: fragment_offset,
            block_before: 0.0,
            block_after: trailing_spacing,
        });
        *remaining = (fragment_capacity - pending_height - trailing_spacing).max(0.0);
    } else if !caption.is_empty() {
        let maximum_height = (page_height - leading_spacing - trailing_spacing).max(1.0);
        let Some(compact_height) = compact_height(table, maximum_height) else {
            return false;
        };
        let height = compact_height + trailing_spacing;
        if height > *remaining
            && page_has_content(pages, first_page_has_title)
            && push_epub_page(pages, budget)
        {
            *remaining = (page_height - leading_spacing).max(0.0);
        }
        pages.last_mut().unwrap().push(PageNode {
            node: table.clone(),
            text_offset,
            block_before: 0.0,
            block_after: trailing_spacing,
        });
        *remaining = (*remaining - height).max(0.0);
    }
    !budget.is_cancelled()
}

fn append_table_band(
    fragment: &mut ContentNode,
    kind: shosai_core::epub::render::TableRowGroupKind,
    rows: &[TableRow],
    same_source_group: bool,
) {
    let ContentNode::Table { row_groups, .. } = fragment else {
        unreachable!("table bands can only be appended to table fragments");
    };
    if let Some(group) = row_groups
        .last_mut()
        .filter(|group| same_source_group && group.kind == kind)
    {
        group.rows.extend_from_slice(rows);
    } else {
        row_groups.push(TableRowGroup {
            kind,
            rows: rows.to_vec(),
        });
    }
}

fn split_epub_caption_suffix(
    caption: &[shosai_core::epub::render::TextSpan],
    style: Option<&shosai_core::epub::render::NodeStyle>,
    font_size: f32,
    width: f32,
    maximum_height: f32,
    fonts: Option<&EpubFontBook>,
) -> Option<(
    Vec<shosai_core::epub::render::TextSpan>,
    Vec<shosai_core::epub::render::TextSpan>,
)> {
    let length = spans_text_len(caption);
    if length == 0
        || epub_table_caption_height(fonts, caption, style, font_size, width, maximum_height)
            <= maximum_height
    {
        return None;
    }
    let mut low = 1;
    let mut high = length;
    let mut split = None;
    while low < high {
        let start = low + (high - low) / 2;
        let suffix = slice_epub_spans(caption, start, length - start);
        if epub_table_caption_height(fonts, &suffix, style, font_size, width, maximum_height)
            <= maximum_height
        {
            split = Some(start);
            high = start;
        } else {
            low = start + 1;
        }
    }
    let start = split
        .or_else(|| {
            let start = length - 1;
            let suffix = slice_epub_spans(caption, start, 1);
            (epub_table_caption_height(fonts, &suffix, style, font_size, width, maximum_height)
                <= maximum_height)
                .then_some(start)
        })
        .unwrap_or(length);
    Some((
        slice_epub_spans(caption, 0, start),
        slice_epub_spans(caption, start, length - start),
    ))
}

fn table_row_bands<'a>(
    rows: &'a [TableRow],
    budget: &EpubPaginationBudget,
) -> Option<Vec<&'a [TableRow]>> {
    let mut bands = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        if budget.is_cancelled() {
            return None;
        }
        let mut end = start + 1;
        let mut row_index = start;
        while row_index < end {
            for (cell_index, cell) in rows[row_index].cells.iter().enumerate() {
                if cell_index % EPUB_PAGINATION_LOOP_CHUNK == 0 && budget.is_cancelled() {
                    return None;
                }
                let span = if cell.row_span == 0 {
                    rows.len() - row_index
                } else {
                    usize::from(cell.row_span)
                };
                end = end.max((row_index + span).min(rows.len()));
            }
            row_index += 1;
        }
        bands.push(&rows[start..end]);
        start = end;
    }
    Some(bands)
}

pub fn epub_table_layout_width(
    row_groups: &[TableRowGroup],
    style: &shosai_core::epub::render::NodeStyle,
    available_width: f32,
) -> f32 {
    let columns = epub_table_column_count(row_groups);
    let preferred = match style.width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => value * available_width,
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value,
        None => (columns as f32 * MIN_EPUB_TABLE_CELL_WIDTH)
            .max(MIN_EPUB_TABLE_WIDTH)
            .max(available_width),
    };
    let maximum = match style.max_width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => value * available_width,
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value,
        None => MAX_EPUB_TABLE_WIDTH,
    };
    preferred.min(maximum).clamp(1.0, MAX_EPUB_TABLE_WIDTH)
}

pub fn epub_figure_content_width(
    style: &shosai_core::epub::render::NodeStyle,
    available_width: f32,
    font_size: f32,
) -> f32 {
    let width = match style.width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => value * available_width,
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value,
        None => available_width,
    };
    let maximum = match style.max_width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => value * available_width,
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value,
        None => available_width,
    };
    width
        .min(maximum)
        .min((available_width - style.margin_left_em.unwrap_or(0.0).max(0.0) * font_size).max(1.0))
        .max(1.0)
}

pub fn epub_figure_margin_left(
    style: &shosai_core::epub::render::NodeStyle,
    available_width: f32,
    font_size: f32,
    figure_width: f32,
) -> f32 {
    (style.margin_left_em.unwrap_or(0.0).max(0.0) * font_size)
        .min((available_width - figure_width).max(0.0))
}

pub fn epub_table_content_width(
    style: &shosai_core::epub::render::NodeStyle,
    table_width: f32,
    available_width: f32,
    font_size: f32,
) -> f32 {
    if style.width.is_some() {
        table_width
            .min(
                (available_width - style.margin_left_em.unwrap_or(0.0).max(0.0) * font_size)
                    .max(1.0),
            )
            .max(1.0)
    } else {
        let margin = style.margin_left_em.unwrap_or(0.0).max(0.0) * font_size;
        (table_width - margin.min((table_width - 1.0).max(0.0))).max(1.0)
    }
}

pub fn epub_table_margin_left(
    style: &shosai_core::epub::render::NodeStyle,
    font_size: f32,
    available_width: f32,
    table_width: f32,
    content_width: f32,
) -> f32 {
    let margin = style.margin_left_em.unwrap_or(0.0).max(0.0) * font_size;
    if style.width.is_some() {
        margin.min((available_width - content_width).max(0.0))
    } else {
        margin.min((table_width - content_width).max(0.0))
    }
}

fn epub_table_column_count(row_groups: &[TableRowGroup]) -> usize {
    epub_table_column_count_from_placements(&epub_table_cell_placements(row_groups))
}

fn epub_table_column_count_from_placements(placements: &[Vec<EpubTableCellPlacement>]) -> usize {
    placements
        .iter()
        .flatten()
        .map(|placement| placement.column + placement.span)
        .max()
        .unwrap_or(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpubTableCellPlacement {
    pub column: usize,
    pub span: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EpubTableCellGeometry {
    pub placement: EpubTableCellPlacement,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EpubTableGeometry {
    pub row_heights: Vec<f32>,
    pub cells: Vec<Vec<EpubTableCellGeometry>>,
    pub height: f32,
}

/// Fenwick trees supporting ordered range additions and range sums in
/// logarithmic time. Rowspan sizing uses both operations, so plain prefix
/// sums would need rebuilding after every spanning cell.
struct EpubTableRowHeights {
    slope: Vec<f32>,
    intercept: Vec<f32>,
}

impl EpubTableRowHeights {
    fn new(len: usize, initial: f32) -> Self {
        let mut heights = Self {
            slope: vec![0.0; len + 1],
            intercept: vec![0.0; len + 1],
        };
        heights.add_range(0, len, initial);
        heights
    }

    fn add(tree: &mut [f32], mut index: usize, value: f32) {
        while index < tree.len() {
            tree[index] += value;
            index += index & index.wrapping_neg();
        }
    }

    fn sum(tree: &[f32], mut end: usize) -> f32 {
        let mut total = 0.0;
        while end > 0 {
            total += tree[end];
            end &= end - 1;
        }
        total
    }

    fn prefix_sum(&self, end: usize) -> f32 {
        Self::sum(&self.slope, end) * end as f32 - Self::sum(&self.intercept, end)
    }

    fn range_sum(&self, start: usize, end: usize) -> f32 {
        self.prefix_sum(end) - self.prefix_sum(start)
    }

    fn add_range(&mut self, start: usize, end: usize, value: f32) {
        if start == end {
            return;
        }
        Self::add(&mut self.slope, start + 1, value);
        Self::add(&mut self.slope, end + 1, -value);
        Self::add(&mut self.intercept, start + 1, value * start as f32);
        Self::add(&mut self.intercept, end + 1, -value * end as f32);
    }

    fn into_values_cancellable(
        self,
        cancellation: Option<&EpubPaginationCancellation>,
    ) -> Option<Vec<f32>> {
        let mut values = Vec::with_capacity(self.slope.len() - 1);
        for row in 0..self.slope.len() - 1 {
            if !table_row_checkpoint(cancellation, row) {
                return None;
            }
            values.push(self.range_sum(row, row + 1));
        }
        Some(values)
    }
}

/// Builds the logical grid used by both measurement and painting. Row spans are
/// scoped to their row group, as required by the table model's group semantics.
pub fn epub_table_cell_placements(
    row_groups: &[TableRowGroup],
) -> Vec<Vec<EpubTableCellPlacement>> {
    epub_table_cell_placements_cancellable(row_groups, None)
        .expect("the no-cancellation table placement path cannot be interrupted")
}

fn epub_table_cell_placements_cancellable(
    row_groups: &[TableRowGroup],
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<Vec<Vec<EpubTableCellPlacement>>> {
    #[cfg(test)]
    TABLE_PLACEMENT_PASSES.with(|passes| passes.set(passes.get() + 1));
    let mut placements = Vec::new();
    let mut cell_index = 0;
    let mut global_row = 0;
    for group in row_groups {
        let mut occupied_until = vec![0_usize; MAX_EPUB_TABLE_COLUMNS];
        for (row_index, row) in group.rows.iter().enumerate() {
            if !table_row_checkpoint(cancellation, global_row) {
                return None;
            }
            global_row += 1;
            let mut row_placements = Vec::with_capacity(row.cells.len());
            let mut column = 0_usize;
            for cell in &row.cells {
                if !table_cell_checkpoint(cancellation, cell_index) {
                    return None;
                }
                cell_index += 1;
                let requested_span = usize::from(cell.column_span.max(1));
                let span = requested_span.min(MAX_EPUB_TABLE_COLUMNS);
                while column < MAX_EPUB_TABLE_COLUMNS
                    && (column..column.saturating_add(span).min(MAX_EPUB_TABLE_COLUMNS))
                        .any(|slot| occupied_until[slot] > row_index)
                {
                    column += 1;
                }
                // Malicious aggregate spans can exhaust the bounded grid. Clamp
                // deterministically to its final slot instead of growing storage.
                column = column.min(MAX_EPUB_TABLE_COLUMNS - 1);
                let span = span.min(MAX_EPUB_TABLE_COLUMNS - column).max(1);
                let row_end = if cell.row_span == 0 {
                    group.rows.len()
                } else {
                    row_index
                        .saturating_add(usize::from(cell.row_span))
                        .min(group.rows.len())
                };
                occupied_until[column..column + span].fill(row_end);
                row_placements.push(EpubTableCellPlacement { column, span });
                column += span;
            }
            placements.push(row_placements);
        }
    }
    cancellation
        .is_none_or(|cancellation| !cancellation.is_cancelled())
        .then_some(placements)
}

/// Measures the complete logical table once. Pagination and painting provide
/// the same intrinsic-cell measurer and consume these row and cell rectangles.
#[cfg(test)]
pub fn epub_table_geometry(
    row_groups: &[TableRowGroup],
    column_widths: &[f32],
    measure_cell: impl FnMut(&shosai_core::epub::render::TableCell, f32) -> f32,
) -> EpubTableGeometry {
    let placements = epub_table_cell_placements(row_groups);
    epub_table_geometry_from_placements(row_groups, &placements, column_widths, measure_cell)
}

pub fn epub_table_geometry_from_placements(
    row_groups: &[TableRowGroup],
    placements: &[Vec<EpubTableCellPlacement>],
    column_widths: &[f32],
    measure_cell: impl FnMut(&shosai_core::epub::render::TableCell, f32) -> f32,
) -> EpubTableGeometry {
    let mut measure_cell = measure_cell;
    epub_table_geometry_from_placements_cancellable(
        row_groups,
        placements,
        column_widths,
        |cell, width| Some(measure_cell(cell, width)),
        None,
    )
    .expect("the no-cancellation table geometry path cannot be interrupted")
}

fn epub_table_geometry_from_placements_cancellable(
    row_groups: &[TableRowGroup],
    placements: &[Vec<EpubTableCellPlacement>],
    column_widths: &[f32],
    mut measure_cell: impl FnMut(&shosai_core::epub::render::TableCell, f32) -> Option<f32>,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<EpubTableGeometry> {
    let rows = row_groups
        .iter()
        .flat_map(|group| &group.rows)
        .collect::<Vec<_>>();
    let mut row_heights = EpubTableRowHeights::new(rows.len(), 2.0 * EPUB_TABLE_CELL_PADDING);
    let mut intrinsic = Vec::with_capacity(rows.len());
    let mut global_row = 0;
    let mut cell_index = 0;
    for group in row_groups {
        for (group_row, row) in group.rows.iter().enumerate() {
            if !table_row_checkpoint(cancellation, global_row) {
                return None;
            }
            let mut measured = Vec::with_capacity(row.cells.len());
            for (cell, placement) in row.cells.iter().zip(&placements[global_row]) {
                if !table_cell_checkpoint(cancellation, cell_index) {
                    return None;
                }
                cell_index += 1;
                let height = measure_cell(
                    cell,
                    epub_table_cell_content_width(*placement, column_widths),
                )?;
                let span = if cell.row_span == 0 {
                    group.rows.len() - group_row
                } else {
                    usize::from(cell.row_span)
                }
                .min(group.rows.len() - group_row)
                .max(1);
                measured.push((height, span));
            }
            for &(height, span) in &measured {
                if span == 1 {
                    let current = row_heights.range_sum(global_row, global_row + 1);
                    row_heights.add_range(global_row, global_row + 1, (height - current).max(0.0));
                }
            }
            intrinsic.push(measured);
            global_row += 1;
        }
    }
    for (row, measured) in intrinsic.iter().enumerate() {
        if !table_row_checkpoint(cancellation, row) {
            return None;
        }
        for (index, &(height, span)) in measured.iter().enumerate() {
            if !table_row_checkpoint(cancellation, index) {
                return None;
            }
            if span > 1 {
                let current = row_heights.range_sum(row, row + span)
                    + EPUB_TABLE_ROW_SPACING * span.saturating_sub(1) as f32;
                let deficit = (height - current).max(0.0) / span as f32;
                row_heights.add_range(row, row + span, deficit);
            }
        }
    }
    let row_heights = row_heights.into_values_cancellable(cancellation)?;
    let mut y = 0.0;
    let mut row_y = Vec::with_capacity(row_heights.len());
    let mut row_height_prefix = Vec::with_capacity(row_heights.len() + 1);
    row_height_prefix.push(0.0);
    for (row, height) in row_heights.iter().enumerate() {
        if !table_row_checkpoint(cancellation, row) {
            return None;
        }
        row_y.push(y);
        y += *height + EPUB_TABLE_ROW_SPACING;
        row_height_prefix.push(row_height_prefix[row] + height);
    }
    let mut column_width_prefix = Vec::with_capacity(column_widths.len() + 1);
    column_width_prefix.push(0.0);
    for (column, width) in column_widths.iter().enumerate() {
        if !table_row_checkpoint(cancellation, column) {
            return None;
        }
        column_width_prefix.push(column_width_prefix[column] + width);
    }
    let mut cells = Vec::with_capacity(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        if !table_row_checkpoint(cancellation, row_index) {
            return None;
        }
        let mut row_cells = Vec::with_capacity(row.cells.len());
        for ((_, placement), &(_, span)) in row
            .cells
            .iter()
            .zip(&placements[row_index])
            .zip(&intrinsic[row_index])
        {
            if !table_cell_checkpoint(cancellation, cell_index) {
                return None;
            }
            cell_index += 1;
            let x = column_width_prefix[placement.column]
                + BLOCKQUOTE_SPACING * placement.column as f32;
            let height = row_height_prefix[row_index + span] - row_height_prefix[row_index]
                + EPUB_TABLE_ROW_SPACING * span.saturating_sub(1) as f32;
            row_cells.push(EpubTableCellGeometry {
                placement: *placement,
                x,
                y: row_y[row_index],
                width: epub_table_cell_width(*placement, column_widths),
                height,
            });
        }
        cells.push(row_cells);
    }
    Some(EpubTableGeometry {
        row_heights,
        cells,
        height: y - if rows.is_empty() {
            0.0
        } else {
            EPUB_TABLE_ROW_SPACING
        },
    })
}

#[cfg(test)]
pub fn epub_table_geometry_bounded(
    row_groups: &[TableRowGroup],
    column_widths: &[f32],
    lines_per_page: usize,
    font_size: f32,
    height: f32,
    fonts: Option<&EpubFontBook>,
) -> EpubTableGeometry {
    let placements = epub_table_cell_placements(row_groups);
    epub_table_geometry_bounded_from_placements(
        row_groups,
        &placements,
        column_widths,
        lines_per_page,
        font_size,
        height,
        fonts,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn epub_table_geometry_bounded_from_placements(
    row_groups: &[TableRowGroup],
    placements: &[Vec<EpubTableCellPlacement>],
    column_widths: &[f32],
    lines_per_page: usize,
    font_size: f32,
    height: f32,
    fonts: Option<&EpubFontBook>,
) -> EpubTableGeometry {
    epub_table_geometry_bounded_from_placements_cancellable(
        row_groups,
        placements,
        column_widths,
        lines_per_page,
        font_size,
        height,
        fonts,
        None,
    )
    .expect("the no-cancellation bounded table geometry path cannot be interrupted")
}

#[allow(clippy::too_many_arguments)]
fn epub_table_geometry_bounded_from_placements_cancellable(
    row_groups: &[TableRowGroup],
    placements: &[Vec<EpubTableCellPlacement>],
    column_widths: &[f32],
    lines_per_page: usize,
    font_size: f32,
    height: f32,
    fonts: Option<&EpubFontBook>,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<EpubTableGeometry> {
    epub_table_geometry_from_placements_cancellable(
        row_groups,
        placements,
        column_widths,
        |cell, cell_width| {
            if cancellation.is_some_and(EpubPaginationCancellation::is_cancelled) {
                return None;
            }
            let chars_per_line = (cell_width / (font_size * AVERAGE_CHARACTER_WIDTH).max(1.0))
                .floor()
                .max(1.0) as usize;
            let spacing = epub_node_list_spacing_cancellable(
                &cell.children,
                font_size,
                EPUB_TABLE_CELL_SPACING,
                cancellation,
            )?;
            let mut remaining_height = (height - spacing - 2.0 * EPUB_TABLE_CELL_PADDING).max(1.0);
            let mut content_height = 0.0;
            for (child_index, child) in cell.children.iter().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, child_index) {
                    return None;
                }
                let child_height = epub_bounded_node_height_cancellable(
                    fonts,
                    child,
                    font_size,
                    cell_width,
                    remaining_height,
                    chars_per_line,
                    lines_per_page,
                    cancellation,
                )?;
                remaining_height = (remaining_height - child_height).max(1.0);
                content_height += child_height;
            }
            Some(content_height + spacing + 2.0 * EPUB_TABLE_CELL_PADDING)
        },
        cancellation,
    )
}

#[allow(clippy::too_many_arguments)]
fn epub_bounded_node_height_cancellable(
    fonts: Option<&EpubFontBook>,
    node: &ContentNode,
    font_size: f32,
    width: f32,
    height: f32,
    chars_per_line: usize,
    lines_per_page: usize,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<f32> {
    content_node_internal_checkpoints(node, cancellation)?;
    Some(epub_bounded_node_height(
        fonts,
        node,
        font_size,
        width,
        height,
        chars_per_line,
        lines_per_page,
    ))
}

fn spans_internal_checkpoints(
    spans: &[shosai_core::epub::render::TextSpan],
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<()> {
    let mut characters = 0;
    for span in spans {
        for _ in span.text.chars() {
            if !table_cell_internal_checkpoint(cancellation, characters) {
                return None;
            }
            characters += 1;
        }
    }
    Some(())
}

fn content_node_internal_checkpoints(
    node: &ContentNode,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<()> {
    match node {
        ContentNode::Heading { spans, .. } | ContentNode::Paragraph(spans, _) => {
            spans_internal_checkpoints(spans, cancellation)
        }
        ContentNode::BlockQuote { children, .. } | ContentNode::Figure { children, .. } => {
            for (index, child) in children.iter().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
                content_node_internal_checkpoints(child, cancellation)?;
            }
            Some(())
        }
        ContentNode::Table { row_groups, .. } => {
            for (index, cell) in row_groups
                .iter()
                .flat_map(|group| &group.rows)
                .flat_map(|row| &row.cells)
                .enumerate()
            {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
                for child in &cell.children {
                    content_node_internal_checkpoints(child, cancellation)?;
                }
            }
            Some(())
        }
        ContentNode::UnorderedList(items) | ContentNode::OrderedList { items, .. } => {
            for (index, item) in items.iter().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
                spans_internal_checkpoints(item, cancellation)?;
            }
            Some(())
        }
        ContentNode::Image { alt, caption, .. } => {
            for (index, _) in alt.chars().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
            }
            spans_internal_checkpoints(caption, cancellation)
        }
        ContentNode::Math { content, .. } => {
            for (index, _) in content.fallback.chars().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
            }
            Some(())
        }
        ContentNode::CodeBlock { code, .. } | ContentNode::InlineCode(code) => {
            for (index, _) in code.chars().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
            }
            Some(())
        }
        ContentNode::HorizontalRule => Some(()),
    }
}

fn epub_node_list_spacing_cancellable(
    nodes: &[ContentNode],
    font_size: f32,
    default_spacing: f32,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<f32> {
    let mut spacing = 0.0;
    for boundary in 0..=nodes.len() {
        if !table_cell_internal_checkpoint(cancellation, boundary) {
            return None;
        }
        spacing += epub_node_boundary_spacing(nodes, boundary, font_size, default_spacing);
    }
    Some(spacing)
}

pub fn epub_table_cell_content_height(
    children: &[ContentNode],
    font_size: f32,
    height: f32,
) -> f32 {
    (height
        - epub_node_list_spacing(children, font_size, EPUB_TABLE_CELL_SPACING)
        - 2.0 * EPUB_TABLE_CELL_PADDING)
        .max(1.0)
}

#[allow(clippy::too_many_arguments)]
pub fn epub_bounded_node_height(
    fonts: Option<&EpubFontBook>,
    node: &ContentNode,
    font_size: f32,
    width: f32,
    height: f32,
    chars_per_line: usize,
    lines_per_page: usize,
) -> f32 {
    measured_epub_compact_node_height_bounded(fonts, node, font_size, width, height).unwrap_or_else(
        || {
            estimated_epub_compact_node_height_bounded(
                node,
                chars_per_line,
                lines_per_page,
                font_size,
                width,
                height,
                None,
            )
        },
    )
}

#[cfg(test)]
pub fn epub_table_column_widths(row_groups: &[TableRowGroup], table_width: f32) -> Vec<f32> {
    let placements = epub_table_cell_placements(row_groups);
    epub_table_column_widths_from_placements(row_groups, table_width, &placements)
}

pub fn epub_table_column_widths_from_placements(
    row_groups: &[TableRowGroup],
    table_width: f32,
    placements: &[Vec<EpubTableCellPlacement>],
) -> Vec<f32> {
    epub_table_column_widths_from_placements_cancellable(row_groups, table_width, placements, None)
        .expect("the no-cancellation table width path cannot be interrupted")
}

fn epub_table_column_widths_from_placements_cancellable(
    row_groups: &[TableRowGroup],
    table_width: f32,
    placements: &[Vec<EpubTableCellPlacement>],
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<Vec<f32>> {
    let column_count = epub_table_column_count_from_placements(placements);
    let gaps = BLOCKQUOTE_SPACING * column_count.saturating_sub(1) as f32;
    let available = (table_width - gaps).max(column_count as f32);
    let minimum = (2.0 * EPUB_TABLE_CELL_PADDING + 4.0).min(available / column_count as f32);
    let mut weights = vec![0.25_f32; column_count];
    let mut authored_widths = vec![None::<f32>; column_count];

    let mut cell_index = 0;
    for (row_index, (row, row_placements)) in row_groups
        .iter()
        .flat_map(|group| &group.rows)
        .zip(placements)
        .enumerate()
    {
        if !table_row_checkpoint(cancellation, row_index) {
            return None;
        }
        for (cell, placement) in row.cells.iter().zip(row_placements) {
            if !table_cell_checkpoint(cancellation, cell_index) {
                return None;
            }
            cell_index += 1;
            let column = placement.column;
            let span = placement.span.min(column_count - column);
            if span == 0 {
                break;
            }
            let weight = table_cell_visual_characters_cancellable(cell, cancellation)?.max(1)
                as f32
                / span as f32;
            for column_weight in &mut weights[column..column + span] {
                *column_weight = column_weight.max(weight);
            }
            if let Some(width) = cell.style.width {
                let width = match width {
                    shosai_core::epub::render::NodeWidth::Percent(value) => value * available,
                    shosai_core::epub::render::NodeWidth::Pixels(value) => value,
                } / span as f32;
                for column_width in &mut authored_widths[column..column + span] {
                    *column_width = Some(column_width.unwrap_or(0.0).max(width));
                }
            }
        }
    }

    let unconstrained = authored_widths
        .iter()
        .filter(|width| width.is_none())
        .count();
    let minimum_unconstrained = minimum * unconstrained as f32;
    let authored_total = authored_widths
        .iter()
        .flatten()
        .map(|width| width.max(minimum))
        .sum::<f32>();
    let authored_budget = (available - minimum_unconstrained).max(0.0);
    let authored_scale = if unconstrained == 0 && authored_total > 0.0 {
        available / authored_total
    } else if authored_total > authored_budget && authored_total > 0.0 {
        authored_budget / authored_total
    } else {
        1.0
    };
    let fixed = authored_total * authored_scale;
    let remaining = (available - fixed - minimum_unconstrained).max(0.0);
    let unconstrained_weight = weights
        .iter()
        .zip(&authored_widths)
        .filter_map(|(weight, width)| width.is_none().then_some(*weight))
        .sum::<f32>()
        .max(f32::EPSILON);
    let widths = weights
        .into_iter()
        .zip(authored_widths)
        .map(|(weight, width)| match width {
            Some(width) => width.max(minimum) * authored_scale,
            None => minimum + remaining * weight / unconstrained_weight,
        })
        .collect();
    cancellation
        .is_none_or(|cancellation| !cancellation.is_cancelled())
        .then_some(widths)
}

pub fn epub_table_cell_width(placement: EpubTableCellPlacement, column_widths: &[f32]) -> f32 {
    let first_column = placement.column.min(column_widths.len());
    let span = placement
        .span
        .min(column_widths.len().saturating_sub(first_column));
    column_widths[first_column..first_column + span]
        .iter()
        .sum::<f32>()
        + BLOCKQUOTE_SPACING * span.saturating_sub(1) as f32
}

pub fn epub_table_cell_content_width(
    placement: EpubTableCellPlacement,
    column_widths: &[f32],
) -> f32 {
    (epub_table_cell_width(placement, column_widths) - 2.0 * EPUB_TABLE_CELL_PADDING).max(1.0)
}

fn table_cell_visual_characters_cancellable(
    cell: &shosai_core::epub::render::TableCell,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<usize> {
    let mut longest = 0;
    for (index, child) in cell.children.iter().enumerate() {
        if !table_cell_internal_checkpoint(cancellation, index) {
            return None;
        }
        longest = longest.max(content_node_visual_characters_cancellable(
            child,
            cancellation,
        )?);
    }
    Some(longest)
}

fn spans_visual_characters_cancellable(
    spans: &[shosai_core::epub::render::TextSpan],
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<usize> {
    let mut longest = 0;
    let mut current = 0;
    for (span_index, span) in spans.iter().enumerate() {
        if !table_cell_internal_checkpoint(cancellation, span_index) {
            return None;
        }
        for (character_index, character) in span.text.chars().enumerate() {
            if !table_cell_internal_checkpoint(cancellation, character_index) {
                return None;
            }
            if character == '\n' {
                longest = longest.max(current);
                current = 0;
            } else if !character.is_whitespace() && character != '\u{200b}' {
                current += 1;
            }
        }
    }
    Some(longest.max(current))
}

fn content_node_visual_characters_cancellable(
    node: &ContentNode,
    cancellation: Option<&EpubPaginationCancellation>,
) -> Option<usize> {
    match node {
        ContentNode::Heading { spans, .. } | ContentNode::Paragraph(spans, _) => {
            spans_visual_characters_cancellable(spans, cancellation)
        }
        ContentNode::BlockQuote { children, .. } | ContentNode::Figure { children, .. } => {
            let mut longest = 0;
            for (index, child) in children.iter().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
                longest = longest.max(content_node_visual_characters_cancellable(
                    child,
                    cancellation,
                )?);
            }
            Some(longest)
        }
        ContentNode::Table { row_groups, .. } => {
            let mut longest = 0;
            for (index, cell) in row_groups
                .iter()
                .flat_map(|group| &group.rows)
                .flat_map(|row| &row.cells)
                .enumerate()
            {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
                longest = longest.max(table_cell_visual_characters_cancellable(
                    cell,
                    cancellation,
                )?);
            }
            Some(longest)
        }
        ContentNode::Math { content, .. } => Some(content.fallback.chars().count()),
        ContentNode::UnorderedList(items) | ContentNode::OrderedList { items, .. } => {
            let mut longest = 0;
            for (index, item) in items.iter().enumerate() {
                if !table_cell_internal_checkpoint(cancellation, index) {
                    return None;
                }
                longest = longest.max(spans_visual_characters_cancellable(item, cancellation)?);
            }
            Some(longest)
        }
        ContentNode::Image { alt, caption, .. } => Some(
            alt.chars()
                .count()
                .max(spans_visual_characters_cancellable(caption, cancellation)?)
                .max(8),
        ),
        ContentNode::CodeBlock { code, .. } => Some(
            code.lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0),
        ),
        ContentNode::InlineCode(code) => Some(code.chars().count()),
        ContentNode::HorizontalRule => Some(1),
    }
}

fn measure_epub_spans(
    fonts: Option<&EpubFontBook>,
    spans: &[shosai_core::epub::render::TextSpan],
    base_size: f32,
    max_width: f32,
    direction: shosai_core::epub::style::TextDirection,
    alignment: Option<shosai_core::epub::style::TextAlignment>,
) -> Option<EpubTextLayout> {
    measure_epub_spans_with_prefix(
        fonts, "", base_size, spans, base_size, max_width, direction, alignment,
    )
}

#[allow(clippy::too_many_arguments)]
fn measure_epub_spans_with_prefix(
    fonts: Option<&EpubFontBook>,
    prefix: &str,
    prefix_size: f32,
    spans: &[shosai_core::epub::render::TextSpan],
    base_size: f32,
    max_width: f32,
    direction: shosai_core::epub::style::TextDirection,
    alignment: Option<shosai_core::epub::style::TextAlignment>,
) -> Option<EpubTextLayout> {
    if spans_text_len(spans).saturating_add(prefix.chars().count()) > EPUB_PAGINATION_SHAPE_CHUNK {
        return None;
    }
    let fonts = fonts.filter(|fonts| uses_native_fonts(fonts, spans))?;
    let mut runs = Vec::with_capacity(spans.len() + usize::from(!prefix.is_empty()));
    if !prefix.is_empty() {
        runs.push(EpubTextRun {
            text: prefix.to_owned(),
            family: None,
            monospace: false,
            font_size: prefix_size,
            bold: false,
            italic: false,
            foreground: [0, 0, 0, 255],
            link: None,
        });
    }
    runs.extend(spans.iter().map(|span| EpubTextRun {
        text: span.text.clone(),
        family: span.font_family.as_deref().map(str::to_owned),
        monospace: span.monospace,
        font_size: base_size * span.font_size_multiplier,
        bold: span.bold,
        italic: span.italic,
        foreground: [0, 0, 0, 255],
        link: None,
    }));
    let line_height = runs
        .iter()
        .map(|run| run.font_size)
        .fold(base_size, f32::max)
        * TEXT_LINE_HEIGHT;
    fonts
        .measure_text(&EpubTextRequest {
            runs,
            max_width: max_width.max(1.0),
            line_height,
            scale: 1.0,
            align: match alignment {
                Some(shosai_core::epub::style::TextAlignment::Center) => EpubTextAlign::Center,
                Some(shosai_core::epub::style::TextAlignment::Right) => EpubTextAlign::Right,
                Some(shosai_core::epub::style::TextAlignment::Justify) => EpubTextAlign::Justified,
                _ => EpubTextAlign::Left,
            },
            direction: match direction {
                shosai_core::epub::style::TextDirection::Ltr => EpubTextDirection::LeftToRight,
                shosai_core::epub::style::TextDirection::Rtl => EpubTextDirection::RightToLeft,
            },
            highlights: Vec::new(),
        })
        .ok()
}

pub fn paragraph_width(
    width: f32,
    font_size: f32,
    style: &shosai_core::epub::render::NodeStyle,
) -> f32 {
    let available = (width - style.margin_left_em.unwrap_or(0.0) * font_size).max(1.0);
    match style.width {
        Some(shosai_core::epub::render::NodeWidth::Percent(value)) => {
            (value * width).clamp(1.0, available)
        }
        Some(shosai_core::epub::render::NodeWidth::Pixels(value)) => value.clamp(1.0, available),
        None => available,
    }
}

fn blockquote_width(
    width: f32,
    font_size: f32,
    style: &shosai_core::epub::render::NodeStyle,
) -> f32 {
    (width - style.margin_left_em.unwrap_or(1.0) * font_size).max(1.0)
}

fn blockquote_continuation_page_size(
    page_size: Size,
    font_size: f32,
    style: &shosai_core::epub::render::NodeStyle,
) -> Size {
    Size::new(
        blockquote_width(page_size.width, font_size, style),
        page_size.height,
    )
}

pub fn uses_native_fonts(
    fonts: &EpubFontBook,
    spans: &[shosai_core::epub::render::TextSpan],
) -> bool {
    spans_text_len(spans) <= shosai_core::epub::EPUB_TEXT_MAX_SCALARS
        && spans.iter().any(|span| {
            span.font_family
                .as_deref()
                .is_some_and(|family| fonts.contains_family(family))
        })
}

#[allow(clippy::too_many_arguments)]
fn paginate_measured_paragraph(
    spans: &[shosai_core::epub::render::TextSpan],
    style: &shosai_core::epub::render::NodeStyle,
    measure: &impl Fn(&[shosai_core::epub::render::TextSpan]) -> Option<EpubTextLayout>,
    text_offset: usize,
    block_spacing: f32,
    page_height: f32,
    first_page_has_title: bool,
    pages: &mut Vec<PageNodes>,
    remaining: &mut f32,
    budget: &mut EpubPaginationBudget,
) -> bool {
    let text_len = spans_text_len(spans);
    let mut start = 0;
    let mut shape_window = EPUB_PAGINATION_SHAPE_CHUNK;
    let mut shaping_work = text_len
        .saturating_mul(4)
        .saturating_add(EPUB_PAGINATION_SHAPE_CHUNK);
    while start < text_len {
        if budget.is_cancelled() {
            return false;
        }
        let window_len = (text_len - start).min(shape_window);
        if shaping_work < window_len {
            return false;
        }
        shaping_work -= window_len;
        let remaining_spans = slice_epub_spans(spans, start, window_len);
        let Some(layout) = measure(&remaining_spans) else {
            return false;
        };
        if budget.is_cancelled() {
            return false;
        }
        let line_height = layout
            .lines
            .windows(2)
            .map(|lines| lines[1].top - lines[0].top)
            .find(|height| *height > 0.0)
            .unwrap_or_else(|| layout.height.max(1.0));
        if *remaining < line_height + block_spacing
            && page_has_content(pages, first_page_has_title)
            && push_epub_page(pages, budget)
        {
            *remaining = page_height;
        }
        let at_limit = budget.remaining_page_breaks == 0;
        let available = (*remaining - block_spacing).max(line_height);
        let fit = (available / line_height).floor().max(1.0) as usize;
        let end_line = if at_limit {
            layout.lines.len()
        } else {
            fit.min(layout.lines.len())
        };
        let mut length = layout
            .lines
            .get(end_line.saturating_sub(1))
            .map_or(window_len, |line| line.scalars.end)
            .min(window_len)
            .max(1);
        let mut page_spans = slice_epub_spans(spans, start, length);
        if shaping_work < length {
            return false;
        }
        shaping_work -= length;
        let Some(mut page_layout) = measure(&page_spans) else {
            return false;
        };
        if budget.is_cancelled() {
            return false;
        }
        while !at_limit && page_layout.height > available && length > 1 {
            if budget.is_cancelled() {
                return false;
            }
            let previous = page_layout
                .lines
                .iter()
                .rev()
                .nth(1)
                .map_or(0, |line| line.scalars.end);
            if previous == 0 || previous >= length {
                break;
            }
            if shaping_work < previous {
                return false;
            }
            shaping_work -= previous;
            let adjusted_spans = slice_epub_spans(spans, start, previous);
            let Some(adjusted) = measure(&adjusted_spans) else {
                return false;
            };
            length = previous;
            page_spans = adjusted_spans;
            page_layout = adjusted;
        }
        let is_last = start + length >= text_len;
        let mut fragment_style = style.clone();
        if start > 0 {
            fragment_style.block_before_em = Some(0.0);
        }
        if !is_last {
            fragment_style.block_after_em = Some(0.0);
        }
        pages.last_mut().unwrap().push(PageNode {
            node: ContentNode::Paragraph(page_spans, fragment_style),
            text_offset: text_offset + start,
            block_before: 0.0,
            block_after: 0.0,
        });
        *remaining = (*remaining
            - (page_layout.height + if is_last { block_spacing } else { 0.0 }))
        .max(0.0);
        start += length;
        shape_window = length
            .saturating_mul(2)
            .clamp(1, EPUB_PAGINATION_SHAPE_CHUNK);
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn paginate_epub_list(
    items: &[Vec<shosai_core::epub::render::TextSpan>],
    ordered_start: Option<usize>,
    text_offset: usize,
    chars_per_line: usize,
    font_size: f32,
    line_spacing: f32,
    page_height: f32,
    page_width: f32,
    fonts: Option<&EpubFontBook>,
    first_page_has_title: bool,
    pages: &mut Vec<PageNodes>,
    remaining: &mut f32,
    budget: &mut EpubPaginationBudget,
) -> bool {
    let mut pagination_items = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if index % EPUB_PAGINATION_LOOP_CHUNK == 0 && budget.is_cancelled() {
            return false;
        }
        pagination_items.push(pagination_inline_spans(
            item,
            font_size,
            page_width,
            page_height,
            shosai_core::epub::style::TextDirection::Ltr,
            None,
        ));
    }
    let items = pagination_items.as_slice();
    let mut consumed_items = 0;
    let mut consumed_text = 0;
    let block_spacing = font_size * line_spacing;

    while consumed_items < items.len() {
        if budget.is_cancelled() {
            return false;
        }
        let first_scale = spans_font_scale(&items[consumed_items]);
        let first_line_height = font_size * TEXT_LINE_HEIGHT * first_scale;
        if *remaining < first_line_height + block_spacing
            && page_has_content(pages, first_page_has_title)
        {
            if push_epub_page(pages, budget) {
                *remaining = page_height;
            } else {
                pages.last_mut().unwrap().push(PageNode {
                    node: epub_list_node(
                        &items[consumed_items..],
                        ordered_start.map(|start| start.saturating_add(consumed_items)),
                    ),
                    text_offset: text_offset + consumed_text,
                    block_before: 0.0,
                    block_after: 0.0,
                });
                return !budget.is_cancelled();
            }
        }

        let available_height = (*remaining - block_spacing).max(first_line_height);
        let mut chunk_height = 0.0;
        let mut take = 0;
        for (chunk_index, item) in items[consumed_items..].iter().enumerate() {
            if chunk_index % EPUB_PAGINATION_LOOP_CHUNK == 0 && budget.is_cancelled() {
                return false;
            }
            let scale = spans_font_scale(item);
            let item_chars_per_line = scaled_characters_per_line(chars_per_line, scale);
            let item_lines = (spans_text_len(item) + 4)
                .div_ceil(item_chars_per_line)
                .max(1);
            let item_spacing = if take == 0 { 0.0 } else { 4.0 };
            let absolute_index = consumed_items + chunk_index;
            let prefix = ordered_start.map_or_else(
                || "  \u{2022} ".to_owned(),
                |start| format!("  {}. ", start.saturating_add(absolute_index)),
            );
            let item_height =
                measure_epub_spans_with_prefix(
                    fonts,
                    &prefix,
                    font_size * scale,
                    item,
                    font_size,
                    page_width,
                    shosai_core::epub::style::TextDirection::Ltr,
                    None,
                )
                .map_or(
                    item_lines as f32 * font_size * TEXT_LINE_HEIGHT * scale,
                    |layout| layout.height,
                ) + inline_math_height_reserve(item, font_size, page_width, page_height);
            if take > 0 && chunk_height + item_spacing + item_height > available_height {
                break;
            }
            if take == 0
                && item_height > available_height
                && page_has_content(pages, first_page_has_title)
            {
                break;
            }
            chunk_height += item_spacing + item_height;
            take += 1;
        }

        if take == 0 {
            if push_epub_page(pages, budget) {
                *remaining = page_height;
            } else {
                pages.last_mut().unwrap().push(PageNode {
                    node: epub_list_node(
                        &items[consumed_items..],
                        ordered_start.map(|start| start.saturating_add(consumed_items)),
                    ),
                    text_offset: text_offset + consumed_text,
                    block_before: 0.0,
                    block_after: 0.0,
                });
                return !budget.is_cancelled();
            }
            continue;
        }

        let node = epub_list_node(
            &items[consumed_items..consumed_items + take],
            ordered_start.map(|start| start.saturating_add(consumed_items)),
        );
        pages.last_mut().unwrap().push(PageNode {
            node,
            text_offset: text_offset + consumed_text,
            block_before: 0.0,
            block_after: 0.0,
        });
        *remaining = (*remaining - chunk_height - block_spacing).max(0.0);
        consumed_text += items[consumed_items..consumed_items + take]
            .iter()
            .map(|item| spans_text_len(item) + 1)
            .sum::<usize>();
        consumed_items += take;
    }
    !budget.is_cancelled()
}

fn epub_list_node(
    items: &[Vec<shosai_core::epub::render::TextSpan>],
    ordered_start: Option<usize>,
) -> ContentNode {
    match ordered_start {
        Some(start) => ContentNode::OrderedList {
            items: items.to_vec(),
            start,
        },
        None => ContentNode::UnorderedList(items.to_vec()),
    }
}

#[derive(Clone)]
struct EpubSpanCursor<'a> {
    spans: &'a [shosai_core::epub::render::TextSpan],
    span_index: usize,
    byte_offset: usize,
    consumed: usize,
    remaining: usize,
}

impl<'a> EpubSpanCursor<'a> {
    fn new(spans: &'a [shosai_core::epub::render::TextSpan]) -> Self {
        Self {
            spans,
            span_index: 0,
            byte_offset: 0,
            consumed: 0,
            remaining: spans_text_len(spans),
        }
    }

    fn consumed(&self) -> usize {
        self.consumed
    }

    fn remaining(&self) -> usize {
        self.remaining
    }

    fn split_length(&self, maximum: usize, budget: &EpubPaginationBudget) -> Option<usize> {
        if self.remaining <= maximum {
            return (!budget.is_cancelled()).then_some(self.remaining);
        }
        let mut length = 0;
        let mut last_whitespace = None;
        for (index, span) in self.spans[self.span_index..].iter().enumerate() {
            let start = if index == 0 { self.byte_offset } else { 0 };
            if span.math.is_some() {
                let span_len = span.text[start..].chars().count();
                if length + span_len > maximum {
                    return Some(if length == 0 { span_len } else { length });
                }
                length += span_len;
                continue;
            }
            for character in span.text[start..].chars() {
                if length % EPUB_PAGINATION_SHAPE_CHUNK == 0 && budget.is_cancelled() {
                    return None;
                }
                if length == maximum {
                    break;
                }
                length += 1;
                if character.is_whitespace() {
                    last_whitespace = Some(length);
                }
            }
            if length == maximum {
                break;
            }
        }
        Some(
            last_whitespace
                .filter(|length| *length >= maximum / 2)
                .unwrap_or(maximum),
        )
    }

    fn take(
        &mut self,
        length: usize,
        budget: &EpubPaginationBudget,
    ) -> Option<Vec<shosai_core::epub::render::TextSpan>> {
        let mut output = Vec::new();
        let mut remaining = length;
        while remaining > 0 && self.span_index < self.spans.len() {
            let source = &self.spans[self.span_index];
            let suffix = &source.text[self.byte_offset..];
            let mut bytes = 0;
            let mut characters = 0;
            for (index, character) in suffix.chars().take(remaining).enumerate() {
                if index % EPUB_PAGINATION_SHAPE_CHUNK == 0 && budget.is_cancelled() {
                    return None;
                }
                bytes += character.len_utf8();
                characters += 1;
            }
            if characters > 0 {
                let mut span = source.clone();
                span.text = suffix[..bytes].to_string();
                output.push(span);
                self.byte_offset += bytes;
                self.consumed += characters;
                self.remaining -= characters;
                remaining -= characters;
            }
            if self.byte_offset == source.text.len() {
                self.span_index += 1;
                self.byte_offset = 0;
            }
        }
        (!budget.is_cancelled()).then_some(output)
    }
}

fn epub_span_split_length(
    spans: &[shosai_core::epub::render::TextSpan],
    start: usize,
    maximum: usize,
) -> usize {
    let remaining = spans_text_len(spans).saturating_sub(start);
    if remaining <= maximum {
        return remaining;
    }
    let window = spans
        .iter()
        .flat_map(|span| span.text.chars())
        .skip(start)
        .take(maximum)
        .collect::<Vec<_>>();
    window
        .iter()
        .rposition(|character| character.is_whitespace())
        .map(|index| index + 1)
        .filter(|length| *length >= maximum / 2)
        .unwrap_or(maximum)
}

fn slice_epub_spans(
    spans: &[shosai_core::epub::render::TextSpan],
    start: usize,
    length: usize,
) -> Vec<shosai_core::epub::render::TextSpan> {
    let end = start + length;
    let mut offset = 0;
    spans
        .iter()
        .filter_map(|span| {
            let span_len = span.text.chars().count();
            let local_start = start.saturating_sub(offset).min(span_len);
            let local_end = end.saturating_sub(offset).min(span_len);
            offset += span_len;
            (local_start < local_end).then(|| {
                let mut sliced = span.clone();
                sliced.text = span
                    .text
                    .chars()
                    .skip(local_start)
                    .take(local_end - local_start)
                    .collect();
                if local_start != 0 || local_end != span_len {
                    sliced.math = None;
                }
                sliced
            })
        })
        .collect()
}

fn estimated_epub_node_height(
    node: &ContentNode,
    chars_per_line: usize,
    lines_per_page: usize,
    font_size: f32,
    line_spacing: f32,
) -> f32 {
    estimated_epub_compact_node_height(node, chars_per_line, lines_per_page, font_size)
        + font_size * line_spacing
}

fn estimated_epub_blockquote_height(
    children: &[ContentNode],
    style: &shosai_core::epub::render::NodeStyle,
    _chars_per_line: usize,
    lines_per_page: usize,
    font_size: f32,
    width: f32,
    height: f32,
) -> f32 {
    let width = blockquote_width(width, font_size, style);
    let chars_per_line = (width / (font_size * AVERAGE_CHARACTER_WIDTH).max(1.0))
        .floor()
        .max(1.0) as usize;
    children
        .iter()
        .map(|child| {
            estimated_epub_compact_node_height_bounded(
                child,
                chars_per_line,
                lines_per_page,
                font_size,
                width,
                height,
                None,
            )
        })
        .sum::<f32>()
        + epub_fragment_list_spacing(children, font_size, BLOCKQUOTE_SPACING, style)
}

fn split_epub_blockquote_prefix(
    children: &[ContentNode],
    available_height: f32,
    lines_per_page: usize,
    font_size: f32,
    page_width: f32,
    fonts: Option<&EpubFontBook>,
    budget: &EpubPaginationBudget,
) -> Option<(Vec<ContentNode>, Vec<ContentNode>, f32, usize)> {
    let chars_per_line = (page_width / (font_size * AVERAGE_CHARACTER_WIDTH).max(1.0))
        .floor()
        .max(1.0) as usize;
    let mut prefix = Vec::new();
    let mut prefix_height = 0.0;
    let mut consumed_text = 0;
    for (index, child) in children.iter().enumerate() {
        if budget.is_cancelled() {
            return None;
        }
        let spacing = epub_node_boundary_spacing(children, index, font_size, BLOCKQUOTE_SPACING);
        let trailing = if index + 1 == children.len() {
            epub_node_boundary_spacing(children, children.len(), font_size, BLOCKQUOTE_SPACING)
        } else {
            0.0
        };
        let child_height = measured_epub_compact_node_height(fonts, child, font_size, page_width)
            .unwrap_or_else(|| {
                estimated_epub_compact_node_height_bounded(
                    child,
                    chars_per_line,
                    lines_per_page,
                    font_size,
                    page_width,
                    available_height,
                    None,
                )
            });
        if prefix_height + spacing + child_height + trailing <= available_height {
            prefix.push(child.clone());
            prefix_height += spacing + child_height;
            if index + 1 == children.len() {
                prefix_height += trailing;
            }
            consumed_text += content_node_text_len(child) + 1;
            continue;
        }

        if let ContentNode::BlockQuote {
            children: nested_children,
            style,
        } = child
        {
            let nested_available = available_height - prefix_height - spacing;
            if nested_available > 0.0 {
                let (nested_prefix, nested_remaining, nested_height, nested_consumed_text) =
                    split_epub_blockquote_prefix(
                        nested_children,
                        nested_available,
                        lines_per_page,
                        font_size,
                        blockquote_width(page_width, font_size, style),
                        fonts,
                        budget,
                    )?;
                if !nested_prefix.is_empty() {
                    let mut prefix_style = style.clone();
                    prefix_style.fragment_after = !nested_remaining.is_empty();
                    prefix.push(ContentNode::BlockQuote {
                        children: nested_prefix,
                        style: prefix_style,
                    });
                    prefix_height += spacing + nested_height;
                    let mut remaining = Vec::new();
                    if !nested_remaining.is_empty() {
                        let mut remaining_style = style.clone();
                        remaining_style.fragment_before = true;
                        remaining.push(ContentNode::BlockQuote {
                            children: nested_remaining,
                            style: remaining_style,
                        });
                    }
                    remaining.extend_from_slice(&children[index + 1..]);
                    suppress_epub_fragment_boundary(&mut prefix, &mut remaining);
                    return Some((
                        prefix,
                        remaining,
                        prefix_height,
                        consumed_text + nested_consumed_text,
                    ));
                }
            }
        }

        if let ContentNode::Paragraph(spans, style) = child {
            let paragraph_available = available_height - prefix_height - spacing;
            let base_scale = style.font_size_multiplier.unwrap_or(1.0);
            let effective_scale = base_scale * spans_font_scale(spans);
            let base_size = font_size * base_scale;
            let line_height = font_size * TEXT_LINE_HEIGHT * effective_scale;
            let effective_width = paragraph_width(page_width, font_size, style);
            let paragraph_chars_per_line = (effective_width
                / (font_size * AVERAGE_CHARACTER_WIDTH * effective_scale).max(1.0))
            .floor()
            .max(1.0) as usize;
            let pagination_spans = pagination_inline_spans(
                spans,
                base_size,
                effective_width,
                paragraph_available.max(1.0),
                style.direction,
                style.text_align,
            );
            let spans = pagination_spans.as_slice();
            let available_lines = (paragraph_available / line_height).floor().max(0.0) as usize;
            let maximum = paragraph_chars_per_line * available_lines;
            let text_len = spans_text_len(spans);
            let measured = measure_epub_spans(
                fonts,
                spans,
                base_size,
                effective_width,
                style.direction,
                style.text_align,
            );
            let mut take = measured.as_ref().map_or_else(
                || epub_span_split_length(spans, 0, maximum),
                |layout| {
                    layout
                        .lines
                        .iter()
                        .take_while(|line| line.top + line_height <= paragraph_available)
                        .last()
                        .map_or(0, |line| line.scalars.end)
                },
            );
            let mut reshaped = None;
            while measured.is_some() && take > 0 && take < text_len {
                if budget.is_cancelled() {
                    return None;
                }
                let candidate = slice_epub_spans(spans, 0, take);
                let Some(layout) = measure_epub_spans(
                    fonts,
                    &candidate,
                    base_size,
                    effective_width,
                    style.direction,
                    style.text_align,
                ) else {
                    take = 0;
                    break;
                };
                if layout.height <= paragraph_available {
                    reshaped = Some(layout);
                    break;
                }
                let previous = layout
                    .lines
                    .iter()
                    .rev()
                    .nth(1)
                    .map_or(0, |line| line.scalars.end);
                if previous == 0 || previous >= take {
                    take = 0;
                    break;
                }
                take = previous;
            }
            if take > 0 && take < text_len {
                let prefix_spans = slice_epub_spans(spans, 0, take);
                let remaining_spans = slice_epub_spans(spans, take, text_len - take);
                let paragraph_height = reshaped.or(measured).map_or_else(
                    || take.div_ceil(paragraph_chars_per_line).max(1) as f32 * line_height,
                    |layout| layout.height,
                ) + inline_math_height_reserve_for_context(
                    &prefix_spans,
                    base_size,
                    effective_width,
                    paragraph_available.max(1.0),
                    style.direction,
                    style.text_align,
                );
                prefix.push(ContentNode::Paragraph(prefix_spans, style.clone()));
                prefix_height += spacing + paragraph_height;
                let mut remaining = vec![ContentNode::Paragraph(remaining_spans, style.clone())];
                remaining.extend_from_slice(&children[index + 1..]);
                suppress_epub_fragment_boundary(&mut prefix, &mut remaining);
                return Some((prefix, remaining, prefix_height, consumed_text + take));
            }
        }

        let mut remaining = children[index..].to_vec();
        suppress_epub_fragment_boundary(&mut prefix, &mut remaining);
        return Some((prefix, remaining, prefix_height, consumed_text));
    }

    Some((prefix, Vec::new(), prefix_height, consumed_text))
}

fn suppress_epub_fragment_boundary(prefix: &mut [ContentNode], remaining: &mut [ContentNode]) {
    if let Some(style) = prefix.last_mut().and_then(ContentNode::style_mut) {
        style.block_after_em = Some(0.0);
    }
    if let Some(style) = remaining.first_mut().and_then(ContentNode::style_mut) {
        style.block_before_em = Some(0.0);
    }
}

fn estimated_epub_compact_node_height(
    node: &ContentNode,
    chars_per_line: usize,
    lines_per_page: usize,
    font_size: f32,
) -> f32 {
    estimated_epub_compact_node_height_bounded(
        node,
        chars_per_line,
        lines_per_page,
        font_size,
        chars_per_line as f32 * font_size * AVERAGE_CHARACTER_WIDTH,
        lines_per_page as f32 * font_size * TEXT_LINE_HEIGHT,
        Some(lines_per_page as f32 * font_size * TEXT_LINE_HEIGHT),
    )
}

pub fn epub_table_caption_height(
    fonts: Option<&EpubFontBook>,
    caption: &[shosai_core::epub::render::TextSpan],
    style: Option<&shosai_core::epub::render::NodeStyle>,
    font_size: f32,
    width: f32,
    height: f32,
) -> f32 {
    if caption.is_empty() {
        return 0.0;
    }
    let caption_size = font_size
        * style
            .and_then(|style| style.font_size_multiplier)
            .unwrap_or(1.0);
    let span_scale = spans_font_scale(caption);
    let characters_per_line = (width
        / (caption_size * span_scale * AVERAGE_CHARACTER_WIDTH).max(1.0))
    .floor()
    .max(1.0) as usize;
    let fallback = caption
        .iter()
        .map(|span| span.text.as_str())
        .collect::<String>()
        .split('\n')
        .map(|line| line.chars().count().div_ceil(characters_per_line).max(1))
        .sum::<usize>() as f32
        * caption_size
        * span_scale
        * TEXT_LINE_HEIGHT;
    measure_epub_spans(
        fonts,
        caption,
        caption_size,
        width,
        style.map_or(Default::default(), |style| style.direction),
        style.and_then(|style| style.text_align),
    )
    .map_or(fallback, |layout| layout.height)
        + inline_math_height_reserve(caption, caption_size, width, height)
}

fn estimated_epub_compact_node_height_bounded(
    node: &ContentNode,
    chars_per_line: usize,
    lines_per_page: usize,
    font_size: f32,
    width: f32,
    height: f32,
    percentage_height_basis: Option<f32>,
) -> f32 {
    let wrapped = |characters: usize, scale: f32| {
        characters
            .div_ceil(scaled_characters_per_line(chars_per_line, scale))
            .max(1) as f32
    };
    let text_line_height = font_size * TEXT_LINE_HEIGHT;
    match node {
        ContentNode::Heading {
            spans,
            level,
            style,
            ..
        } => {
            let heading_scale = match level {
                1 => 2.0,
                2 => 1.6,
                3 => 1.3,
                4 => 1.1,
                _ => 1.0,
            };
            let style_scale = style.font_size_multiplier.unwrap_or(1.0);
            let scale = heading_scale * style_scale * spans_font_scale(spans);
            wrapped(spans_text_len(spans), scale) * text_line_height * scale
                + inline_math_height_reserve(
                    spans,
                    font_size * heading_scale * style_scale,
                    width,
                    height,
                )
        }
        ContentNode::BlockQuote { children, style } => estimated_epub_blockquote_height(
            children,
            style,
            chars_per_line,
            lines_per_page,
            font_size,
            width,
            height,
        ),
        ContentNode::Figure { children, style } => {
            let figure_width = epub_figure_content_width(style, width, font_size);
            let figure_chars_per_line = (figure_width
                / (font_size * AVERAGE_CHARACTER_WIDTH).max(1.0))
            .floor()
            .max(1.0) as usize;
            children
                .iter()
                .map(|child| {
                    estimated_epub_compact_node_height_bounded(
                        child,
                        figure_chars_per_line,
                        lines_per_page,
                        font_size,
                        figure_width,
                        height,
                        None,
                    )
                })
                .sum::<f32>()
                + epub_fragment_list_spacing(children, font_size, BLOCKQUOTE_SPACING, style)
        }
        ContentNode::Table {
            caption,
            caption_style,
            row_groups,
            style,
        } => {
            let table_width = epub_table_layout_width(row_groups, style, width);
            let table_content_width =
                epub_table_content_width(style, table_width, width, font_size);
            let placements = epub_table_cell_placements(row_groups);
            let column_widths = epub_table_column_widths_from_placements(
                row_groups,
                table_content_width,
                &placements,
            );
            let caption_height = (!caption.is_empty()).then(|| {
                epub_table_caption_height(
                    None,
                    caption,
                    caption_style.as_ref(),
                    font_size,
                    table_content_width,
                    height,
                )
            });
            let caption_gap = EPUB_TABLE_ROW_SPACING
                * usize::from(caption_height.is_some() && !row_groups.is_empty()) as f32;
            let geometry = epub_table_geometry_bounded_from_placements(
                row_groups,
                &placements,
                &column_widths,
                lines_per_page,
                font_size,
                (height - caption_height.unwrap_or(0.0) - caption_gap).max(1.0),
                None,
            );
            caption_height.unwrap_or(0.0) + geometry.height + caption_gap
        }
        ContentNode::Math { content, style, .. } => {
            let scale = style.font_size_multiplier.unwrap_or(1.0);
            let size = font_size * scale;
            content
                .expression
                .as_ref()
                .and_then(|expression| {
                    math_layout::layout_math_for_bounds(expression, size, width, height)
                })
                .map_or_else(
                    || wrapped(content.fallback.chars().count(), scale) * text_line_height * scale,
                    |layout| layout.height,
                )
        }
        ContentNode::UnorderedList(items) | ContentNode::OrderedList { items, .. } => {
            items
                .iter()
                .map(|item| {
                    let scale = spans_font_scale(item);
                    wrapped(spans_text_len(item) + 4, scale) * text_line_height * scale
                        + inline_math_height_reserve(item, font_size, width, height)
                })
                .sum::<f32>()
                + 4.0 * items.len().saturating_sub(1) as f32
        }
        ContentNode::CodeBlock { code, .. } => {
            code.lines().count().max(1) as f32 * text_line_height * 0.85 + 24.0
        }
        ContentNode::InlineCode(code) => {
            wrapped(code.chars().count(), 0.9) * text_line_height * 0.9
        }
        ContentNode::Image { .. } => epub_image_layout(
            node,
            font_size,
            width,
            percentage_height_basis,
            Some(height),
            None,
        )
        .map_or_else(
            || (lines_per_page / 2).max(4) as f32 * font_size * TEXT_LINE_HEIGHT,
            EpubImageLayout::total_height,
        ),
        ContentNode::HorizontalRule => text_line_height,
        ContentNode::Paragraph(spans, style) => {
            let scale = style.font_size_multiplier.unwrap_or(1.0) * spans_font_scale(spans);
            let effective_width = paragraph_width(width, font_size, style);
            let characters_per_line = (effective_width
                / (font_size * AVERAGE_CHARACTER_WIDTH * scale).max(1.0))
            .floor()
            .max(1.0) as usize;
            spans_text_len(spans).div_ceil(characters_per_line).max(1) as f32
                * text_line_height
                * scale
                + inline_math_height_reserve(
                    spans,
                    font_size * style.font_size_multiplier.unwrap_or(1.0),
                    effective_width,
                    height,
                )
        }
    }
}

fn measured_epub_compact_node_height(
    fonts: Option<&EpubFontBook>,
    node: &ContentNode,
    font_size: f32,
    width: f32,
) -> Option<f32> {
    measured_epub_compact_node_height_bounded(fonts, node, font_size, width, f32::MAX)
}

fn measured_epub_compact_node_height_bounded(
    fonts: Option<&EpubFontBook>,
    node: &ContentNode,
    font_size: f32,
    width: f32,
    height: f32,
) -> Option<f32> {
    match node {
        ContentNode::Heading {
            spans,
            level,
            style,
        } => {
            let heading_scale = match level {
                1 => 2.0,
                2 => 1.6,
                3 => 1.3,
                4 => 1.1,
                _ => 1.0,
            } * style.font_size_multiplier.unwrap_or(1.0);
            measure_epub_spans(
                fonts,
                spans,
                font_size * heading_scale,
                width,
                style.direction,
                style.text_align,
            )
            .map(|layout| {
                layout.height
                    + inline_math_height_reserve_for_context(
                        spans,
                        font_size * heading_scale,
                        width,
                        height,
                        style.direction,
                        style.text_align,
                    )
            })
        }
        ContentNode::Paragraph(spans, style) => {
            let base_size = font_size * style.font_size_multiplier.unwrap_or(1.0);
            measure_epub_spans(
                fonts,
                spans,
                base_size,
                paragraph_width(width, font_size, style),
                style.direction,
                style.text_align,
            )
            .map(|layout| {
                let effective_width = paragraph_width(width, font_size, style);
                layout.height
                    + inline_math_height_reserve_for_context(
                        spans,
                        base_size,
                        effective_width,
                        height,
                        style.direction,
                        style.text_align,
                    )
            })
        }
        ContentNode::Math { content, style, .. } => {
            let span = shosai_core::epub::render::TextSpan {
                text: content.fallback.clone(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            };
            let size = font_size * style.font_size_multiplier.unwrap_or(1.0);
            if let Some(layout) = content.expression.as_ref().and_then(|expression| {
                math_layout::layout_math_for_bounds(expression, size, width, height)
            }) {
                return Some(layout.height);
            }
            measure_epub_spans(
                fonts,
                std::slice::from_ref(&span),
                size,
                width,
                style.direction,
                style.text_align,
            )
            .map(|layout| layout.height)
        }
        ContentNode::UnorderedList(items) => {
            measured_epub_list_height(fonts, items, None, font_size, width)
        }
        ContentNode::OrderedList { items, start } => {
            measured_epub_list_height(fonts, items, Some(*start), font_size, width)
        }
        ContentNode::BlockQuote { children, style } => {
            let width = blockquote_width(width, font_size, style);
            let heights: Option<Vec<_>> = children
                .iter()
                .map(|child| measured_epub_compact_node_height(fonts, child, font_size, width))
                .collect();
            heights.map(|heights| {
                heights.into_iter().sum::<f32>()
                    + epub_fragment_list_spacing(children, font_size, BLOCKQUOTE_SPACING, style)
            })
        }
        ContentNode::Figure { children, style } => {
            let width = epub_figure_content_width(style, width, font_size);
            let heights: Option<Vec<_>> = children
                .iter()
                .map(|child| measured_epub_compact_node_height(fonts, child, font_size, width))
                .collect();
            heights.map(|heights| {
                heights.into_iter().sum::<f32>()
                    + epub_fragment_list_spacing(children, font_size, BLOCKQUOTE_SPACING, style)
            })
        }
        ContentNode::Image { .. } => {
            epub_image_layout(node, font_size, width, None, Some(height), fonts)
                .map(EpubImageLayout::total_height)
        }
        _ => None,
    }
}

fn measured_epub_list_height(
    fonts: Option<&EpubFontBook>,
    items: &[Vec<shosai_core::epub::render::TextSpan>],
    ordered_start: Option<usize>,
    font_size: f32,
    width: f32,
) -> Option<f32> {
    let mut any = false;
    let height = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let scale = spans_font_scale(item);
            let fallback = (spans_text_len(item) + 4)
                .div_ceil(scaled_characters_per_line(
                    (width / (font_size * AVERAGE_CHARACTER_WIDTH).max(1.0)) as usize,
                    scale,
                ))
                .max(1) as f32
                * font_size
                * TEXT_LINE_HEIGHT
                * scale;
            let prefix = ordered_start.map_or_else(
                || "  \u{2022} ".to_owned(),
                |start| format!("  {}. ", start.saturating_add(index)),
            );
            measure_epub_spans_with_prefix(
                fonts,
                &prefix,
                font_size * scale,
                item,
                font_size,
                width,
                shosai_core::epub::style::TextDirection::Ltr,
                None,
            )
            .map_or(fallback, |layout| {
                any = true;
                layout.height
            }) + inline_math_height_reserve(item, font_size, width, f32::MAX)
        })
        .sum::<f32>()
        + 4.0 * items.len().saturating_sub(1) as f32;
    any.then_some(height)
}

pub fn spans_text_len(spans: &[super::render::TextSpan]) -> usize {
    spans.iter().map(|span| span.text.chars().count()).sum()
}

pub fn spans_font_scale(spans: &[super::render::TextSpan]) -> f32 {
    spans
        .iter()
        .map(|span| span.font_size_multiplier)
        .reduce(f32::max)
        .unwrap_or(1.0)
}

fn inline_math_height_reserve(
    spans: &[shosai_core::epub::render::TextSpan],
    base_size: f32,
    width: f32,
    height: f32,
) -> f32 {
    inline_math_height_reserve_for_context(
        spans,
        base_size,
        width,
        height,
        shosai_core::epub::style::TextDirection::Ltr,
        None,
    )
}

fn inline_math_height_reserve_for_context(
    spans: &[shosai_core::epub::render::TextSpan],
    base_size: f32,
    width: f32,
    height: f32,
    direction: shosai_core::epub::style::TextDirection,
    alignment: Option<shosai_core::epub::style::TextAlignment>,
) -> f32 {
    let geometry = spans
        .iter()
        .filter_map(|span| {
            let layout = layout_inline_math_span_for_context(
                span, base_size, width, height, direction, alignment,
            )?;
            let line_height = base_size * span.font_size_multiplier * TEXT_LINE_HEIGHT;
            Some((layout.height - line_height).max(0.0))
        })
        .sum::<f32>();
    if geometry == 0.0 {
        return 0.0;
    }
    let scale = spans_font_scale(spans);
    let chars_per_line = (width / (base_size * AVERAGE_CHARACTER_WIDTH).max(1.0))
        .floor()
        .max(1.0) as usize;
    let lines = spans_text_len(spans)
        .div_ceil(scaled_characters_per_line(chars_per_line, scale))
        .max(1);
    geometry + lines.saturating_sub(1) as f32 * base_size * INLINE_MATH_WRAP_SPACING
}

pub fn layout_inline_math_span(
    span: &shosai_core::epub::render::TextSpan,
    base_size: f32,
    width: f32,
    height: f32,
) -> Option<math_layout::MathLayout> {
    let math = span.math.as_ref()?;
    if math.display != shosai_core::epub::MathDisplay::Inline {
        return None;
    }
    let expression = math.expression.as_ref()?;
    let size = base_size * span.font_size_multiplier;
    math_layout::layout_math_for_bounds(
        expression,
        size,
        width,
        height.min(size * TEXT_LINE_HEIGHT * MAX_INLINE_MATH_LINE_HEIGHTS),
    )
}

pub fn layout_inline_math_span_for_context(
    span: &shosai_core::epub::render::TextSpan,
    base_size: f32,
    width: f32,
    height: f32,
    direction: shosai_core::epub::style::TextDirection,
    alignment: Option<shosai_core::epub::style::TextAlignment>,
) -> Option<math_layout::MathLayout> {
    if direction != shosai_core::epub::style::TextDirection::Ltr
        || alignment == Some(shosai_core::epub::style::TextAlignment::Justify)
    {
        return None;
    }
    layout_inline_math_span(span, base_size, width, height)
}

fn pagination_inline_spans(
    spans: &[shosai_core::epub::render::TextSpan],
    base_size: f32,
    width: f32,
    height: f32,
    direction: shosai_core::epub::style::TextDirection,
    alignment: Option<shosai_core::epub::style::TextAlignment>,
) -> Vec<shosai_core::epub::render::TextSpan> {
    let admit_flow = inline_math_flow_is_admitted(spans);
    spans
        .iter()
        .cloned()
        .map(|mut span| {
            if !admit_flow
                || layout_inline_math_span_for_context(
                    &span, base_size, width, height, direction, alignment,
                )
                .is_none()
            {
                span.math = None;
            }
            span
        })
        .collect()
}

pub fn inline_math_flow_is_admitted(spans: &[super::render::TextSpan]) -> bool {
    spans
        .iter()
        .map(|span| span.text.split_inclusive(char::is_whitespace).count())
        .sum::<usize>()
        <= MAX_INLINE_MATH_FLOW_ITEMS
}

pub fn content_node_text_len(node: &ContentNode) -> usize {
    match node {
        ContentNode::Heading { spans, .. } => spans_text_len(spans),
        ContentNode::Paragraph(spans, _) => spans_text_len(spans),
        ContentNode::BlockQuote { children, .. } => children
            .iter()
            .map(|child| content_node_text_len(child) + 1)
            .sum(),
        ContentNode::Figure { children, .. } => {
            children.iter().map(content_node_text_len).sum::<usize>()
                + children.len().saturating_sub(1)
        }
        ContentNode::Table {
            caption,
            row_groups,
            ..
        } => {
            let caption_len = spans_text_len(caption) + usize::from(!caption.is_empty());
            caption_len
                + row_groups
                    .iter()
                    .flat_map(|group| &group.rows)
                    .map(|row| {
                        row.cells
                            .iter()
                            .map(|cell| {
                                cell.children
                                    .iter()
                                    .enumerate()
                                    .map(|(index, child)| {
                                        content_node_text_len(child)
                                            + usize::from(cell.block_starts.contains(&index))
                                    })
                                    .sum::<usize>()
                            })
                            .sum::<usize>()
                            + row.cells.len().saturating_sub(1)
                            + 1
                    })
                    .sum::<usize>()
        }
        ContentNode::UnorderedList(items) | ContentNode::OrderedList { items, .. } => {
            items.iter().map(|spans| spans_text_len(spans) + 1).sum()
        }
        ContentNode::CodeBlock { code, .. } | ContentNode::InlineCode(code) => code.chars().count(),
        ContentNode::Image { alt, caption, .. } => {
            alt.chars().count() + usize::from(!caption.is_empty()) + spans_text_len(caption)
        }
        ContentNode::Math { content, .. } => content.fallback.chars().count(),
        ContentNode::HorizontalRule => 0,
    }
}

pub fn content_starts_with_heading(nodes: &[ContentNode], title: &str) -> bool {
    nodes.first().is_some_and(|node| match node {
        ContentNode::Heading { spans, .. } => {
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
                .trim()
                == title.trim()
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_test_cell(
        text: &str,
        width: Option<shosai_core::epub::render::NodeWidth>,
    ) -> shosai_core::epub::render::TableCell {
        use shosai_core::epub::render::{NodeStyle, TableCell, TextSpan};

        TableCell {
            id: None,
            header: false,
            scope: None,
            headers: Vec::new(),
            row_span: 1,
            column_span: 1,
            children: vec![ContentNode::Paragraph(
                vec![TextSpan {
                    text: text.into(),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: true,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: true,
                    link: None,
                }],
                NodeStyle::default(),
            )],
            block_starts: Vec::new(),
            style: NodeStyle {
                width,
                ..Default::default()
            },
        }
    }

    fn one_line_table(rows: usize) -> ContentNode {
        use shosai_core::epub::render::{
            NodeStyle, TableCell, TableRow, TableRowGroup, TableRowGroupKind, TextSpan,
        };

        let paragraph = || {
            ContentNode::Paragraph(
                vec![TextSpan {
                    text: "cell".into(),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: None,
                }],
                NodeStyle::default(),
            )
        };
        ContentNode::Table {
            caption: Vec::new(),
            caption_style: None,
            row_groups: vec![TableRowGroup {
                kind: TableRowGroupKind::Body,
                rows: (0..rows)
                    .map(|_| TableRow {
                        cells: vec![TableCell {
                            id: None,
                            header: false,
                            scope: None,
                            headers: Vec::new(),
                            row_span: 1,
                            column_span: 1,
                            children: vec![paragraph()],
                            block_starts: Vec::new(),
                            style: NodeStyle::default(),
                        }],
                    })
                    .collect(),
            }],
            style: NodeStyle::default(),
        }
    }

    #[test]
    fn nested_blockquote_and_paragraph_margins_reduce_shaping_width() {
        let block = shosai_core::epub::render::NodeStyle {
            margin_left_em: Some(2.0),
            ..Default::default()
        };
        let paragraph = shosai_core::epub::render::NodeStyle {
            margin_left_em: Some(1.0),
            ..Default::default()
        };
        let inner = blockquote_width(240.0, 16.0, &block);
        assert_eq!(inner, 208.0);
        assert_eq!(blockquote_width(inner, 16.0, &Default::default()), 192.0);
        assert_eq!(paragraph_width(192.0, 16.0, &paragraph), 176.0);
        assert_eq!(
            blockquote_continuation_page_size(Size::new(240.0, 320.0), 16.0, &block),
            Size::new(208.0, 320.0),
            "continued quote pages must retain their effective inner width"
        );

        let child = ContentNode::Paragraph(
            vec![shosai_core::epub::render::TextSpan {
                text: "x".repeat(25),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 2.0,
                preserve_whitespace: false,
                link: None,
            }],
            shosai_core::epub::render::NodeStyle {
                margin_left_em: Some(10.0),
                ..Default::default()
            },
        );
        let wide = estimated_epub_blockquote_height(
            std::slice::from_ref(&child),
            &Default::default(),
            27,
            20,
            16.0,
            240.0,
            320.0,
        );
        let narrow = estimated_epub_blockquote_height(
            std::slice::from_ref(&child),
            &block,
            27,
            20,
            16.0,
            240.0,
            320.0,
        );
        assert!(narrow > wide);

        let available_height = 16.0 * TEXT_LINE_HEIGHT * 2.0;
        let (prefix, remaining, prefix_height, _) = split_epub_blockquote_prefix(
            std::slice::from_ref(&child),
            available_height,
            20,
            16.0,
            208.0,
            None,
            &EpubPaginationBudget::default(),
        )
        .unwrap();
        assert!(!prefix.is_empty());
        assert!(!remaining.is_empty());
        assert!(prefix_height <= available_height);
        assert_eq!(
            prefix.iter().map(content_node_text_len).sum::<usize>()
                + remaining.iter().map(content_node_text_len).sum::<usize>(),
            content_node_text_len(&child)
        );
    }

    #[test]
    fn authored_block_spacing_overrides_reader_default() {
        let node = ContentNode::Paragraph(
            Vec::new(),
            shosai_core::epub::render::NodeStyle {
                block_before_em: Some(2.0),
                block_after_em: Some(0.0),
                ..Default::default()
            },
        );

        assert_eq!(epub_node_block_sides(&node, 16.0, 20.0), (32.0, 0.0));
        assert_eq!(
            epub_node_block_sides(
                &ContentNode::Paragraph(
                    Vec::new(),
                    shosai_core::epub::render::NodeStyle::default(),
                ),
                16.0,
                20.0,
            ),
            (0.0, 20.0)
        );
    }

    #[test]
    fn authored_margins_collapse_at_outer_and_adjacent_boundaries() {
        use shosai_core::epub::render::NodeStyle;
        let paragraph = |before, after| {
            ContentNode::Paragraph(
                Vec::new(),
                NodeStyle {
                    block_before_em: Some(before),
                    block_after_em: Some(after),
                    ..NodeStyle::default()
                },
            )
        };
        let nodes = vec![paragraph(1.0, 0.0), paragraph(2.0, 3.0)];
        assert_eq!(epub_node_boundary_spacing(&nodes, 0, 10.0, 16.0), 10.0);
        assert_eq!(epub_node_boundary_spacing(&nodes, 1, 10.0, 16.0), 20.0);
        assert_eq!(epub_node_boundary_spacing(&nodes, 2, 10.0, 16.0), 30.0);
    }

    #[test]
    fn following_top_margin_is_between_blocks_not_after_the_following_block() {
        use shosai_core::epub::render::NodeStyle;
        let nodes = vec![
            ContentNode::Paragraph(
                Vec::new(),
                NodeStyle {
                    block_after_em: Some(0.0),
                    ..NodeStyle::default()
                },
            ),
            ContentNode::Paragraph(
                Vec::new(),
                NodeStyle {
                    block_before_em: Some(2.0),
                    block_after_em: Some(0.0),
                    ..NodeStyle::default()
                },
            ),
        ];
        assert_eq!(epub_node_boundary_spacing(&nodes, 1, 16.0, 20.0), 32.0);
        assert_eq!(epub_node_boundary_spacing(&nodes, 2, 16.0, 20.0), 0.0);
    }

    #[test]
    fn pagination_stores_authored_boundaries_for_every_block_kind() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        use shosai_core::epub::{MathContent, MathDisplay};
        let span = |text: &str| TextSpan {
            text: text.into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        let style = NodeStyle {
            block_before_em: Some(0.5),
            block_after_em: Some(0.75),
            ..Default::default()
        };
        let nodes = vec![
            ContentNode::Math {
                content: MathContent {
                    display: MathDisplay::Block,
                    expression: None,
                    fallback: "x".into(),
                },
                style: style.clone(),
                link: None,
            },
            ContentNode::BlockQuote {
                children: vec![ContentNode::Paragraph(
                    vec![span("quote")],
                    Default::default(),
                )],
                style: style.clone(),
            },
            {
                let mut table = one_line_table(1);
                if let ContentNode::Table {
                    style: table_style, ..
                } = &mut table
                {
                    *table_style = style.clone();
                }
                table
            },
            ContentNode::Paragraph(vec![span("Text")], style),
        ];
        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(600.0, 800.0));
        let fragments = &pages[0];
        assert_eq!(fragments.len(), 4);
        assert_eq!(fragments[0].block_before, 8.0);
        assert!(fragments.windows(2).all(|pair| pair[0].block_after == 12.0));
        assert_eq!(fragments[3].block_after, 12.0);
    }

    #[test]
    fn large_heading_margin_forces_break_and_is_truncated_at_page_edge() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        let span = |text: &str| TextSpan {
            text: text.into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        // Regression for: <h1 style="margin-bottom:200px">Title</h1><p>Text</p>
        let nodes = vec![
            ContentNode::Heading {
                level: 1,
                spans: vec![span("Title")],
                style: NodeStyle {
                    block_after_em: Some(12.5),
                    ..Default::default()
                },
            },
            ContentNode::Paragraph(vec![span("Text")], Default::default()),
        ];
        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0][0].block_after, 0.0);
        assert_eq!(pages[1][0].block_before, 0.0);
    }

    #[test]
    fn declared_system_families_do_not_select_the_embedded_font_renderer() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/sample.epub").to_vec(),
        )
        .expect("fixture should be a valid EPUB");
        assert!(epub.fonts().is_empty());
        let spans = vec![shosai_core::epub::render::TextSpan {
            text: "This must remain visible".into(),
            math: None,
            font_family: Some("serif".into()),
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];

        assert!(!uses_native_fonts(epub.fonts(), &spans));
    }

    #[test]
    fn semantic_table_lengths_match_shared_search_offsets() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/table.epub").to_vec(),
        )
        .expect("table fixture should be a valid EPUB");
        let chapter = epub.presentation().chapter(0).unwrap();
        let retained = chapter
            .nodes()
            .iter()
            .map(content_node_text_len)
            .sum::<usize>()
            + chapter.nodes().len();

        assert_eq!(retained, chapter.search_text().chars().count());
        assert_eq!(
            chapter
                .nodes()
                .iter()
                .filter(|node| matches!(node, ContentNode::Table { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn table_math_admission_uses_the_padded_cell_width() {
        use shosai_core::epub::render::{
            NodeStyle, TableCell, TableRow, TableRowGroupKind, TextSpan,
        };
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let token = "abcdefghijklmnopqrstuvwxyzabcdefghij";
        let fallback = format!("({token})/({token})");
        let math = TextSpan {
            text: fallback.clone(),
            math: Some(MathContent {
                display: MathDisplay::Inline,
                expression: Some(MathExpression::Fraction(
                    Box::new(MathExpression::Token(token.into())),
                    Box::new(MathExpression::Token(token.into())),
                )),
                fallback: fallback.clone(),
            }),
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        let paragraph = |span: TextSpan| ContentNode::Paragraph(vec![span], NodeStyle::default());
        let cell = |child| TableCell {
            id: None,
            header: false,
            scope: None,
            headers: Vec::new(),
            row_span: 1,
            column_span: 1,
            children: vec![child],
            block_starts: Vec::new(),
            style: NodeStyle::default(),
        };
        let second = TextSpan {
            text: fallback.clone(),
            math: None,
            ..math.clone()
        };
        let table = |first| ContentNode::Table {
            caption: Vec::new(),
            caption_style: None,
            row_groups: vec![TableRowGroup {
                kind: TableRowGroupKind::Body,
                rows: vec![TableRow {
                    cells: vec![cell(first), cell(paragraph(second.clone()))],
                }],
            }],
            style: NodeStyle::default(),
        };
        let outer_width = 400.0;
        let inner_width = (outer_width - BLOCKQUOTE_SPACING) / 2.0 - 2.0 * EPUB_TABLE_CELL_PADDING;
        assert!(layout_inline_math_span(&math, 16.0, outer_width, 300.0).is_some());
        assert!(layout_inline_math_span(&math, 16.0, inner_width, 300.0).is_none());
        let native = table(paragraph(math.clone()));
        let mut fallback_span = math;
        fallback_span.math = None;
        let fallback = table(paragraph(fallback_span));
        let chars_per_line = (outer_width / (16.0 * AVERAGE_CHARACTER_WIDTH)) as usize;

        assert_eq!(
            estimated_epub_compact_node_height(&native, chars_per_line, 20, 16.0),
            estimated_epub_compact_node_height(&fallback, chars_per_line, 20, 16.0),
            "math that does not fit the padded cell must use the same fallback measurement as painting"
        );
    }

    #[test]
    fn table_display_math_height_uses_native_geometry() {
        use shosai_core::epub::render::{
            NodeStyle, TableCell, TableRow, TableRowGroup, TableRowGroupKind,
        };
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let expression = MathExpression::Fraction(
            Box::new(MathExpression::Token("a".into())),
            Box::new(MathExpression::SquareRoot(vec![MathExpression::Token(
                "b".into(),
            )])),
        );
        let table = ContentNode::Table {
            caption: Vec::new(),
            caption_style: None,
            row_groups: vec![TableRowGroup {
                kind: TableRowGroupKind::Body,
                rows: vec![TableRow {
                    cells: vec![TableCell {
                        id: None,
                        header: false,
                        scope: None,
                        headers: Vec::new(),
                        row_span: 1,
                        column_span: 1,
                        children: vec![ContentNode::Math {
                            content: MathContent {
                                display: MathDisplay::Block,
                                expression: Some(expression.clone()),
                                fallback: "(a)/(sqrt(b))".into(),
                            },
                            style: NodeStyle::default(),
                            link: None,
                        }],
                        block_starts: Vec::new(),
                        style: NodeStyle::default(),
                    }],
                }],
            }],
            style: NodeStyle::default(),
        };
        let font_size = 16.0;
        let outer_width = 360.0;
        let cell_width = outer_width - 2.0 * EPUB_TABLE_CELL_PADDING;
        let native_height =
            math_layout::layout_math_for_bounds(&expression, font_size, cell_width, 240.0)
                .expect("fixture math should fit the table cell")
                .height;
        let chars_per_line = (outer_width / (font_size * AVERAGE_CHARACTER_WIDTH)).floor() as usize;

        assert!(
            estimated_epub_compact_node_height(&table, chars_per_line, 20, font_size)
                >= native_height + 2.0 * EPUB_TABLE_CELL_PADDING,
            "table pagination must reserve the native display-math height painted in the cell"
        );
    }

    #[test]
    fn oversized_inline_math_flow_falls_back_before_pagination_layout() {
        use shosai_core::epub::render::TextSpan;
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let mut spans = (0..256)
            .map(|_| TextSpan {
                text: "word ".into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            })
            .collect::<Vec<_>>();
        spans.push(TextSpan {
            text: "(a)/(b)".into(),
            math: Some(MathContent {
                display: MathDisplay::Inline,
                expression: Some(MathExpression::Fraction(
                    Box::new(MathExpression::Token("a".into())),
                    Box::new(MathExpression::Token("b".into())),
                )),
                fallback: "(a)/(b)".into(),
            }),
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: Some("chapter.xhtml#proof".into()),
        });
        let pagination = pagination_inline_spans(
            &spans,
            16.0,
            360.0,
            500.0,
            shosai_core::epub::style::TextDirection::Ltr,
            None,
        );

        assert!(pagination.iter().all(|span| span.math.is_none()));
        assert_eq!(
            pagination.last().and_then(|span| span.link.as_deref()),
            Some("chapter.xhtml#proof"),
            "aggregate fallback must retain source links"
        );
        assert_eq!(spans_text_len(&pagination), spans_text_len(&spans));
    }

    #[test]
    fn standalone_math_pagination_uses_the_native_painted_height() {
        use shosai_core::epub::render::NodeStyle;
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let expression = MathExpression::Fraction(
            Box::new(MathExpression::Token("a".into())),
            Box::new(MathExpression::SquareRoot(vec![MathExpression::Token(
                "b".into(),
            )])),
        );
        let node = ContentNode::Math {
            content: MathContent {
                display: MathDisplay::Block,
                expression: Some(expression.clone()),
                fallback: "(a)/(sqrt(b))".into(),
            },
            style: NodeStyle::default(),
            link: None,
        };
        let native = math_layout::layout_math_for_bounds(&expression, 20.0, 600.0, 700.0)
            .expect("supported standalone math should use native geometry");

        assert_eq!(
            measured_epub_compact_node_height(None, &node, 20.0, 600.0),
            Some(native.height),
            "pagination and painting must consume the same geometry"
        );
        assert_eq!(
            content_node_text_len(&node),
            "(a)/(sqrt(b))".chars().count(),
            "native presentation must not change shared source offsets"
        );
    }

    #[test]
    fn inline_math_is_atomic_and_uses_shared_geometry_during_pagination() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let text_span = |text: &str| TextSpan {
            text: text.into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        let fallback = "(numerator)/(denominator)";
        let math_span = TextSpan {
            text: fallback.into(),
            math: Some(MathContent {
                display: MathDisplay::Inline,
                expression: Some(MathExpression::Fraction(
                    Box::new(MathExpression::Token("numerator".into())),
                    Box::new(MathExpression::Token("denominator".into())),
                )),
                fallback: fallback.into(),
            }),
            ..text_span("")
        };
        let spans = vec![
            text_span(&"before ".repeat(24)),
            math_span.clone(),
            text_span(&" after".repeat(24)),
        ];
        let plain = ContentNode::Paragraph(
            spans
                .iter()
                .cloned()
                .map(|mut span| {
                    span.math = None;
                    span
                })
                .collect(),
            NodeStyle::default(),
        );
        let paragraph = ContentNode::Paragraph(spans.clone(), NodeStyle::default());
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&paragraph),
            None,
            16.0,
            1.6,
            Size::new(180.0, 150.0),
        );
        let fragments = pages.iter().flatten().collect::<Vec<_>>();
        let retained_math = fragments
            .iter()
            .filter_map(|page_node| match &page_node.node {
                ContentNode::Paragraph(spans, _) => spans.iter().find(|span| span.math.is_some()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(retained_math, vec![&math_span]);
        assert_eq!(
            fragments
                .iter()
                .flat_map(|page_node| match &page_node.node {
                    ContentNode::Paragraph(spans, _) => spans,
                    _ => unreachable!(),
                })
                .map(|span| span.text.as_str())
                .collect::<String>(),
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
        );
        let mut expected_offset = 0;
        for page_node in fragments {
            assert_eq!(page_node.text_offset, expected_offset);
            expected_offset += content_node_text_len(&page_node.node);
        }
        let layout = layout_inline_math_span(&math_span, 16.0, 180.0, 150.0)
            .expect("supported inline math must retain native geometry");
        assert!(layout.height > 16.0 * TEXT_LINE_HEIGHT);
        let geometry_extra = layout.height - 16.0 * TEXT_LINE_HEIGHT;
        assert!(
            inline_math_height_reserve(&spans, 16.0, 180.0, 150.0)
                >= geometry_extra + 16.0 * INLINE_MATH_WRAP_SPACING,
            "wrapped native math must reserve inter-line clearance in addition to geometry"
        );
        let mut display_span = math_span.clone();
        display_span.math.as_mut().unwrap().display = MathDisplay::Block;
        assert!(layout_inline_math_span(&display_span, 16.0, 180.0, 150.0).is_none());
        assert!(
            estimated_epub_compact_node_height(&paragraph, 20, 10, 16.0)
                > estimated_epub_compact_node_height(&plain, 20, 10, 16.0)
        );
    }

    #[test]
    fn native_inline_admission_controls_atomic_pagination_and_reserve() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        use shosai_core::epub::style::{TextAlignment, TextDirection};
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let math_span = |expression: MathExpression, fallback: String| TextSpan {
            text: fallback.clone(),
            math: Some(MathContent {
                display: MathDisplay::Inline,
                expression: Some(expression),
                fallback,
            }),
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        let fraction = || {
            math_span(
                MathExpression::Fraction(
                    Box::new(MathExpression::Token("a".into())),
                    Box::new(MathExpression::Token("b".into())),
                ),
                "(a)/(b)".into(),
            )
        };
        let retained_math = |node: &ContentNode| match node {
            ContentNode::Paragraph(spans, _) => {
                spans.iter().filter(|span| span.math.is_some()).count()
            }
            _ => 0,
        };
        let page_size = Size::new(180.0, 120.0);
        let fallback_cases = [
            (
                fraction(),
                NodeStyle {
                    direction: TextDirection::Rtl,
                    ..Default::default()
                },
            ),
            (
                fraction(),
                NodeStyle {
                    text_align: Some(TextAlignment::Justify),
                    ..Default::default()
                },
            ),
            (
                math_span(
                    MathExpression::Token("overwide".repeat(80)),
                    "overwide".repeat(80),
                ),
                NodeStyle::default(),
            ),
            (
                math_span(
                    MathExpression::Fraction(
                        Box::new(MathExpression::Fraction(
                            Box::new(MathExpression::Token("a".into())),
                            Box::new(MathExpression::Token("b".into())),
                        )),
                        Box::new(MathExpression::Fraction(
                            Box::new(MathExpression::Token("c".into())),
                            Box::new(MathExpression::Token("d".into())),
                        )),
                    ),
                    "((a)/(b))/((c)/(d))".into(),
                ),
                NodeStyle::default(),
            ),
            (
                math_span(MathExpression::Token("\u{10ffff}".into()), "missing".into()),
                NodeStyle::default(),
            ),
        ];

        for (span, style) in fallback_cases {
            let height = if span.text == "((a)/(b))/((c)/(d))" {
                20.0
            } else {
                page_size.height
            };
            let pages = paginate_epub_chapter(
                &[ContentNode::Paragraph(vec![span], style)],
                None,
                16.0,
                1.6,
                Size::new(page_size.width, height),
            );
            assert_eq!(
                pages
                    .iter()
                    .flatten()
                    .map(|page| retained_math(&page.node))
                    .sum::<usize>(),
                0,
                "fallback presentation must remain splittable instead of carrying atomic geometry"
            );
        }

        let native = ContentNode::Paragraph(vec![fraction()], NodeStyle::default());
        let native_pages = paginate_epub_chapter(&[native], None, 16.0, 1.6, page_size);
        assert_eq!(
            native_pages
                .iter()
                .flatten()
                .map(|page| retained_math(&page.node))
                .sum::<usize>(),
            1,
            "admitted native geometry must remain one atomic span"
        );
    }

    #[test]
    fn deeply_nested_inline_fraction_uses_readable_fallback() {
        use shosai_core::epub::render::TextSpan;
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let token = |text: &str| MathExpression::Token(text.into());
        let fraction = |top, bottom| MathExpression::Fraction(Box::new(top), Box::new(bottom));
        let expression = fraction(
            fraction(
                fraction(token("a"), token("b")),
                fraction(token("c"), token("d")),
            ),
            fraction(
                fraction(token("e"), token("f")),
                fraction(token("g"), token("h")),
            ),
        );
        let fallback = "(((a)/(b))/((c)/(d)))/(((e)/(f))/((g)/(h)))";
        let span = TextSpan {
            text: fallback.into(),
            math: Some(MathContent {
                display: MathDisplay::Inline,
                expression: Some(expression),
                fallback: fallback.into(),
            }),
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };

        assert!(
            layout_inline_math_span(&span, 20.0, 388.0, 500.0).is_none(),
            "inline geometry spanning several text lines must use readable fallback even when it fits the page"
        );
        let pages = paginate_epub_chapter(
            &[ContentNode::Paragraph(vec![span], Default::default())],
            None,
            20.0,
            1.6,
            Size::new(388.0, 500.0),
        );
        assert!(pages.iter().flatten().all(|page| {
            matches!(
                &page.node,
                ContentNode::Paragraph(spans, _)
                    if spans.iter().all(|span| span.math.is_none())
            )
        }));
    }

    #[test]
    fn paragraph_pagination_accounts_for_inline_geometry_at_page_boundaries() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let plain = |text: String| TextSpan {
            text,
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        let mut spans = Vec::new();
        for index in 0..8 {
            spans.push(plain(format!("segment {index} before ")));
            spans.push(TextSpan {
                text: "(a)/(b)".into(),
                math: Some(MathContent {
                    display: MathDisplay::Inline,
                    expression: Some(MathExpression::Fraction(
                        Box::new(MathExpression::Token("a".into())),
                        Box::new(MathExpression::Token("b".into())),
                    )),
                    fallback: "(a)/(b)".into(),
                }),
                ..plain(String::new())
            });
            spans.push(plain(" after. ".into()));
        }
        let plain_spans = spans
            .iter()
            .cloned()
            .map(|mut span| {
                span.math = None;
                span
            })
            .collect::<Vec<_>>();
        let source = spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        let size = Size::new(180.0, 120.0);
        let pages = paginate_epub_chapter(
            &[ContentNode::Paragraph(spans, NodeStyle::default())],
            None,
            20.0,
            1.6,
            size,
        );
        let plain_pages = paginate_epub_chapter(
            &[ContentNode::Paragraph(plain_spans, NodeStyle::default())],
            None,
            20.0,
            1.6,
            size,
        );

        assert!(
            pages.len() > plain_pages.len(),
            "native geometry and wrap clearance must affect actual page placement"
        );
        let fragments = pages.iter().flatten().collect::<Vec<_>>();
        assert_eq!(
            fragments
                .iter()
                .flat_map(|page| match &page.node {
                    ContentNode::Paragraph(spans, _) => spans,
                    _ => unreachable!(),
                })
                .map(|span| span.text.as_str())
                .collect::<String>(),
            source
        );
        let mut expected_offset = 0;
        for page in fragments {
            assert_eq!(page.text_offset, expected_offset);
            expected_offset += content_node_text_len(&page.node);
        }
    }

    #[test]
    fn blockquote_splits_do_not_duplicate_fallback_math_metadata() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let fallback = "fallback ".repeat(80);
        let math = TextSpan {
            text: fallback.clone(),
            math: Some(MathContent {
                display: MathDisplay::Inline,
                expression: Some(MathExpression::Token(fallback.clone())),
                fallback: fallback.clone(),
            }),
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        let pages = paginate_epub_chapter(
            &[ContentNode::BlockQuote {
                children: vec![ContentNode::Paragraph(vec![math], NodeStyle::default())],
                style: NodeStyle::default(),
            }],
            None,
            16.0,
            1.6,
            Size::new(180.0, 120.0),
        );
        let mut text = String::new();
        let mut retained_math = 0;
        for page in pages.iter().flatten() {
            let ContentNode::BlockQuote { children, .. } = &page.node else {
                unreachable!();
            };
            for child in children {
                if let ContentNode::Paragraph(spans, _) = child {
                    text.extend(spans.iter().map(|span| span.text.as_str()));
                    retained_math += spans.iter().filter(|span| span.math.is_some()).count();
                }
            }
        }
        assert_eq!(text, fallback);
        assert_eq!(
            retained_math, 0,
            "splittable fallback must not clone native metadata"
        );
    }

    #[test]
    fn unsupported_or_overwide_math_keeps_the_readable_text_path() {
        use shosai_core::epub::render::NodeStyle;
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let unsupported = ContentNode::Math {
            content: MathContent {
                display: MathDisplay::Block,
                expression: None,
                fallback: "readable fallback".into(),
            },
            style: NodeStyle::default(),
            link: None,
        };
        let overwide = ContentNode::Math {
            content: MathContent {
                display: MathDisplay::Block,
                expression: Some(MathExpression::Token("wide expression".into())),
                fallback: "wide expression".into(),
            },
            style: NodeStyle::default(),
            link: None,
        };

        assert!(
            estimated_epub_compact_node_height(&unsupported, 40, 20, 20.0) > 0.0,
            "unsupported math must remain measurable through its fallback"
        );
        let ContentNode::Math { content, .. } = &overwide else {
            unreachable!();
        };
        assert!(
            math_layout::layout_math_for_bounds(
                content.expression.as_ref().unwrap(),
                20.0,
                1.0,
                700.0,
            )
            .is_none()
        );
        assert!(estimated_epub_compact_node_height(&overwide, 1, 20, 20.0) > 0.0);
    }

    #[test]
    fn dense_math_sequence_moves_complete_matrix_and_fallback_between_pages() {
        use shosai_core::epub::render::{NodeStyle, TextSpan};
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let label = |text: &str| {
            ContentNode::Paragraph(
                vec![TextSpan {
                    text: text.into(),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: None,
                }],
                NodeStyle::default(),
            )
        };
        let math = |expression: Option<MathExpression>, fallback: &str| ContentNode::Math {
            content: MathContent {
                display: MathDisplay::Block,
                expression,
                fallback: fallback.into(),
            },
            style: NodeStyle {
                font_size_multiplier: Some(1.5),
                ..NodeStyle::default()
            },
            link: None,
        };
        let token = |text: &str| MathExpression::Token(text.into());
        let matrix = MathExpression::Fenced {
            open: "(".into(),
            close: ")".into(),
            content: vec![MathExpression::Table(vec![
                vec![token("1"), token("0")],
                vec![token("0"), token("1")],
            ])],
        };
        let nodes = vec![
            label("Native MathML geometry — fraction:"),
            math(
                Some(MathExpression::Fraction(
                    Box::new(MathExpression::Row(vec![
                        token("a"),
                        token("+"),
                        token("b"),
                    ])),
                    Box::new(MathExpression::Row(vec![
                        token("c"),
                        token("+"),
                        token("d"),
                    ])),
                )),
                "(a+b)/(c+d)",
            ),
            label("Indexed root and sub/superscript:"),
            math(
                Some(MathExpression::Row(vec![
                    MathExpression::Root(Box::new(token("x")), Box::new(token("3"))),
                    token("+"),
                    MathExpression::SubSuperscript {
                        base: Box::new(token("y")),
                        subscript: Box::new(token("i")),
                        superscript: Box::new(token("2")),
                    },
                ])),
                "root(x, 3) + y_i^2",
            ),
            label("Fence:"),
            math(
                Some(MathExpression::Fenced {
                    open: "[".into(),
                    close: "]".into(),
                    content: vec![MathExpression::Row(vec![
                        token("p"),
                        token("+"),
                        token("q"),
                    ])],
                }),
                "[p + q]",
            ),
            label("Fenced 2 x 2 matrix:"),
            math(Some(matrix.clone()), "(1 0; 0 1)"),
            label("Unsupported case remains readable:"),
            math(None, "Readable unsupported fallback"),
        ];
        let page_size = Size::new(420.0, 500.0);
        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, page_size);

        assert!(
            pages.len() > 1,
            "dense fixture must exercise a page boundary"
        );
        assert_eq!(
            pages.iter().map(Vec::len).sum::<usize>(),
            nodes.len(),
            "pagination must retain every label, expression, and fallback"
        );
        let mut expected_offset = 0;
        for (page_node, source_node) in pages.iter().flatten().zip(&nodes) {
            assert_eq!(page_node.text_offset, expected_offset);
            assert_eq!(&page_node.node, source_node);
            expected_offset += content_node_text_len(source_node) + 1;
        }
        let matrix_page = pages
            .iter()
            .position(|page| {
                page.iter().any(|node| {
                    matches!(
                        &node.node,
                        ContentNode::Math { content, .. }
                            if content.expression.as_ref() == Some(&matrix)
                    )
                })
            })
            .expect("matrix must remain on one page");
        assert!(matches!(
            pages[matrix_page].as_slice(),
            [
                PageNode {
                    node: ContentNode::Paragraph(spans, _),
                    ..
                },
                PageNode {
                    node: ContentNode::Math { content, .. },
                    ..
                },
                ..
            ] if spans.iter().any(|span| span.text == "Fenced 2 x 2 matrix:")
                && content.expression.as_ref() == Some(&matrix)
        ));
        let matrix_layout = math_layout::layout_math_for_bounds(&matrix, 24.0, 420.0, 500.0)
            .expect("matrix must retain native geometry");
        assert!(matrix_layout.primitives.iter().all(|primitive| {
            primitive.x >= 0.0
                && primitive.y >= 0.0
                && primitive.x + primitive.width <= matrix_layout.width
                && primitive.y + primitive.height <= matrix_layout.height
        }));
        for value in ["1", "0"] {
            assert!(matrix_layout.primitives.iter().any(|primitive| {
                matches!(&primitive.kind, math_layout::MathPrimitiveKind::Text(text) if text == value)
            }));
        }
        let zero_rows = matrix_layout
            .primitives
            .iter()
            .filter(|primitive| {
                matches!(&primitive.kind, math_layout::MathPrimitiveKind::Text(text) if text == "0")
            })
            .map(|primitive| primitive.y)
            .collect::<Vec<_>>();
        assert_eq!(zero_rows.len(), 2);
        assert_ne!(
            zero_rows[0], zero_rows[1],
            "both matrix rows must be positioned"
        );
        assert!(
            pages[matrix_page..]
                .iter()
                .any(|page| page.iter().any(|node| {
                    matches!(
                        &node.node,
                        ContentNode::Math { content, .. }
                            if content.fallback == "Readable unsupported fallback"
                    )
                }))
        );
    }

    #[test]
    fn paginated_tables_split_only_between_complete_rowspan_bands() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/table.epub").to_vec(),
        )
        .expect("table fixture should be a valid EPUB");
        let table = epub
            .presentation()
            .chapter(0)
            .unwrap()
            .nodes()
            .iter()
            .find(|node| matches!(node, ContentNode::Table { .. }))
            .expect("fixture must retain a semantic table");
        let expected_rows = match table {
            ContentNode::Table { row_groups, .. } => row_groups
                .iter()
                .map(|group| group.rows.len())
                .sum::<usize>(),
            _ => unreachable!(),
        };

        let pages = paginate_epub_chapter(
            std::slice::from_ref(table),
            None,
            16.0,
            1.4,
            Size::new(240.0, 72.0),
        );
        let fragments = pages
            .iter()
            .flatten()
            .map(|page_node| (&page_node.node, page_node.text_offset))
            .collect::<Vec<_>>();

        assert!(pages.len() > 1, "tall tables must fragment by row bands");
        assert_eq!(
            fragments
                .iter()
                .map(|(node, _)| match node {
                    ContentNode::Table { row_groups, .. } => row_groups
                        .iter()
                        .map(|group| group.rows.len())
                        .sum::<usize>(),
                    ContentNode::Paragraph(..) => 0,
                    other => panic!("expected table or caption fragment, got {other:?}"),
                })
                .sum::<usize>(),
            expected_rows
        );
        for (node, _) in &fragments {
            let ContentNode::Table { row_groups, .. } = node else {
                continue;
            };
            for group in row_groups {
                for (row_index, row) in group.rows.iter().enumerate() {
                    let required_rows = row
                        .cells
                        .iter()
                        .map(|cell| {
                            if cell.row_span == 0 {
                                group.rows.len() - row_index
                            } else {
                                usize::from(cell.row_span)
                            }
                        })
                        .max()
                        .unwrap_or(1);
                    assert!(row_index + required_rows <= group.rows.len());
                }
            }
        }
        for pair in fragments.windows(2) {
            let expected = pair[0].1 + content_node_text_len(pair[0].0);
            assert!(
                pair[1].1 == expected || pair[1].1 == expected + 1,
                "a detached caption may leave only its table separator between fragments"
            );
        }
        let (last, last_offset) = fragments.last().expect("table must produce fragments");
        assert_eq!(
            last_offset + content_node_text_len(last),
            content_node_text_len(table)
        );
    }

    #[test]
    fn table_height_estimation_includes_rendered_padding_and_spacing() {
        let one_row = one_line_table(1);
        let height = estimated_epub_compact_node_height(&one_row, 40, 20, 16.0);

        assert!(
            (height - (16.0 * TEXT_LINE_HEIGHT + 16.0)).abs() < 0.001,
            "estimated table height was {height}"
        );
    }

    #[test]
    fn nested_authored_margins_replace_container_default_spacing() {
        let paragraph = |before, after| {
            let mut cell = table_test_cell("content", None);
            let ContentNode::Paragraph(_, style) = &mut cell.children[0] else {
                unreachable!();
            };
            style.block_before_em = before;
            style.block_after_em = after;
            cell.children.remove(0)
        };
        let plain = paragraph(None, None);
        let authored = paragraph(Some(2.0), Some(3.0));

        assert_eq!(
            epub_node_list_spacing(std::slice::from_ref(&plain), 16.0, 8.0),
            8.0
        );
        assert_eq!(
            epub_node_list_spacing(std::slice::from_ref(&authored), 16.0, 8.0),
            80.0
        );

        let plain_quote = ContentNode::BlockQuote {
            children: vec![plain.clone()],
            style: Default::default(),
        };
        let authored_quote = ContentNode::BlockQuote {
            children: vec![authored.clone()],
            style: Default::default(),
        };
        assert_eq!(
            estimated_epub_compact_node_height(&authored_quote, 40, 20, 16.0)
                - estimated_epub_compact_node_height(&plain_quote, 40, 20, 16.0),
            72.0
        );

        let mut plain_table = one_line_table(1);
        let mut authored_table = plain_table.clone();
        let ContentNode::Table { row_groups, .. } = &mut plain_table else {
            unreachable!();
        };
        row_groups[0].rows[0].cells[0].children = vec![plain];
        let ContentNode::Table { row_groups, .. } = &mut authored_table else {
            unreachable!();
        };
        row_groups[0].rows[0].cells[0].children = vec![authored];
        assert_eq!(
            estimated_epub_compact_node_height(&authored_table, 40, 20, 16.0)
                - estimated_epub_compact_node_height(&plain_table, 40, 20, 16.0),
            76.0
        );
    }

    #[test]
    fn fitting_rowspan_bands_share_one_paginated_table_surface() {
        let table = one_line_table(3);
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&table),
            None,
            16.0,
            1.4,
            Size::new(360.0, 300.0),
        );
        let fragments = pages
            .iter()
            .flatten()
            .filter(|page_node| matches!(page_node.node, ContentNode::Table { .. }))
            .collect::<Vec<_>>();

        assert_eq!(fragments.len(), 1);
        assert_eq!(
            content_node_text_len(&fragments[0].node),
            content_node_text_len(&table)
        );
    }

    #[test]
    fn table_bands_merge_only_within_the_same_source_row_group() {
        let mut fragment = one_line_table(1);
        let ContentNode::Table { row_groups, .. } = &fragment else {
            unreachable!();
        };
        let rows = row_groups[0].rows.clone();
        append_table_band(
            &mut fragment,
            shosai_core::epub::render::TableRowGroupKind::Body,
            &rows,
            false,
        );
        let ContentNode::Table { row_groups, .. } = &fragment else {
            unreachable!();
        };
        assert_eq!(row_groups.len(), 2);

        let mut same_group = one_line_table(1);
        append_table_band(
            &mut same_group,
            shosai_core::epub::render::TableRowGroupKind::Body,
            &rows,
            true,
        );
        let ContentNode::Table { row_groups, .. } = &same_group else {
            unreachable!();
        };
        assert_eq!(row_groups.len(), 1);
        assert_eq!(row_groups[0].rows.len(), 2);
    }

    #[test]
    fn exhausted_table_budget_appends_remaining_bands_without_losing_rows() {
        let table = one_line_table(200);
        let mut budget = EpubPaginationBudget {
            remaining_page_breaks: 1,
            ..Default::default()
        };
        let pages = paginate_epub_chapter_with_budget(
            std::slice::from_ref(&table),
            None,
            16.0,
            1.4,
            Size::new(360.0, 60.0),
            None,
            &mut budget,
        );
        let rows = pages
            .iter()
            .flatten()
            .filter_map(|page_node| match &page_node.node {
                ContentNode::Table { row_groups, .. } => Some(
                    row_groups
                        .iter()
                        .map(|group| group.rows.len())
                        .sum::<usize>(),
                ),
                _ => None,
            })
            .sum::<usize>();

        assert_eq!(pages.len(), 2);
        assert_eq!(rows, 200);
        assert_eq!(budget.remaining_page_breaks, 0);
    }

    #[test]
    fn table_pagination_charges_authored_spacing_only_to_the_final_fragment() {
        let paragraph = table_test_cell("following", None).children.remove(0);
        let mut compact_table = one_line_table(1);
        let ContentNode::Table { style, .. } = &mut compact_table else {
            unreachable!();
        };
        style.block_after_em = Some(0.0);
        let compact_pages = paginate_epub_chapter(
            &[compact_table, paragraph.clone()],
            None,
            16.0,
            1.4,
            Size::new(360.0, 120.0),
        );
        assert_eq!(compact_pages.len(), 1);

        let mut spaced_table = one_line_table(1);
        let ContentNode::Table { style, .. } = &mut spaced_table else {
            unreachable!();
        };
        style.block_after_em = Some(4.0);
        let spaced_pages = paginate_epub_chapter(
            &[spaced_table, paragraph.clone()],
            None,
            16.0,
            1.4,
            Size::new(360.0, 120.0),
        );
        assert_eq!(spaced_pages.len(), 2);
        assert_eq!(spaced_pages[0][0].block_after, 0.0);

        let mut fragmented_table = one_line_table(6);
        let ContentNode::Table { style, .. } = &mut fragmented_table else {
            unreachable!();
        };
        style.block_after_em = Some(4.0);
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&fragmented_table),
            None,
            16.0,
            1.4,
            Size::new(360.0, 120.0),
        );
        let fragments = pages
            .iter()
            .flatten()
            .filter(|page_node| matches!(page_node.node, ContentNode::Table { .. }))
            .collect::<Vec<_>>();

        assert!(fragments.len() > 1);
        assert!(
            fragments[..fragments.len() - 1]
                .iter()
                .all(|fragment| fragment.block_after == 0.0)
        );
        assert_eq!(fragments.last().unwrap().block_after, 64.0);
    }

    #[test]
    fn narrow_table_layout_overflows_without_unbounded_column_amplification() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/table.epub").to_vec(),
        )
        .expect("table fixture should be a valid EPUB");
        let ContentNode::Table {
            row_groups, style, ..
        } = epub
            .presentation()
            .chapter(0)
            .unwrap()
            .nodes()
            .iter()
            .find(|node| matches!(node, ContentNode::Table { .. }))
            .expect("fixture must retain a semantic table")
        else {
            unreachable!();
        };

        assert_eq!(epub_table_layout_width(row_groups, style, 240.0), 360.0);
        assert_eq!(epub_table_layout_width(row_groups, style, 600.0), 600.0);

        let mut amplified = row_groups.clone();
        amplified[0].rows[0].cells[0].column_span = 1_000;
        assert_eq!(epub_table_layout_width(&amplified, style, 240.0), 4_096.0);
    }

    #[test]
    fn authored_table_width_is_shared_by_measurement_and_paint() {
        let ContentNode::Table {
            row_groups,
            mut style,
            ..
        } = one_line_table(2)
        else {
            unreachable!();
        };
        style.width = Some(shosai_core::epub::render::NodeWidth::Percent(0.5));

        assert_eq!(epub_table_layout_width(&row_groups, &style, 1_000.0), 500.0);
        style.width = Some(shosai_core::epub::render::NodeWidth::Pixels(720.0));
        assert_eq!(epub_table_layout_width(&row_groups, &style, 1_000.0), 720.0);
        style.width = Some(shosai_core::epub::render::NodeWidth::Pixels(200.0));
        assert_eq!(epub_table_layout_width(&row_groups, &style, 1_000.0), 200.0);
        style.width = Some(shosai_core::epub::render::NodeWidth::Percent(0.5));
        assert_eq!(epub_table_layout_width(&row_groups, &style, 600.0), 300.0);
        style.width = Some(shosai_core::epub::render::NodeWidth::Percent(1.0));
        style.max_width = Some(shosai_core::epub::render::NodeWidth::Pixels(320.0));
        assert_eq!(epub_table_layout_width(&row_groups, &style, 1_000.0), 320.0);
        style.max_width = Some(shosai_core::epub::render::NodeWidth::Percent(0.25));
        assert_eq!(epub_table_layout_width(&row_groups, &style, 1_000.0), 250.0);
    }

    #[test]
    fn ordered_figure_width_and_margin_share_one_content_box() {
        let mut style = shosai_core::epub::render::NodeStyle {
            width: Some(shosai_core::epub::render::NodeWidth::Percent(0.5)),
            max_width: Some(shosai_core::epub::render::NodeWidth::Pixels(320.0)),
            margin_left_em: Some(1.0),
            ..Default::default()
        };

        assert_eq!(epub_figure_content_width(&style, 1_000.0, 16.0), 320.0);
        assert_eq!(
            epub_table_content_width(&style, 320.0, 1_000.0, 16.0),
            320.0
        );
        let table_margin = epub_table_margin_left(&style, 16.0, 1_000.0, 320.0, 320.0);
        assert_eq!(table_margin, 16.0);
        assert_eq!(320.0 + table_margin, 336.0);

        style.width = None;
        style.max_width = None;
        style.margin_left_em = Some(2.0);
        let content_width = epub_table_content_width(&style, 360.0, 360.0, 16.0);
        let margin = epub_table_margin_left(&style, 16.0, 360.0, 360.0, content_width);
        assert_eq!((content_width, margin), (328.0, 32.0));
        assert_eq!(content_width + margin, 360.0);

        style.margin_left_em = Some(100.0);
        let content_width = epub_table_content_width(&style, 360.0, 360.0, 16.0);
        let margin = epub_table_margin_left(&style, 16.0, 360.0, 360.0, content_width);
        assert_eq!((content_width, margin), (1.0, 359.0));
        assert_eq!(content_width + margin, 360.0);
    }

    #[test]
    fn embedded_font_table_cells_use_native_row_geometry() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/fonts.epub").to_vec(),
        )
        .unwrap();
        let mut table = one_line_table(1);
        let ContentNode::Table { row_groups, .. } = &mut table else {
            unreachable!();
        };
        let ContentNode::Paragraph(spans, _) = &mut row_groups[0].rows[0].cells[0].children[0]
        else {
            unreachable!();
        };
        spans[0].text = "Embedded font text that wraps across several lines".into();
        spans[0].font_family = Some("FixtureTtf".into());
        let measured = measure_epub_spans(
            Some(epub.fonts()),
            spans,
            16.0,
            120.0,
            Default::default(),
            None,
        )
        .expect("fixture text should use native shaping");
        let geometry =
            epub_table_geometry_bounded(row_groups, &[120.0], 20, 16.0, 600.0, Some(epub.fonts()));
        let spacing = epub_node_list_spacing(
            &row_groups[0].rows[0].cells[0].children,
            16.0,
            EPUB_TABLE_CELL_SPACING,
        );

        assert_eq!(
            geometry.row_heights[0],
            measured.height + spacing + 2.0 * EPUB_TABLE_CELL_PADDING
        );
    }

    #[test]
    fn narrow_authored_table_wraps_caption_at_its_painted_width() {
        let mut table = one_line_table(1);
        let ContentNode::Table { caption, style, .. } = &mut table else {
            unreachable!();
        };
        *caption = table_test_cell(
            "A long authored caption that must wrap inside a narrow table instead of the page",
            None,
        )
        .children
        .into_iter()
        .next()
        .and_then(|node| match node {
            ContentNode::Paragraph(spans, _) => Some(spans),
            _ => None,
        })
        .unwrap();
        style.width = Some(shosai_core::epub::render::NodeWidth::Pixels(200.0));

        let narrow = estimated_epub_compact_node_height_bounded(
            &table,
            100,
            20,
            16.0,
            800.0,
            600.0,
            Some(600.0),
        );
        let ContentNode::Table { style, .. } = &mut table else {
            unreachable!();
        };
        style.width = None;
        let page_width = estimated_epub_compact_node_height_bounded(
            &table,
            100,
            20,
            16.0,
            800.0,
            600.0,
            Some(600.0),
        );

        assert!(narrow >= page_width + 2.0 * 16.0 * TEXT_LINE_HEIGHT);
    }

    #[test]
    fn table_caption_height_counts_forced_newlines() {
        let caption = vec![shosai_core::epub::render::TextSpan {
            text: "first line\n\nthird line".into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: true,
            link: None,
        }];

        assert_eq!(
            epub_table_caption_height(None, &caption, None, 16.0, 500.0, 600.0),
            3.0 * 16.0 * TEXT_LINE_HEIGHT
        );
    }

    #[test]
    fn table_caption_height_uses_embedded_font_measurement() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/fonts.epub").to_vec(),
        )
        .unwrap();
        let caption = vec![shosai_core::epub::render::TextSpan {
            text: "A caption shaped with the embedded fixture font".into(),
            math: None,
            font_family: Some("FixtureTtf".into()),
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let measured = measure_epub_spans(
            Some(epub.fonts()),
            &caption,
            16.0,
            120.0,
            Default::default(),
            None,
        )
        .expect("fixture caption should use its embedded font");

        assert_eq!(
            epub_table_caption_height(Some(epub.fonts()), &caption, None, 16.0, 120.0, 600.0,),
            measured.height
        );
    }

    #[test]
    fn sparse_code_table_prefix_does_not_receive_half_the_table_width() {
        use shosai_core::epub::render::{TableRow, TableRowGroup, TableRowGroupKind};

        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![TableRow {
                cells: vec![
                    table_test_cell("\u{200b} ", None),
                    table_test_cell("> =some_long_code_expression", None),
                ],
            }],
        }];

        let widths = epub_table_column_widths(&row_groups, 360.0);

        assert_eq!(widths.len(), 2);
        assert!(widths[0] < 50.0, "sparse prefix was {widths:?}");
        assert!(widths[1] > 300.0, "code column was {widths:?}");
        assert!((widths.iter().sum::<f32>() + BLOCKQUOTE_SPACING - 360.0).abs() < 0.001);
    }

    #[test]
    fn authored_table_percentages_prevent_header_and_spacer_columns_from_expanding() {
        use shosai_core::epub::render::{NodeWidth, TableRow, TableRowGroup, TableRowGroupKind};

        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![TableRow {
                cells: vec![
                    table_test_cell("Autoregressive Model", Some(NodeWidth::Percent(0.156))),
                    table_test_cell("", Some(NodeWidth::Percent(0.1436))),
                    table_test_cell(
                        "The task of predicting the next word in a sequence",
                        Some(NodeWidth::Percent(0.6846)),
                    ),
                ],
            }],
        }];

        let widths = epub_table_column_widths(&row_groups, 600.0);

        assert!(
            widths[2] > widths[0] * 4.0,
            "authored widths were {widths:?}"
        );
        assert!(widths[1] < widths[2] / 4.0, "spacer expanded to {widths:?}");
        assert!((widths.iter().sum::<f32>() + 2.0 * BLOCKQUOTE_SPACING - 600.0).abs() < 0.001);
    }

    #[test]
    fn a_single_authored_column_keeps_its_constraint_and_leaves_only_the_remainder() {
        use shosai_core::epub::render::{NodeWidth, TableRow, TableRowGroup, TableRowGroupKind};

        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![TableRow {
                cells: vec![
                    table_test_cell("first", Some(NodeWidth::Percent(0.8))),
                    table_test_cell("second", None),
                ],
            }],
        }];
        let widths = epub_table_column_widths(&row_groups, 500.0);

        assert!(
            (widths[0] - 0.8 * (500.0 - BLOCKQUOTE_SPACING)).abs() < 0.001,
            "{widths:?}"
        );
        assert!(widths[1] < widths[0] / 3.0, "{widths:?}");
        assert!((widths.iter().sum::<f32>() + BLOCKQUOTE_SPACING - 500.0).abs() < 0.001);
    }

    #[test]
    fn logical_grid_skips_columns_occupied_by_rowspans() {
        use shosai_core::epub::render::{TableRow, TableRowGroup, TableRowGroupKind};

        let mut spanning = table_test_cell("span", None);
        spanning.row_span = 2;
        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![
                TableRow {
                    cells: vec![spanning, table_test_cell("right", None)],
                },
                TableRow {
                    cells: vec![table_test_cell("next", None)],
                },
            ],
        }];
        let placements = epub_table_cell_placements(&row_groups);
        assert_eq!(placements[0][0].column, 0);
        assert_eq!(placements[0][1].column, 1);
        assert_eq!(placements[1][0].column, 1);
    }

    #[test]
    fn table_width_and_geometry_chain_builds_placements_once() {
        use shosai_core::epub::render::{TableRow, TableRowGroup, TableRowGroupKind};

        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![TableRow {
                cells: vec![table_test_cell("cell", None)],
            }],
        }];
        TABLE_PLACEMENT_PASSES.with(|passes| passes.set(0));

        let placements = epub_table_cell_placements(&row_groups);
        let widths = epub_table_column_widths_from_placements(&row_groups, 360.0, &placements);
        let _geometry = epub_table_geometry_bounded_from_placements(
            &row_groups,
            &placements,
            &widths,
            20,
            16.0,
            600.0,
            None,
        );

        TABLE_PLACEMENT_PASSES.with(|passes| assert_eq!(passes.get(), 1));
    }

    #[test]
    fn colspan_and_rowspan_share_placements_and_widths_between_measurement_and_painting() {
        use shosai_core::epub::render::{TableRow, TableRowGroup, TableRowGroupKind};

        let mut spanning = table_test_cell("span", None);
        spanning.row_span = 2;
        spanning.column_span = 2;
        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![
                TableRow {
                    cells: vec![spanning, table_test_cell("third", None)],
                },
                TableRow {
                    cells: vec![table_test_cell("placed third", None)],
                },
            ],
        }];
        let placements = epub_table_cell_placements(&row_groups);
        let widths = epub_table_column_widths(&row_groups, 600.0);

        assert_eq!(
            placements[1][0],
            EpubTableCellPlacement { column: 2, span: 1 }
        );
        let measured = epub_table_cell_content_width(placements[1][0], &widths);
        let painted =
            epub_table_cell_width(placements[1][0], &widths) - 2.0 * EPUB_TABLE_CELL_PADDING;
        assert!((measured - painted).abs() < 0.001);
        assert!((widths.iter().sum::<f32>() + 2.0 * BLOCKQUOTE_SPACING - 600.0).abs() < 0.001);
    }

    #[test]
    fn malicious_aggregate_colspans_are_clamped_to_the_bounded_grid() {
        use shosai_core::epub::render::{TableRow, TableRowGroup, TableRowGroupKind};
        let mut cell = table_test_cell("wide", None);
        cell.column_span = u16::MAX;
        let row_groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![TableRow {
                cells: vec![cell; 300],
            }],
        }];
        let placements = epub_table_cell_placements(&row_groups);
        assert_eq!(epub_table_column_count(&row_groups), MAX_EPUB_TABLE_COLUMNS);
        assert!(
            placements[0]
                .iter()
                .all(|cell| cell.column + cell.span <= MAX_EPUB_TABLE_COLUMNS)
        );
        assert_eq!(
            placements[0].last().unwrap(),
            &EpubTableCellPlacement {
                column: 255,
                span: 1
            }
        );
    }

    #[test]
    fn tall_rowspan_geometry_combines_rows_and_keeps_later_cells_beside_it() {
        use shosai_core::epub::render::{TableRow, TableRowGroup, TableRowGroupKind};
        let mut spanning = table_test_cell("span", None);
        spanning.row_span = 2;
        let groups = vec![TableRowGroup {
            kind: TableRowGroupKind::Body,
            rows: vec![
                TableRow {
                    cells: vec![spanning, table_test_cell("right", None)],
                },
                TableRow {
                    cells: vec![table_test_cell("below right", None)],
                },
            ],
        }];
        let widths = epub_table_column_widths(&groups, 360.0);
        let geometry = epub_table_geometry(&groups, &widths, |cell, _| {
            if cell.row_span == 2 { 100.0 } else { 20.0 }
        });
        assert!((geometry.cells[0][0].height - geometry.height).abs() < 0.001);
        assert_eq!(geometry.cells[1][0].x, geometry.cells[0][1].x);
        assert!(geometry.cells[1][0].y > geometry.cells[0][1].y);
    }

    #[test]
    fn epub_paginator_splits_long_paragraphs_without_losing_formatting() {
        let text = "This is a linked sentence that should wrap cleanly. ".repeat(30);
        let spans = vec![shosai_core::epub::render::TextSpan {
            text: text.clone(),
            math: None,
            font_family: None,
            bold: true,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: Some("chapter-2.xhtml".to_string()),
        }];
        let pages = paginate_epub_chapter(
            &[ContentNode::Paragraph(
                spans,
                shosai_core::epub::render::NodeStyle {
                    block_before_em: Some(2.0),
                    block_after_em: Some(3.0),
                    ..Default::default()
                },
            )],
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
        );

        assert!(pages.len() > 1);
        let chunks = pages
            .iter()
            .flatten()
            .map(|page_node| match &page_node.node {
                ContentNode::Paragraph(spans, _) => &spans[0],
                node => panic!("expected paragraph, got {node:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chunks
                .iter()
                .flat_map(|span| span.text.chars())
                .collect::<String>(),
            text
        );
        assert!(chunks.iter().all(|span| span.bold));
        assert!(
            chunks
                .iter()
                .all(|span| span.link.as_deref() == Some("chapter-2.xhtml"))
        );
        let styles = pages
            .iter()
            .flatten()
            .map(|page_node| {
                page_node
                    .node
                    .style()
                    .expect("paragraph fragment has style")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            styles.first().and_then(|style| style.block_before_em),
            Some(2.0)
        );
        assert_eq!(
            styles.first().and_then(|style| style.block_after_em),
            Some(0.0)
        );
        assert!(styles[1..styles.len() - 1].iter().all(|style| {
            style.block_before_em == Some(0.0) && style.block_after_em == Some(0.0)
        }));
        assert_eq!(
            styles.last().and_then(|style| style.block_before_em),
            Some(0.0)
        );
        assert_eq!(
            styles.last().and_then(|style| style.block_after_em),
            Some(3.0)
        );
    }

    #[test]
    fn epub_paginator_bounds_pathological_text_geometry_without_losing_content() {
        let text = "é".repeat(MAX_EPUB_PAGES + 20);
        let spans = vec![shosai_core::epub::render::TextSpan {
            text: text.clone(),
            math: None,
            font_family: None,
            bold: true,
            italic: true,
            monospace: true,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let style = shosai_core::epub::render::NodeStyle {
            font_size_multiplier: Some(1_000_000_000.0),
            ..Default::default()
        };

        let pages = paginate_epub_chapter(
            &[ContentNode::Paragraph(spans, style)],
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
        );

        assert_eq!(pages.len(), MAX_EPUB_PAGES);
        let chunks = pages
            .iter()
            .flatten()
            .map(|page_node| match &page_node.node {
                ContentNode::Paragraph(spans, _) => &spans[0],
                node => panic!("expected paragraph, got {node:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chunks
                .iter()
                .flat_map(|span| span.text.chars())
                .collect::<String>(),
            text
        );
        assert!(chunks.iter().all(|span| {
            span.bold && span.italic && span.monospace && span.font_size_multiplier == 1.0
        }));
        assert_eq!(chunks.last().unwrap().text.chars().count(), 20);
        assert_eq!(pages.last().unwrap().len(), 2);
        assert_eq!(pages.last().unwrap()[1].text_offset, MAX_EPUB_PAGES);
    }

    #[test]
    fn epub_pagination_budget_is_shared_across_blockquotes_and_chapters() {
        let paragraph = |character: char| {
            ContentNode::Paragraph(
                vec![shosai_core::epub::render::TextSpan {
                    text: character.to_string().repeat(20),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: None,
                }],
                shosai_core::epub::render::NodeStyle {
                    font_size_multiplier: Some(1_000_000_000.0),
                    ..Default::default()
                },
            )
        };
        let first_chapter = vec![
            ContentNode::BlockQuote {
                children: vec![paragraph('a')],
                style: Default::default(),
            },
            ContentNode::BlockQuote {
                children: vec![paragraph('b')],
                style: Default::default(),
            },
        ];
        let second_chapter = vec![paragraph('c')];
        let mut budget = EpubPaginationBudget {
            remaining_page_breaks: 3,
            ..Default::default()
        };

        let first_pages = paginate_epub_chapter_with_budget(
            &first_chapter,
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
            None,
            &mut budget,
        );
        let second_pages = paginate_epub_chapter_with_budget(
            &second_chapter,
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
            None,
            &mut budget,
        );

        assert_eq!(
            first_pages.len() + second_pages.len(),
            5,
            "the three available breaks must not be charged again when recursive pages are integrated"
        );
        assert_eq!(budget.remaining_page_breaks, 0);
        assert!(
            first_pages.iter().flatten().count() + second_pages.iter().flatten().count() <= 7,
            "exhausted recursive pagination must retain source nodes without fresh fragmentation"
        );
        let text = first_pages
            .iter()
            .chain(&second_pages)
            .flat_map(|page| page.iter().map(|node| node.node.clone()))
            .collect::<Vec<_>>();
        let text = shosai_core::search::extract_text_from_nodes(&text)
            .chars()
            .filter(|character| matches!(character, 'a' | 'b' | 'c'))
            .collect::<String>();
        assert_eq!(
            text,
            format!("{}{}{}", "a".repeat(20), "b".repeat(20), "c".repeat(20))
        );
    }

    #[test]
    fn cancelled_pagination_stops_before_processing_nodes() {
        let cancellation = EpubPaginationCancellation::default();
        cancellation.cancel();
        let mut budget = EpubPaginationBudget::default().with_cancellation(cancellation);

        let pages = paginate_epub_chapter_with_budget(
            &[ContentNode::Paragraph(Vec::new(), Default::default())],
            None,
            16.0,
            1.6,
            Size::new(360.0, 600.0),
            None,
            &mut budget,
        );

        assert!(pages.is_empty());
    }

    #[test]
    fn table_cancellation_interrupts_placement_and_geometry_without_publishing_pages() {
        let table = one_line_table(256);
        let total_cells = 256;

        for (cancel_after, completed_passes) in [(8, 0), (total_cells * 2 + 8, 2)] {
            TABLE_CELL_VISITS.with(|visits| visits.set(0));
            TABLE_CANCEL_AFTER_VISITS.with(|limit| limit.set(Some(cancel_after)));
            let cancellation = EpubPaginationCancellation::default();
            let mut budget =
                EpubPaginationBudget::default().with_cancellation(cancellation.clone());

            let pages = paginate_epub_chapter_with_budget(
                std::slice::from_ref(&table),
                None,
                16.0,
                1.6,
                Size::new(360.0, 600.0),
                None,
                &mut budget,
            );
            let visits = TABLE_CELL_VISITS.with(std::cell::Cell::get);
            TABLE_CANCEL_AFTER_VISITS.with(|limit| limit.set(None));

            assert!(cancellation.is_cancelled());
            assert!(
                pages.is_empty(),
                "cancelled table pages must not be published"
            );
            assert!(
                visits < (completed_passes + 1) * total_cells,
                "cancelled pass traversed every cell: {visits} visits"
            );
        }
    }

    #[test]
    fn table_cancellation_interrupts_one_large_cell_at_an_internal_checkpoint() {
        let mut table = one_line_table(1);
        let ContentNode::Table { row_groups, .. } = &mut table else {
            unreachable!();
        };
        let child = row_groups[0].rows[0].cells[0].children[0].clone();
        let total_children = EPUB_PAGINATION_LOOP_CHUNK * 2 + 1;
        row_groups[0].rows[0].cells[0].children = vec![child; total_children];

        TABLE_CELL_INTERNAL_VISITS.with(|visits| visits.set(0));
        // Each retained paragraph visits its child and span. Cancel on child 64,
        // where the bounded child traversal is required to poll.
        TABLE_CANCEL_AFTER_INTERNAL_VISITS
            .with(|limit| limit.set(Some(EPUB_PAGINATION_LOOP_CHUNK * 2 + 1)));
        let cancellation = EpubPaginationCancellation::default();
        let mut budget = EpubPaginationBudget::default().with_cancellation(cancellation.clone());

        let pages = paginate_epub_chapter_with_budget(
            &[table],
            None,
            16.0,
            1.6,
            Size::new(360.0, 600.0),
            None,
            &mut budget,
        );
        let visits = TABLE_CELL_INTERNAL_VISITS.with(std::cell::Cell::get);
        TABLE_CANCEL_AFTER_INTERNAL_VISITS.with(|limit| limit.set(None));

        assert!(cancellation.is_cancelled());
        assert!(
            pages.is_empty(),
            "cancelled table pages must not be published"
        );
        assert_eq!(visits, EPUB_PAGINATION_LOOP_CHUNK * 2 + 1);
        assert!(visits < total_children * 2, "traversal did not stop early");
    }

    #[test]
    fn table_cancellation_interrupts_each_oversized_nested_child() {
        let oversized_paragraph = || {
            let mut table = one_line_table(1);
            let ContentNode::Table { row_groups, .. } = &mut table else {
                unreachable!();
            };
            let ContentNode::Paragraph(spans, _) = &mut row_groups[0].rows[0].cells[0].children[0]
            else {
                unreachable!();
            };
            spans[0].text = "x".repeat(EPUB_PAGINATION_LOOP_CHUNK * 4);
            row_groups[0].rows[0].cells[0].children.remove(0)
        };
        let paragraph = oversized_paragraph();
        let mut nested_table = one_line_table(1);
        let ContentNode::Table { row_groups, .. } = &mut nested_table else {
            unreachable!();
        };
        row_groups[0].rows[0].cells[0].children = vec![oversized_paragraph()];
        let children = [
            paragraph.clone(),
            ContentNode::Figure {
                children: vec![paragraph.clone()],
                style: Default::default(),
            },
            ContentNode::BlockQuote {
                children: vec![paragraph],
                style: Default::default(),
            },
            nested_table,
        ];

        for child in children {
            let mut table = one_line_table(1);
            let ContentNode::Table { row_groups, .. } = &mut table else {
                unreachable!();
            };
            row_groups[0].rows[0].cells[0].children = vec![child];
            TABLE_CELL_INTERNAL_VISITS.with(|visits| visits.set(0));
            TABLE_CANCEL_AFTER_INTERNAL_VISITS.with(|limit| limit.set(Some(16)));
            let cancellation = EpubPaginationCancellation::default();
            let mut budget =
                EpubPaginationBudget::default().with_cancellation(cancellation.clone());

            let pages = paginate_epub_chapter_with_budget(
                &[table],
                None,
                16.0,
                1.6,
                Size::new(360.0, 600.0),
                None,
                &mut budget,
            );
            let visits = TABLE_CELL_INTERNAL_VISITS.with(std::cell::Cell::get);
            TABLE_CANCEL_AFTER_INTERNAL_VISITS.with(|limit| limit.set(None));

            assert!(cancellation.is_cancelled());
            assert!(pages.is_empty(), "cancelled table page was published");
            assert!(
                visits < EPUB_PAGINATION_LOOP_CHUNK * 4,
                "oversized child traversal did not stop early: {visits}"
            );
        }
    }

    #[test]
    fn measured_paragraph_observes_cancellation_between_shaping_chunks() {
        let cancellation = EpubPaginationCancellation::default();
        let signal = cancellation.clone();
        let spans = vec![shosai_core::epub::render::TextSpan {
            text: "x".repeat(EPUB_PAGINATION_SHAPE_CHUNK * 2),
            math: None,
            font_family: Some("Book".into()),
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let measure = move |_: &[shosai_core::epub::render::TextSpan]| {
            signal.cancel();
            Some(EpubTextLayout {
                width: 1.0,
                height: 1.0,
                lines: Vec::new(),
                links: Vec::new(),
                endpoints: Vec::new(),
            })
        };
        let mut pages = vec![Vec::new()];
        let mut remaining = 100.0;
        let mut budget = EpubPaginationBudget::default().with_cancellation(cancellation);

        assert!(!paginate_measured_paragraph(
            &spans,
            &Default::default(),
            &measure,
            0,
            0.0,
            100.0,
            false,
            &mut pages,
            &mut remaining,
            &mut budget,
        ));
        assert!(pages.iter().all(Vec::is_empty));
    }

    #[test]
    fn epub_paginator_splits_long_lists_without_losing_items() {
        let items = (0..30)
            .map(|index| {
                vec![shosai_core::epub::render::TextSpan {
                    text: format!("List item {index}"),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: None,
                }]
            })
            .collect::<Vec<_>>();
        let pages = paginate_epub_chapter(
            &[ContentNode::OrderedList {
                items: items.clone(),
                start: 1,
            }],
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
        );

        assert!(pages.len() > 1, "a long list must span multiple pages");
        let paginated_items = pages
            .iter()
            .flatten()
            .flat_map(|page_node| match &page_node.node {
                ContentNode::OrderedList { items, .. } => items.clone(),
                node => panic!("expected ordered list, got {node:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(paginated_items, items);
        let starts = pages
            .iter()
            .flatten()
            .filter_map(|page_node| match &page_node.node {
                ContentNode::OrderedList { start, .. } => Some(*start),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(starts.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn maximal_ordered_list_start_saturates_across_fragments() {
        let item = vec![shosai_core::epub::render::TextSpan {
            text: "item".into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let pages = paginate_epub_chapter(
            &[ContentNode::OrderedList {
                items: vec![item; 10],
                start: usize::MAX,
            }],
            None,
            16.0,
            1.6,
            Size::new(240.0, 80.0),
        );

        assert!(pages.len() > 1);
        assert!(pages.iter().flatten().all(|page_node| matches!(
            page_node.node,
            ContentNode::OrderedList {
                start: usize::MAX,
                ..
            }
        )));
    }

    #[test]
    fn epub_paginator_keeps_sparse_blockquotes_on_the_same_page() {
        let paragraph = |text: &str| {
            ContentNode::Paragraph(
                vec![shosai_core::epub::render::TextSpan {
                    text: text.to_string(),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: Some(format!("{}.xhtml", text.replace(' ', "-"))),
                }],
                Default::default(),
            )
        };
        let nodes = vec![
            paragraph("Chapter 1"),
            ContentNode::BlockQuote {
                children: vec![paragraph("Section 1.1")],
                style: Default::default(),
            },
            ContentNode::BlockQuote {
                children: vec![paragraph("Section 1.2")],
                style: Default::default(),
            },
        ];

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));

        assert_eq!(pages.len(), 1, "short TOC groups should share a page");
        assert_eq!(pages[0].len(), nodes.len());
    }

    #[test]
    fn epub_paginator_accounts_for_text_height_separately_from_block_spacing() {
        let nodes = (1..=4)
            .map(|index| {
                ContentNode::Paragraph(
                    vec![shosai_core::epub::render::TextSpan {
                        text: format!("Short paragraph {index}"),
                        math: None,
                        font_family: None,
                        bold: false,
                        italic: false,
                        monospace: false,
                        font_size_multiplier: 1.0,
                        preserve_whitespace: false,
                        link: None,
                    }],
                    Default::default(),
                )
            })
            .collect::<Vec<_>>();

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].len(), nodes.len());
    }

    #[test]
    fn epub_paginator_packs_sparse_blockquotes_until_the_page_is_full() {
        let nodes = (1..=5)
            .map(|index| ContentNode::BlockQuote {
                children: vec![ContentNode::Paragraph(
                    vec![shosai_core::epub::render::TextSpan {
                        text: format!("Section 1.{index}"),
                        math: None,
                        font_family: None,
                        bold: false,
                        italic: false,
                        monospace: false,
                        font_size_multiplier: 1.0,
                        preserve_whitespace: false,
                        link: Some(format!("section-{index}.xhtml")),
                    }],
                    Default::default(),
                )],
                style: Default::default(),
            })
            .collect::<Vec<_>>();

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));

        assert_eq!(pages.len(), 2, "TOC groups should flow by page capacity");
        assert_eq!(pages[0].len(), 3);
        assert_eq!(pages[1].len(), 2);
        assert_eq!(pages.iter().map(Vec::len).sum::<usize>(), nodes.len());
    }

    #[test]
    fn epub_paginator_keeps_nested_toc_chapter_together_when_it_fits() {
        let paragraph = |text: String| {
            ContentNode::Paragraph(
                vec![shosai_core::epub::render::TextSpan {
                    link: Some(format!("{}.xhtml", text.replace(' ', "-"))),
                    text,
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                }],
                Default::default(),
            )
        };
        let subsection = |section: usize, count: usize| ContentNode::BlockQuote {
            children: (1..=count)
                .map(|index| paragraph(format!("10.{section}.{index}. Subsection")))
                .collect(),
            style: Default::default(),
        };
        let chapter = ContentNode::BlockQuote {
            children: vec![
                paragraph("10.1. Applications from a system viewpoint".to_string()),
                subsection(1, 3),
                paragraph("10.2. Making a release".to_string()),
                subsection(2, 6),
                paragraph("10.3. Release packaging".to_string()),
                subsection(3, 3),
                paragraph("10.4. Installing a release".to_string()),
                paragraph("10.5. Summary".to_string()),
            ],
            style: Default::default(),
        };
        let chapter_text_len = content_node_text_len(&chapter);

        let pages = paginate_epub_chapter(&[chapter], None, 16.0, 1.6, Size::new(785.0, 865.0));

        assert_eq!(pages.len(), 1, "nested TOC entries fit on one page");
        assert_eq!(content_node_text_len(&pages[0][0].node), chapter_text_len);
    }

    #[test]
    fn epub_paginator_keeps_linked_toc_heading_with_its_first_entry() {
        let paragraph = |text: &str, link: Option<&str>| {
            ContentNode::Paragraph(
                vec![shosai_core::epub::render::TextSpan {
                    text: text.to_string(),
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: link.map(str::to_string),
                }],
                Default::default(),
            )
        };
        let entries = (1..=4)
            .map(|index| paragraph(&format!("13.{index}. Entry"), Some("chapter-13.xhtml")))
            .collect();
        let nodes = vec![
            ContentNode::Heading {
                level: 1,
                spans: vec![shosai_core::epub::render::TextSpan {
                    text: "Previous chapter".to_string(),
                    math: None,
                    font_family: None,
                    bold: true,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: None,
                }],
                style: Default::default(),
            },
            paragraph("Previous summary", None),
            paragraph("Chapter 13", Some("chapter-13.xhtml")),
            ContentNode::BlockQuote {
                children: vec![ContentNode::BlockQuote {
                    children: entries,
                    style: Default::default(),
                }],
                style: Default::default(),
            },
        ];

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));

        let second_page_nodes = pages[1]
            .iter()
            .map(|page_node| page_node.node.clone())
            .collect::<Vec<_>>();
        let second_page_text = shosai_core::search::extract_text_from_nodes(&second_page_nodes);
        assert!(second_page_text.contains("Chapter 13"));
        assert!(second_page_text.contains("13.1. Entry"));
        let all_page_nodes = pages
            .iter()
            .flatten()
            .map(|page_node| page_node.node.clone())
            .collect::<Vec<_>>();
        let all_text = shosai_core::search::extract_text_from_nodes(&all_page_nodes);
        for index in 1..=4 {
            assert_eq!(all_text.matches(&format!("13.{index}. Entry")).count(), 1);
        }
    }

    #[test]
    fn epub_paginator_splits_long_blockquotes_without_losing_children() {
        let children = (0..20)
            .map(|index| {
                ContentNode::Paragraph(
                    vec![shosai_core::epub::render::TextSpan {
                        text: format!("Quoted paragraph {index}"),
                        math: None,
                        font_family: None,
                        bold: false,
                        italic: false,
                        monospace: false,
                        font_size_multiplier: 1.0,
                        preserve_whitespace: false,
                        link: None,
                    }],
                    Default::default(),
                )
            })
            .collect::<Vec<_>>();
        let pages = paginate_epub_chapter(
            &[ContentNode::BlockQuote {
                children: children.clone(),
                style: Default::default(),
            }],
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
        );

        assert!(
            pages.len() > 1,
            "a long blockquote must span multiple pages"
        );
        let paginated_children = pages
            .iter()
            .flatten()
            .flat_map(|page_node| match &page_node.node {
                ContentNode::BlockQuote { children, .. } => children.clone(),
                node => panic!("expected blockquote, got {node:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shosai_core::search::extract_text_from_nodes(&paginated_children),
            shosai_core::search::extract_text_from_nodes(&children)
        );
    }

    #[test]
    fn blockquote_fragments_suppress_authored_margins_at_page_boundaries() {
        let child = |text: &str| {
            let mut node = table_test_cell(text, None).children.remove(0);
            let style = node.style_mut().unwrap();
            style.block_before_em = Some(2.0);
            style.block_after_em = Some(3.0);
            node
        };
        let children = vec![child("first"), child("second")];
        let first_height = estimated_epub_compact_node_height(&children[0], 40, 20, 16.0);
        let (prefix, remaining, reserved, _) = split_epub_blockquote_prefix(
            &children,
            32.0 + first_height,
            20,
            16.0,
            400.0,
            None,
            &EpubPaginationBudget::default(),
        )
        .unwrap();

        assert_eq!(prefix.len(), 1);
        assert_eq!(remaining.len(), 1);
        assert_eq!(prefix[0].style().unwrap().block_after_em, Some(0.0));
        assert_eq!(remaining[0].style().unwrap().block_before_em, Some(0.0));
        assert_eq!(reserved, 32.0 + first_height);
        assert_eq!(
            epub_node_boundary_spacing(&prefix, prefix.len(), 16.0, BLOCKQUOTE_SPACING),
            0.0
        );
        assert_eq!(
            epub_node_boundary_spacing(&remaining, 0, 16.0, BLOCKQUOTE_SPACING),
            0.0
        );

        let styleless = vec![ContentNode::CodeBlock {
            code: "code".into(),
            language: None,
        }];
        let prefix_style = shosai_core::epub::render::NodeStyle {
            fragment_after: true,
            ..Default::default()
        };
        let remaining_style = shosai_core::epub::render::NodeStyle {
            fragment_before: true,
            ..Default::default()
        };
        assert_eq!(
            epub_fragment_boundary_spacing(
                &styleless,
                styleless.len(),
                16.0,
                BLOCKQUOTE_SPACING,
                &prefix_style,
            ),
            0.0
        );
        assert_eq!(
            epub_fragment_boundary_spacing(
                &styleless,
                0,
                16.0,
                BLOCKQUOTE_SPACING,
                &remaining_style,
            ),
            0.0
        );
    }

    #[test]
    fn epub_paginator_preserves_offsets_across_nested_blockquote_splits() {
        fn paragraph(text: String) -> ContentNode {
            ContentNode::Paragraph(
                vec![shosai_core::epub::render::TextSpan {
                    text,
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: None,
                }],
                Default::default(),
            )
        }

        fn first_text(node: &ContentNode) -> &str {
            match node {
                ContentNode::Heading { spans, .. } => &spans[0].text,
                ContentNode::Paragraph(spans, _) => &spans[0].text,
                ContentNode::BlockQuote { children, .. } => first_text(&children[0]),
                node => panic!("expected text content, got {node:?}"),
            }
        }

        let nested = ContentNode::BlockQuote {
            children: (0..12)
                .map(|index| paragraph(format!("Unique nested entry {index:02}")))
                .collect(),
            style: Default::default(),
        };
        let nodes = vec![
            paragraph("Introductory text before the nested quote".to_string()),
            ContentNode::BlockQuote {
                children: vec![nested],
                style: Default::default(),
            },
        ];
        let chapter_text = shosai_core::search::extract_text_from_nodes(&nodes);

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));

        assert!(pages.len() > 1, "the nested quote must be split");
        for page_node in pages.iter().flatten() {
            let text = first_text(&page_node.node);
            let expected = chapter_text
                .find(text)
                .expect("paginated text must exist in the source chapter");
            assert_eq!(
                page_node.text_offset, expected,
                "the page containing {text:?} must retain its source offset"
            );
        }
    }

    #[test]
    fn epub_paginator_keeps_a_linked_label_with_a_splittable_first_entry() {
        let paragraph = |text: String, link: Option<&str>| {
            ContentNode::Paragraph(
                vec![shosai_core::epub::render::TextSpan {
                    text,
                    math: None,
                    font_family: None,
                    bold: false,
                    italic: false,
                    monospace: false,
                    font_size_multiplier: 1.0,
                    preserve_whitespace: false,
                    link: link.map(str::to_string),
                }],
                Default::default(),
            )
        };
        let label = "Chapter 13";
        let first_entry = "Long first entry ".repeat(30);
        let nodes = vec![
            paragraph("Previous page content ".repeat(8), None),
            paragraph(label.to_string(), Some("chapter-13.xhtml")),
            ContentNode::BlockQuote {
                children: vec![paragraph(
                    first_entry.clone(),
                    Some("chapter-13.xhtml#first"),
                )],
                style: Default::default(),
            },
        ];

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 180.0));
        let page_text = pages
            .iter()
            .map(|page| {
                shosai_core::search::extract_text_from_nodes(
                    &page
                        .iter()
                        .map(|page_node| page_node.node.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .find(|text| text.contains(label))
            .expect("the linked label must be paginated");

        assert!(
            page_text.contains("Long first entry"),
            "the linked label must not be left on a page by itself"
        );
    }

    #[test]
    fn epub_paginator_reserves_scaled_width_for_chapter_titles() {
        let nodes = vec![ContentNode::Paragraph(
            vec![shosai_core::epub::render::TextSpan {
                text: "Body text".to_string(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            Default::default(),
        )];
        let title = "Chapter ".repeat(6);

        let pages = paginate_epub_chapter(&nodes, Some(&title), 16.0, 1.6, Size::new(240.0, 150.0));

        assert_eq!(pages.len(), 2);
        assert!(
            pages[0].is_empty(),
            "the title should occupy the first page"
        );
        assert!(matches!(
            pages[1].as_slice(),
            [PageNode {
                node: ContentNode::Paragraph(_, _),
                ..
            }]
        ));
    }

    #[test]
    fn first_authored_margin_remains_reserved_after_moving_past_the_title_page() {
        let paragraph = table_test_cell("following", None).children.remove(0);
        let mut table = one_line_table(1);
        let ContentNode::Table { style, .. } = &mut table else {
            unreachable!();
        };
        style.block_before_em = Some(3.0);
        style.block_after_em = Some(0.0);

        let pages = paginate_epub_chapter(
            &[table, paragraph],
            Some("Title"),
            16.0,
            1.4,
            Size::new(360.0, 120.0),
        );

        assert_eq!(pages.len(), 3);
        assert!(pages[0].is_empty());
        assert!(matches!(
            pages[1].as_slice(),
            [PageNode {
                node: ContentNode::Table { .. },
                block_before: 48.0,
                ..
            }]
        ));
        assert!(matches!(
            pages[2].as_slice(),
            [PageNode {
                node: ContentNode::Paragraph(..),
                ..
            }]
        ));
    }

    #[test]
    fn tall_first_image_fits_inside_its_title_page_margins() {
        let image = ContentNode::Image {
            src: "portrait.png".into(),
            alt: String::new(),
            style: shosai_core::epub::render::NodeStyle {
                block_before_em: Some(3.0),
                block_after_em: Some(0.0),
                ..Default::default()
            },
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 100,
                height: 1_000,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&image),
            Some("Title"),
            16.0,
            1.4,
            Size::new(360.0, 120.0),
        );
        let page_node = &pages[1][0];
        let layout = epub_image_layout(
            &page_node.node,
            16.0,
            360.0,
            Some(120.0),
            Some(120.0 - page_node.block_before - page_node.block_after),
            None,
        )
        .unwrap();

        assert_eq!(page_node.block_before, 48.0);
        assert!(layout.total_height() + page_node.block_before + page_node.block_after <= 120.0);
    }

    #[test]
    fn final_table_image_reserves_the_tables_trailing_margin() {
        let mut table = one_line_table(1);
        let ContentNode::Table {
            row_groups, style, ..
        } = &mut table
        else {
            unreachable!();
        };
        row_groups[0].rows[0].cells[0].children = vec![ContentNode::Image {
            src: "portrait.png".into(),
            alt: String::new(),
            style: Default::default(),
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 100,
                height: 1_000,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        }];
        style.block_before_em = Some(0.0);
        style.block_after_em = Some(2.0);
        let column_widths = epub_table_column_widths(row_groups, 360.0);
        let geometry =
            epub_table_geometry_bounded(row_groups, &column_widths, 10, 16.0, 120.0 - 32.0, None);

        assert!(geometry.height + 32.0 <= 120.0);
    }

    #[test]
    fn epub_paginator_wraps_enlarged_paragraphs_at_their_scaled_width() {
        let style = shosai_core::epub::render::NodeStyle {
            font_size_multiplier: Some(2.0),
            ..Default::default()
        };
        let text = "Enlarged paragraph text should wrap more tightly";
        let nodes = vec![ContentNode::Paragraph(
            vec![shosai_core::epub::render::TextSpan {
                text: text.to_string(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            style,
        )];

        let pages = paginate_epub_chapter(&nodes, None, 16.0, 1.6, Size::new(240.0, 110.0));

        assert!(pages.len() > 1, "enlarged text must use its scaled width");
    }

    #[test]
    fn epub_paginator_uses_inherited_list_font_sizes_for_geometry() {
        let list = |scale| {
            ContentNode::UnorderedList(
                (0..4)
                    .map(|_| {
                        vec![shosai_core::epub::render::TextSpan {
                            text: "A list item with enough text to wrap across lines".to_string(),
                            math: None,
                            font_family: None,
                            bold: false,
                            italic: false,
                            monospace: false,
                            font_size_multiplier: scale,
                            preserve_whitespace: false,
                            link: None,
                        }]
                    })
                    .collect(),
            )
        };
        let paginate =
            |scale| paginate_epub_chapter(&[list(scale)], None, 16.0, 1.6, Size::new(240.0, 180.0));

        let small = paginate(0.5);
        let normal = paginate(1.0);
        let large = paginate(2.0);

        assert!(
            small.len() < normal.len(),
            "small list text should pack tighter"
        );
        assert!(
            normal.len() < large.len(),
            "large list text should consume more pages"
        );
        let ContentNode::UnorderedList(items) = list(0.5) else {
            unreachable!()
        };
        assert_eq!(spans_font_scale(&items[0]), 0.5);
    }

    #[test]
    fn epub_paginator_keeps_a_small_image_with_preceding_text() {
        let pages = paginate_epub_chapter(
            &[
                ContentNode::Paragraph(
                    vec![shosai_core::epub::render::TextSpan {
                        text: "Text before the image".to_string(),
                        math: None,
                        font_family: None,
                        bold: false,
                        italic: false,
                        monospace: false,
                        font_size_multiplier: 1.0,
                        preserve_whitespace: false,
                        link: None,
                    }],
                    Default::default(),
                ),
                ContentNode::Image {
                    src: "portrait.png".to_string(),
                    alt: "Portrait".to_string(),
                    style: Default::default(),
                    caption: Vec::new(),
                    caption_style: None,
                    kind: Some(shosai_core::epub::render::ImageKind::Raster),
                    intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                        width: 120,
                        height: 60,
                    }),
                },
            ],
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
        );

        assert_eq!(pages.len(), 1);
        assert!(matches!(
            pages[0].as_slice(),
            [
                PageNode {
                    node: ContentNode::Paragraph(..),
                    ..
                },
                PageNode {
                    node: ContentNode::Image { .. },
                    ..
                }
            ]
        ));
    }

    #[test]
    fn epub_paginator_moves_only_an_oversized_figure_to_the_next_page() {
        let paragraph = ContentNode::Paragraph(
            vec![shosai_core::epub::render::TextSpan {
                text: "Text before the image".into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            Default::default(),
        );
        let figure = ContentNode::Image {
            src: "portrait.png".into(),
            alt: "Portrait".into(),
            style: Default::default(),
            caption: Vec::new(),
            caption_style: None,
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 300,
                height: 600,
            }),
        };

        let pages = paginate_epub_chapter(
            &[paragraph, figure],
            None,
            16.0,
            1.6,
            Size::new(240.0, 180.0),
        );

        assert_eq!(pages.len(), 2);
        assert!(matches!(
            pages[1].as_slice(),
            [PageNode {
                node: ContentNode::Image { .. },
                ..
            }]
        ));
    }

    #[test]
    fn image_layout_uses_intrinsic_size_and_authored_max_width_without_upscaling() {
        let image = ContentNode::Image {
            src: "diagram.png".into(),
            alt: "Diagram".into(),
            style: shosai_core::epub::render::NodeStyle {
                max_width: Some(shosai_core::epub::render::NodeWidth::Percent(0.95)),
                ..Default::default()
            },
            caption: Vec::new(),
            caption_style: None,
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 125,
                height: 75,
            }),
        };

        let layout =
            epub_image_layout(&image, 16.0, 600.0, Some(800.0), Some(800.0), None).unwrap();

        assert_eq!(layout.width, 125.0);
        assert_eq!(layout.height, 75.0);
    }

    #[test]
    fn explicit_pixel_width_intentionally_upscales_to_exact_resolved_rectangle() {
        let image = ContentNode::Image {
            src: "narrow.png".into(),
            alt: String::new(),
            style: shosai_core::epub::render::NodeStyle {
                width: Some(shosai_core::epub::render::NodeWidth::Pixels(300.0)),
                ..Default::default()
            },
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 100,
                height: 50,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let layout =
            epub_image_layout(&image, 16.0, 400.0, Some(500.0), Some(500.0), None).unwrap();
        assert_eq!((layout.width, layout.height), (300.0, 150.0));
        assert_eq!(layout.total_height(), 150.0);
    }

    #[test]
    fn missing_image_uses_fallback_geometry_and_retains_caption_measurement() {
        let image = ContentNode::Image {
            src: "missing.png".into(),
            alt: "missing".into(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "A caption that remains visible".into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: None,
            intrinsic_size: None,
            kind: None,
        };
        let layout =
            epub_image_layout(&image, 16.0, 240.0, Some(800.0), Some(800.0), None).unwrap();
        assert_eq!(layout.height, 16.0 * TEXT_LINE_HEIGHT);
        assert!(layout.caption_height > 0.0);
        assert!(layout.total_height() < 800.0 / 2.0);
    }

    #[test]
    fn missing_image_fallback_is_bounded_by_its_paint_height() {
        let image = ContentNode::Image {
            src: "missing.png".into(),
            alt: "unbounded fallback text ".repeat(10_000),
            style: Default::default(),
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: None,
            kind: None,
        };
        let layout =
            epub_image_layout(&image, 16.0, 240.0, Some(120.0), Some(120.0), None).unwrap();
        assert!(layout.total_height() <= 120.0);

        let mut table = one_line_table(1);
        let ContentNode::Table { row_groups, .. } = &mut table else {
            unreachable!();
        };
        row_groups[0].rows[0].cells[0].children = vec![image];
        assert!(
            estimated_epub_compact_node_height_bounded(
                &table,
                30,
                10,
                16.0,
                240.0,
                120.0,
                Some(120.0),
            ) <= 120.0
        );
    }

    #[test]
    fn narrow_image_caption_is_measured_at_the_painted_width_without_fonts() {
        let image = ContentNode::Image {
            src: "narrow.png".into(),
            alt: String::new(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "A very long caption that must wrap over several lines at this narrow width"
                    .into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 80,
                height: 40,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let layout =
            epub_image_layout(&image, 16.0, 500.0, Some(600.0), Some(600.0), None).unwrap();
        assert_eq!(layout.width, 80.0);
        assert!(layout.caption_height > 16.0 * TEXT_LINE_HEIGHT);
    }

    #[test]
    fn direct_image_margin_reduces_the_painted_content_box() {
        let image = ContentNode::Image {
            src: "image.png".into(),
            alt: String::new(),
            style: shosai_core::epub::render::NodeStyle {
                margin_left_em: Some(1.0),
                ..Default::default()
            },
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 500,
                height: 250,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let layout = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();

        assert_eq!(
            epub_image_margin_left(image.style().unwrap(), 16.0, 500.0),
            16.0
        );
        assert_eq!(layout.width, 484.0);
        assert_eq!(layout.width + 16.0, 500.0);
    }

    #[test]
    fn fallback_caption_measurement_counts_explicit_newlines_and_wrapped_segments() {
        let mut image = ContentNode::Image {
            src: "narrow.png".into(),
            alt: String::new(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "abcdefghij\n\nklmnopqrst".into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: true,
                link: None,
            }],
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 44,
                height: 22,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let layout = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();
        // At five characters per line, both ten-character runs wrap twice and
        // the empty explicit line contributes one more line.
        assert_eq!(layout.caption_height, 5.0 * 16.0 * TEXT_LINE_HEIGHT);

        if let ContentNode::Image { caption, .. } = &mut image {
            caption[0].text = "abcdefghijklmnopqrst".into();
        }
        let without_newlines = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();
        assert!(layout.caption_height > without_newlines.caption_height);
    }

    #[test]
    fn indefinite_height_ignores_percent_but_honors_pixel_height() {
        let mut image = ContentNode::Image {
            src: "image.png".into(),
            alt: String::new(),
            style: shosai_core::epub::render::NodeStyle {
                height: Some(shosai_core::epub::render::NodeWidth::Percent(0.5)),
                ..Default::default()
            },
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 200,
                height: 100,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };

        let percentage = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();
        assert_eq!((percentage.width, percentage.height), (200.0, 100.0));

        if let ContentNode::Image { style, .. } = &mut image {
            style.height = Some(shosai_core::epub::render::NodeWidth::Pixels(250.0));
        }
        let pixels = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();
        assert_eq!((pixels.width, pixels.height), (500.0, 250.0));
    }

    #[test]
    fn percentage_height_basis_is_independent_from_the_paint_bound() {
        let image = ContentNode::Image {
            src: "image.png".into(),
            alt: String::new(),
            style: shosai_core::epub::render::NodeStyle {
                height: Some(shosai_core::epub::render::NodeWidth::Percent(1.0)),
                ..Default::default()
            },
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 200,
                height: 100,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };

        let auto_height =
            epub_image_layout(&image, 16.0, 1_000.0, None, Some(300.0), None).unwrap();
        let definite_height =
            epub_image_layout(&image, 16.0, 1_000.0, Some(300.0), Some(300.0), None).unwrap();

        assert_eq!((auto_height.width, auto_height.height), (200.0, 100.0));
        assert_eq!(
            (definite_height.width, definite_height.height),
            (600.0, 300.0)
        );
    }

    #[test]
    fn fallback_caption_reserves_enlarged_inline_span_geometry() {
        let mut image = ContentNode::Image {
            src: "image.png".into(),
            alt: String::new(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "caption".into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 200,
                height: 100,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let normal = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();
        let ContentNode::Image { caption, .. } = &mut image else {
            unreachable!();
        };
        caption[0].font_size_multiplier = 2.0;
        let enlarged = epub_image_layout(&image, 16.0, 500.0, None, None, None).unwrap();

        assert!(enlarged.caption_height >= normal.caption_height * 2.0);
    }

    #[test]
    fn embedded_font_caption_reserves_inline_math_geometry() {
        use shosai_core::epub::{MathContent, MathDisplay, MathExpression};

        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/fonts.epub").to_vec(),
        )
        .unwrap();
        let mut span = shosai_core::epub::render::TextSpan {
            text: "x/y".into(),
            math: None,
            font_family: Some("FixtureTtf".into()),
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        };
        assert!(
            measure_epub_spans(
                Some(epub.fonts()),
                std::slice::from_ref(&span),
                16.0,
                200.0,
                Default::default(),
                None,
            )
            .is_some()
        );
        let image = |caption| ContentNode::Image {
            src: "image.png".into(),
            alt: String::new(),
            style: Default::default(),
            caption,
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 200,
                height: 100,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let plain = epub_image_layout(
            &image(vec![span.clone()]),
            16.0,
            500.0,
            None,
            None,
            Some(epub.fonts()),
        )
        .unwrap();
        span.math = Some(MathContent {
            display: MathDisplay::Inline,
            expression: Some(MathExpression::Fraction(
                Box::new(MathExpression::Token("x".into())),
                Box::new(MathExpression::Token("y".into())),
            )),
            fallback: "x/y".into(),
        });
        let math = epub_image_layout(
            &image(vec![span]),
            16.0,
            500.0,
            None,
            None,
            Some(epub.fonts()),
        )
        .unwrap();

        assert!(math.caption_height > plain.caption_height);
    }

    #[test]
    fn nested_figure_uses_embedded_font_image_caption_geometry() {
        let epub = shosai_core::epub::EpubDoc::from_bytes(
            include_bytes!("../../tests/fixtures/epub-conformance/fonts.epub").to_vec(),
        )
        .unwrap();
        let image = ContentNode::Image {
            src: "image.png".into(),
            alt: String::new(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "A long embedded-font caption that wraps at the figure width".into(),
                math: None,
                font_family: Some("FixtureTtf".into()),
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 120,
                height: 60,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let figure = ContentNode::Figure {
            children: vec![image.clone()],
            style: Default::default(),
        };
        let expected =
            epub_image_layout(&image, 16.0, 500.0, None, Some(600.0), Some(epub.fonts()))
                .unwrap()
                .total_height()
                + epub_node_list_spacing(std::slice::from_ref(&image), 16.0, BLOCKQUOTE_SPACING);

        assert_eq!(
            measured_epub_compact_node_height_bounded(
                Some(epub.fonts()),
                &figure,
                16.0,
                500.0,
                600.0,
            ),
            Some(expected)
        );
    }

    #[test]
    fn oversized_image_caption_fragments_without_losing_text() {
        let image = ContentNode::Image {
            src: "image.png".into(),
            alt: "diagram".into(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "long caption ".repeat(200),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 120,
                height: 80,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&image),
            None,
            16.0,
            1.4,
            Size::new(300.0, 180.0),
        );
        let paginated = pages
            .iter()
            .flatten()
            .map(|page_node| page_node.node.clone())
            .collect::<Vec<_>>();

        assert!(pages.len() > 1);
        assert!(matches!(pages[0][0].node, ContentNode::Image { .. }));
        assert!(
            pages
                .iter()
                .skip(1)
                .flatten()
                .any(|node| matches!(node.node, ContentNode::Paragraph(..)))
        );
        assert_eq!(
            shosai_core::search::extract_text_from_nodes(std::slice::from_ref(&image))
                .replace('\n', ""),
            shosai_core::search::extract_text_from_nodes(&paginated).replace('\n', "")
        );
    }

    #[test]
    fn image_caption_can_fragment_before_its_first_character() {
        let caption_text = "caption that cannot share the image page";
        let image = ContentNode::Image {
            src: "image.png".into(),
            alt: "diagram".into(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: caption_text.into(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: Some(shosai_core::epub::render::NodeStyle {
                font_size_multiplier: Some(32.0),
                ..Default::default()
            }),
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 120,
                height: 80,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };

        let (image_fragment, caption_fragment, consumed) =
            split_epub_image_caption(&image, 16.0, 300.0, 180.0, None)
                .expect("oversized caption must split from the image");

        assert!(matches!(
            image_fragment,
            ContentNode::Image { caption, .. } if caption.is_empty()
        ));
        assert_eq!(
            shosai_core::search::extract_text_from_nodes(std::slice::from_ref(&caption_fragment))
                .trim_end_matches('\n'),
            caption_text
        );
        assert_eq!(consumed, "diagram".chars().count() + 1);
        let ContentNode::Paragraph(_, style) = caption_fragment else {
            unreachable!();
        };
        assert_eq!(
            paragraph_width(300.0, 16.0, &style),
            120.0,
            "detached caption must retain the image's resolved width"
        );
        assert_eq!(style.margin_left_em, Some(90.0 / 16.0));

        for (authored_margin, expected_width, expected_left) in
            [(100.0, 1.0, 299.0), (-5.0, 120.0, 90.0)]
        {
            let mut variant = image.clone();
            let ContentNode::Image { style, .. } = &mut variant else {
                unreachable!();
            };
            style.margin_left_em = Some(authored_margin);
            let (_, caption_fragment, _) =
                split_epub_image_caption(&variant, 16.0, 300.0, 180.0, None).unwrap();
            let ContentNode::Paragraph(_, style) = caption_fragment else {
                unreachable!();
            };
            assert_eq!(paragraph_width(300.0, 16.0, &style), expected_width);
            assert_eq!(style.margin_left_em, Some(expected_left / 16.0));
        }
    }

    #[test]
    fn retained_multi_image_figure_uses_shared_page_height() {
        let image = |src: &str| ContentNode::Image {
            src: src.into(),
            alt: src.into(),
            style: Default::default(),
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 200,
                height: 300,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };
        let figure = ContentNode::Figure {
            children: vec![image("first"), image("second")],
            style: Default::default(),
        };
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&figure),
            None,
            16.0,
            1.4,
            Size::new(300.0, 180.0),
        );
        let fragments = pages
            .iter()
            .flatten()
            .filter_map(|page_node| match &page_node.node {
                ContentNode::Figure { children, .. } => Some(children),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(fragments.len(), 2);
        assert!(fragments.iter().all(|children| children.len() == 1));
        assert_eq!(
            shosai_core::search::extract_text_from_nodes(std::slice::from_ref(&figure)),
            shosai_core::search::extract_text_from_nodes(
                &pages
                    .iter()
                    .flatten()
                    .map(|page_node| page_node.node.clone())
                    .collect::<Vec<_>>()
            )
        );
    }

    #[test]
    fn oversized_table_caption_fragments_before_the_first_row() {
        let mut table = one_line_table(1);
        let ContentNode::Table { caption, style, .. } = &mut table else {
            unreachable!();
        };
        style.width = Some(shosai_core::epub::render::NodeWidth::Pixels(120.0));
        style.margin_left_em = Some(2.0);
        *caption = vec![shosai_core::epub::render::TextSpan {
            text: "long table caption ".repeat(200),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let pages = paginate_epub_chapter(
            std::slice::from_ref(&table),
            None,
            16.0,
            1.4,
            Size::new(360.0, 180.0),
        );
        let paginated = pages
            .iter()
            .flatten()
            .map(|page_node| page_node.node.clone())
            .collect::<Vec<_>>();

        assert!(
            pages
                .iter()
                .flatten()
                .any(|node| matches!(node.node, ContentNode::Paragraph(..)))
        );
        assert!(pages.iter().flatten().any(|node| matches!(
            &node.node,
            ContentNode::Table {
                caption,
                row_groups,
                ..
            } if !caption.is_empty() && !row_groups.is_empty()
        )));
        assert!(
            pages
                .iter()
                .flatten()
                .filter_map(|node| match &node.node {
                    ContentNode::Paragraph(_, style) => Some(style),
                    _ => None,
                })
                .all(|style| {
                    paragraph_width(360.0, 16.0, style) == 120.0
                        && style.margin_left_em == Some(2.0)
                })
        );
        assert_eq!(
            shosai_core::search::extract_text_from_nodes(std::slice::from_ref(&table))
                .replace('\n', ""),
            shosai_core::search::extract_text_from_nodes(&paginated).replace('\n', "")
        );
    }

    #[test]
    fn table_caption_can_fragment_entirely_before_a_tall_first_row() {
        let caption = vec![shosai_core::epub::render::TextSpan {
            text: "caption".into(),
            math: None,
            font_family: None,
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let style = shosai_core::epub::render::NodeStyle {
            font_size_multiplier: Some(32.0),
            ..Default::default()
        };

        let (prefix, suffix) =
            split_epub_caption_suffix(&caption, Some(&style), 16.0, 300.0, 1.0, None)
                .expect("caption must move ahead of a row when no character fits");

        assert_eq!(spans_text_len(&prefix), spans_text_len(&caption));
        assert!(suffix.is_empty());
    }

    #[test]
    fn caption_only_table_fragments_without_losing_text() {
        let caption_text = "caption-only table ".repeat(200);
        let table = ContentNode::Table {
            caption: vec![shosai_core::epub::render::TextSpan {
                text: caption_text.clone(),
                math: None,
                font_family: None,
                bold: false,
                italic: false,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: None,
            row_groups: Vec::new(),
            style: shosai_core::epub::render::NodeStyle {
                width: Some(shosai_core::epub::render::NodeWidth::Pixels(120.0)),
                margin_left_em: Some(2.0),
                ..Default::default()
            },
        };

        let pages = paginate_epub_chapter(
            std::slice::from_ref(&table),
            None,
            16.0,
            1.4,
            Size::new(300.0, 180.0),
        );
        let fragments = pages
            .iter()
            .flatten()
            .map(|node| node.node.clone())
            .collect::<Vec<_>>();

        assert!(pages.len() > 1);
        assert!(
            fragments
                .iter()
                .all(|node| matches!(node, ContentNode::Paragraph(..)))
        );
        assert!(fragments.iter().all(|node| match node {
            ContentNode::Paragraph(_, style) => {
                paragraph_width(300.0, 16.0, style) == 120.0 && style.margin_left_em == Some(2.0)
            }
            _ => false,
        }));
        assert_eq!(
            shosai_core::search::extract_text_from_nodes(&fragments).replace('\n', ""),
            caption_text
        );
    }

    #[test]
    fn raster_fallback_geometry_remains_intrinsically_bounded() {
        let image = ContentNode::Image {
            src: "undecodable.png".into(),
            alt: "very long fallback text ".repeat(200),
            style: Default::default(),
            caption: Vec::new(),
            caption_style: None,
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 2,
                height: 1,
            }),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
        };

        let layout = epub_image_layout(&image, 16.0, 300.0, Some(180.0), Some(180.0), None)
            .expect("intrinsic image geometry must remain available to fallback painting");

        assert_eq!((layout.width, layout.height), (2.0, 1.0));
    }

    #[test]
    fn oversized_figure_scales_with_its_caption_inside_page_bounds() {
        let image = ContentNode::Image {
            src: "portrait.png".into(),
            alt: "Portrait".into(),
            style: Default::default(),
            caption: vec![shosai_core::epub::render::TextSpan {
                text: "Figure 1. A retained caption".into(),
                math: None,
                font_family: None,
                bold: false,
                italic: true,
                monospace: false,
                font_size_multiplier: 1.0,
                preserve_whitespace: false,
                link: None,
            }],
            caption_style: Some(Default::default()),
            kind: Some(shosai_core::epub::render::ImageKind::Raster),
            intrinsic_size: Some(shosai_core::epub::render::ImageSize {
                width: 600,
                height: 1_200,
            }),
        };

        let layout =
            epub_image_layout(&image, 16.0, 400.0, Some(300.0), Some(300.0), None).unwrap();

        assert!(layout.width < 400.0);
        assert!(layout.caption_height > 0.0);
        assert!(layout.total_height() <= 300.0 + f32::EPSILON);
    }

    #[test]
    fn measured_paragraph_splits_preserve_unicode_clusters_text_and_offsets() {
        let text = "Aé 👩\u{200d}🔬 B";
        let spans = vec![shosai_core::epub::render::TextSpan {
            text: text.into(),
            math: None,
            font_family: Some("Book Alias".into()),
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let layout = EpubTextLayout {
            width: 100.0,
            height: 40.0,
            lines: vec![
                shosai_core::epub::EpubTextLine {
                    top: 0.0,
                    width: 40.0,
                    rtl: false,
                    scalars: 0..3,
                    pixel_width: 0,
                    pixel_height: 20,
                    rgba: Vec::new(),
                },
                shosai_core::epub::EpubTextLine {
                    top: 20.0,
                    width: 60.0,
                    rtl: false,
                    scalars: 3..8,
                    pixel_width: 0,
                    pixel_height: 20,
                    rgba: Vec::new(),
                },
            ],
            links: Vec::new(),
            endpoints: Vec::new(),
        };
        let measured_lengths = std::cell::RefCell::new(Vec::new());
        let measure = |spans: &[shosai_core::epub::render::TextSpan]| {
            let scalars = spans_text_len(spans);
            measured_lengths.borrow_mut().push(scalars);
            if scalars == 8 {
                return Some(layout.clone());
            }
            Some(EpubTextLayout {
                width: 60.0,
                height: 20.0,
                lines: vec![shosai_core::epub::EpubTextLine {
                    top: 0.0,
                    width: 60.0,
                    rtl: false,
                    scalars: 0..scalars,
                    pixel_width: 0,
                    pixel_height: 20,
                    rgba: Vec::new(),
                }],
                links: Vec::new(),
                endpoints: Vec::new(),
            })
        };
        let mut pages = vec![Vec::new()];
        let mut remaining = 25.0;
        assert!(paginate_measured_paragraph(
            &spans,
            &Default::default(),
            &measure,
            10,
            4.0,
            25.0,
            false,
            &mut pages,
            &mut remaining,
            &mut EpubPaginationBudget::default(),
        ));

        let chunks = pages
            .iter()
            .flat_map(|page| page.iter())
            .map(|node| {
                let ContentNode::Paragraph(spans, _) = &node.node else {
                    panic!("measured paragraph must remain a paragraph");
                };
                (
                    node.text_offset,
                    spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chunks,
            vec![(10, "Aé ".into()), (13, "👩\u{200d}🔬 B".into())]
        );
        assert_eq!(
            chunks
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<String>(),
            text
        );
        assert!(
            measured_lengths
                .borrow()
                .windows(3)
                .any(|calls| calls == [8, 3, 5])
        );
    }

    #[test]
    fn measured_pagination_bounds_each_native_shaping_request() {
        let text = "x".repeat(EPUB_PAGINATION_SHAPE_CHUNK * 3 + 17);
        let spans = vec![shosai_core::epub::render::TextSpan {
            text: text.clone(),
            math: None,
            font_family: Some("Book".into()),
            bold: false,
            italic: false,
            monospace: false,
            font_size_multiplier: 1.0,
            preserve_whitespace: false,
            link: None,
        }];
        let measured = std::cell::RefCell::new(Vec::new());
        let measure = |spans: &[shosai_core::epub::render::TextSpan]| {
            let count = spans_text_len(spans);
            measured.borrow_mut().push(count);
            Some(EpubTextLayout {
                width: 100.0,
                height: count as f32 * 20.0,
                lines: (0..count)
                    .map(|index| shosai_core::epub::EpubTextLine {
                        top: index as f32 * 20.0,
                        width: 100.0,
                        rtl: false,
                        scalars: index..index + 1,
                        pixel_width: 0,
                        pixel_height: 20,
                        rgba: Vec::new(),
                    })
                    .collect(),
                links: Vec::new(),
                endpoints: Vec::new(),
            })
        };
        let mut pages = vec![Vec::new()];
        let mut remaining = 20.0;
        assert!(paginate_measured_paragraph(
            &spans,
            &Default::default(),
            &measure,
            0,
            0.0,
            20.0,
            false,
            &mut pages,
            &mut remaining,
            &mut EpubPaginationBudget::default(),
        ));

        assert!(
            measured
                .borrow()
                .iter()
                .all(|count| *count <= EPUB_PAGINATION_SHAPE_CHUNK)
        );
        assert!(
            measured.borrow().iter().sum::<usize>() <= text.len() * 4 + EPUB_PAGINATION_SHAPE_CHUNK,
            "overlapping native suffix work must remain linear in paragraph size"
        );
        let retained = pages
            .iter()
            .flatten()
            .filter_map(|page| match &page.node {
                ContentNode::Paragraph(spans, _) => Some(
                    spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(retained, text);
    }

    #[test]
    fn readable_width_caps_long_lines() {
        let page = page_size(Size::new(2_000.0, 900.0), false, 20.0, 16.0, 1.6);
        let characters = page.width / (16.0 * AVERAGE_CHARACTER_WIDTH);
        assert!((characters - MAX_CHARACTERS_PER_LINE as f32).abs() < 0.01);
    }

    #[test]
    fn visible_pages_follow_horizontal_spreads() {
        assert_eq!(visible_pages(0, 3, true), vec![0, 1]);
        assert_eq!(visible_pages(2, 3, true), vec![2]);
    }
}
