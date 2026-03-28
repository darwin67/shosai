use std::path::PathBuf;

use iced::keyboard;
use iced::widget::{
    button, center, column, container, image, rich_text, row, scrollable, span, text, text_input,
};
use iced::{Element, Font, Length, Subscription, Task};

use shosai_core::document::{Document, RenderedPage};
use shosai_core::epub::EpubDoc;
use shosai_core::epub::render::{ContentNode, parse_chapter_xhtml};
use shosai_core::pdf::PdfDoc;
use shosai_core::reading_state::{FileReadingState, ReadingStateStore};

// ---------------------------------------------------------------------------
// Open document wrapper
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum OpenDocument {
    Pdf(PdfDoc),
    Epub(EpubDoc),
}

// ---------------------------------------------------------------------------
// Zoom (PDF only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    Manual(f32),
    FitWidth,
    FitPage,
}

impl ZoomMode {
    fn scale(&self) -> f32 {
        match self {
            ZoomMode::Manual(s) => *s,
            ZoomMode::FitWidth => 1.0,
            ZoomMode::FitPage => 1.0,
        }
    }

    fn label(&self) -> String {
        match self {
            ZoomMode::Manual(s) => format!("{}%", (s * 100.0) as u32),
            ZoomMode::FitWidth => "Fit Width".to_string(),
            ZoomMode::FitPage => "Fit Page".to_string(),
        }
    }
}

impl Default for ZoomMode {
    fn default() -> Self {
        ZoomMode::Manual(1.0)
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct State {
    file_path: Option<PathBuf>,
    document: Option<OpenDocument>,
    /// Current page (PDF) or chapter (EPUB) index (0-based).
    current_page: usize,
    /// Total pages (PDF) or chapters (EPUB).
    total_pages: usize,
    /// Zoom mode (PDF only).
    zoom: ZoomMode,
    /// Cached rendered PDF page.
    rendered_page: Option<RenderedPage>,
    /// Cached parsed EPUB chapter content nodes.
    chapter_content: Vec<ContentNode>,
    /// Page/chapter input field text.
    page_input: String,
    error: Option<String>,
    reading_state: Option<ReadingStateStore>,
    /// EPUB font size in pixels.
    font_size: f32,
    /// EPUB line spacing multiplier (1.0 = normal).
    line_spacing: f32,
    /// Reader color theme.
    theme: ReaderTheme,
}

/// Color theme for the EPUB reader.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ReaderTheme {
    #[default]
    Light,
    Dark,
    Sepia,
}

impl ReaderTheme {
    fn background(&self) -> iced::Color {
        match self {
            ReaderTheme::Light => iced::Color::WHITE,
            ReaderTheme::Dark => iced::Color::from_rgb(0.12, 0.12, 0.14),
            ReaderTheme::Sepia => iced::Color::from_rgb(0.96, 0.92, 0.84),
        }
    }

    fn text_color(&self) -> iced::Color {
        match self {
            ReaderTheme::Light => iced::Color::from_rgb(0.1, 0.1, 0.1),
            ReaderTheme::Dark => iced::Color::from_rgb(0.85, 0.85, 0.85),
            ReaderTheme::Sepia => iced::Color::from_rgb(0.3, 0.2, 0.1),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ReaderTheme::Light => "Light",
            ReaderTheme::Dark => "Dark",
            ReaderTheme::Sepia => "Sepia",
        }
    }

    fn next(&self) -> Self {
        match self {
            ReaderTheme::Light => ReaderTheme::Dark,
            ReaderTheme::Dark => ReaderTheme::Sepia,
            ReaderTheme::Sepia => ReaderTheme::Light,
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    // File
    OpenFile,
    FileSelected(Option<PathBuf>),

    // Navigation
    NextPage,
    PrevPage,
    PageInputChanged(String),
    GoToPage,

    // Zoom (PDF)
    ZoomIn,
    ZoomOut,
    SetZoomFitWidth,
    SetZoomFitPage,

    // EPUB reading controls
    FontSizeUp,
    FontSizeDown,
    CycleTheme,

    // Keyboard
    KeyPressed(keyboard::Event),
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

pub fn boot() -> (State, Task<Message>) {
    let reading_state = match ReadingStateStore::open() {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("warning: failed to open reading state database: {e}");
            None
        }
    };

    let state = State {
        file_path: None,
        document: None,
        current_page: 0,
        total_pages: 0,
        zoom: ZoomMode::default(),
        rendered_page: None,
        chapter_content: Vec::new(),
        page_input: String::new(),
        error: None,
        reading_state,
        font_size: 16.0,
        line_spacing: 1.6,
        theme: ReaderTheme::default(),
    };
    (state, Task::none())
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::OpenFile => {
            return Task::perform(
                async {
                    let file = rfd::AsyncFileDialog::new()
                        .add_filter("Ebooks", &["pdf", "epub"])
                        .add_filter("PDF", &["pdf"])
                        .add_filter("EPUB", &["epub"])
                        .set_title("Open File")
                        .pick_file()
                        .await;

                    file.map(|f| f.path().to_path_buf())
                },
                Message::FileSelected,
            );
        }

        Message::FileSelected(Some(path)) => {
            open_file(state, path);
        }

        Message::FileSelected(None) => {}

        Message::NextPage => {
            if state.document.is_some() && state.current_page + 1 < state.total_pages {
                state.current_page += 1;
                state.page_input = format!("{}", state.current_page + 1);
                refresh_content(state);
                save_reading_state(state);
            }
        }

        Message::PrevPage => {
            if state.document.is_some() && state.current_page > 0 {
                state.current_page -= 1;
                state.page_input = format!("{}", state.current_page + 1);
                refresh_content(state);
                save_reading_state(state);
            }
        }

        Message::PageInputChanged(value) => {
            state.page_input = value;
        }

        Message::GoToPage => {
            if let Ok(page_num) = state.page_input.parse::<usize>()
                && page_num >= 1
                && page_num <= state.total_pages
            {
                state.current_page = page_num - 1;
                refresh_content(state);
                save_reading_state(state);
            }
            state.page_input = format!("{}", state.current_page + 1);
        }

        Message::ZoomIn => {
            if matches!(state.document, Some(OpenDocument::Pdf(_))) {
                let new_scale = (state.zoom.scale() + 0.25).min(5.0);
                state.zoom = ZoomMode::Manual(new_scale);
                refresh_content(state);
                save_reading_state(state);
            }
        }

        Message::ZoomOut => {
            if matches!(state.document, Some(OpenDocument::Pdf(_))) {
                let new_scale = (state.zoom.scale() - 0.25).max(0.25);
                state.zoom = ZoomMode::Manual(new_scale);
                refresh_content(state);
                save_reading_state(state);
            }
        }

        Message::SetZoomFitWidth => {
            state.zoom = ZoomMode::FitWidth;
            refresh_content(state);
            save_reading_state(state);
        }

        Message::SetZoomFitPage => {
            state.zoom = ZoomMode::FitPage;
            refresh_content(state);
            save_reading_state(state);
        }

        Message::FontSizeUp => {
            state.font_size = (state.font_size + 2.0).min(48.0);
        }

        Message::FontSizeDown => {
            state.font_size = (state.font_size - 2.0).max(8.0);
        }

        Message::CycleTheme => {
            state.theme = state.theme.next();
        }

        Message::KeyPressed(event) => {
            return handle_key_event(event);
        }
    }

    Task::none()
}

fn open_file(state: &mut State, path: PathBuf) {
    state.error = None;
    state.rendered_page = None;
    state.chapter_content = Vec::new();

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let result: Result<(), String> = match ext.as_str() {
        "pdf" => match PdfDoc::open(&path) {
            Ok(doc) => {
                state.total_pages = doc.page_count();
                state.document = Some(OpenDocument::Pdf(doc));
                Ok(())
            }
            Err(e) => Err(format!("Failed to open PDF: {e}")),
        },
        "epub" => match EpubDoc::open(&path) {
            Ok(doc) => {
                state.total_pages = doc.chapter_count();
                state.document = Some(OpenDocument::Epub(doc));
                Ok(())
            }
            Err(e) => Err(format!("Failed to open EPUB: {e}")),
        },
        _ => Err(format!("Unsupported file format: .{ext}")),
    };

    match result {
        Ok(()) => {
            // Restore reading position.
            let saved = state
                .reading_state
                .as_ref()
                .and_then(|store| store.get(&path));
            if let Some(saved) = saved {
                state.current_page = saved.page.min(state.total_pages.saturating_sub(1));
                if matches!(state.document, Some(OpenDocument::Pdf(_))) {
                    state.zoom = ZoomMode::Manual(saved.zoom);
                }
            } else {
                state.current_page = 0;
                state.zoom = ZoomMode::Manual(1.0);
            }

            state.page_input = format!("{}", state.current_page + 1);
            state.file_path = Some(path);
            refresh_content(state);
        }
        Err(msg) => {
            state.error = Some(msg);
        }
    }
}

fn handle_key_event(event: keyboard::Event) -> Task<Message> {
    if let keyboard::Event::KeyPressed { key, modifiers, .. } = event {
        match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::ArrowRight)
            | keyboard::Key::Named(keyboard::key::Named::PageDown) => {
                return Task::done(Message::NextPage);
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft)
            | keyboard::Key::Named(keyboard::key::Named::PageUp) => {
                return Task::done(Message::PrevPage);
            }

            keyboard::Key::Character(c) if c == "=" || c == "+" => {
                return Task::done(Message::ZoomIn);
            }
            keyboard::Key::Character("-") => {
                return Task::done(Message::ZoomOut);
            }

            keyboard::Key::Character(c) if c == "o" && modifiers.command() => {
                return Task::done(Message::OpenFile);
            }

            _ => {}
        }
    }
    Task::none()
}

/// Refresh the visible content for the current page/chapter.
fn refresh_content(state: &mut State) {
    match &state.document {
        Some(OpenDocument::Pdf(doc)) => {
            let scale = state.zoom.scale();
            match doc.render_page(state.current_page, scale) {
                Ok(page) => {
                    state.rendered_page = Some(page);
                    state.chapter_content = Vec::new();
                    state.error = None;
                }
                Err(e) => {
                    state.error = Some(format!("Failed to render page: {e}"));
                    state.rendered_page = None;
                }
            }
        }
        Some(OpenDocument::Epub(doc)) => {
            state.rendered_page = None;
            if let Some(chapter) = doc.chapter(state.current_page) {
                let base_path = chapter
                    .path
                    .rsplit_once('/')
                    .map(|(dir, _)| dir)
                    .unwrap_or("");
                state.chapter_content = parse_chapter_xhtml(&chapter.content, base_path);
                state.error = None;
            } else {
                state.chapter_content = Vec::new();
                state.error = Some(format!("Chapter {} not found", state.current_page));
            }
        }
        None => {}
    }
}

fn save_reading_state(state: &State) {
    if let (Some(path), Some(store)) = (&state.file_path, &state.reading_state) {
        let reading = FileReadingState {
            page: state.current_page,
            zoom: state.zoom.scale(),
        };
        if let Err(e) = store.set(path, &reading) {
            eprintln!("warning: failed to save reading state: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(state: &State) -> Element<'_, Message> {
    let content = column![toolbar(state), content_view(state)]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill);

    content.into()
}

fn toolbar(state: &State) -> Element<'_, Message> {
    let open_btn = button("Open").on_press(Message::OpenFile);

    let has_doc = state.document.is_some();
    let is_pdf = matches!(state.document, Some(OpenDocument::Pdf(_)));
    let is_epub = matches!(state.document, Some(OpenDocument::Epub(_)));
    let can_prev = has_doc && state.current_page > 0;
    let can_next = has_doc && state.current_page + 1 < state.total_pages;

    let mut prev_btn = button("<");
    if can_prev {
        prev_btn = prev_btn.on_press(Message::PrevPage);
    }

    let mut next_btn = button(">");
    if can_next {
        next_btn = next_btn.on_press(Message::NextPage);
    }

    let nav_label = if is_epub { "Ch" } else { "Page" };

    let page_input = text_input(nav_label, &state.page_input)
        .on_input(Message::PageInputChanged)
        .on_submit(Message::GoToPage)
        .width(60);

    let page_label = text(if has_doc {
        format!("/ {}", state.total_pages)
    } else {
        String::new()
    });

    let mut toolbar_items: Vec<Element<'_, Message>> = vec![
        open_btn.into(),
        prev_btn.into(),
        page_input.into(),
        page_label.into(),
        next_btn.into(),
    ];

    // PDF: zoom controls
    if is_pdf || !has_doc {
        let zoom_out_btn = if is_pdf {
            button("-").on_press(Message::ZoomOut)
        } else {
            button("-")
        };
        let zoom_label = text(state.zoom.label()).width(70);
        let zoom_in_btn = if is_pdf {
            button("+").on_press(Message::ZoomIn)
        } else {
            button("+")
        };
        let mut fit_w = button("W");
        let mut fit_p = button("P");
        if is_pdf {
            fit_w = fit_w.on_press(Message::SetZoomFitWidth);
            fit_p = fit_p.on_press(Message::SetZoomFitPage);
        }

        toolbar_items.push(zoom_out_btn.into());
        toolbar_items.push(zoom_label.into());
        toolbar_items.push(zoom_in_btn.into());
        toolbar_items.push(fit_w.into());
        toolbar_items.push(fit_p.into());
    }

    // EPUB: font size + theme controls
    if is_epub {
        let size_label = text(format!("{}px", state.font_size as u32)).width(50);
        toolbar_items.push(button("A-").on_press(Message::FontSizeDown).into());
        toolbar_items.push(size_label.into());
        toolbar_items.push(button("A+").on_press(Message::FontSizeUp).into());
        toolbar_items.push(
            button(state.theme.label())
                .on_press(Message::CycleTheme)
                .into(),
        );
    }

    let toolbar_row = row(toolbar_items)
        .spacing(8)
        .align_y(iced::Alignment::Center);

    container(toolbar_row).padding(8).width(Length::Fill).into()
}

fn content_view(state: &State) -> Element<'_, Message> {
    if let Some(error) = &state.error {
        return center(text(error).size(16))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

    match &state.document {
        Some(OpenDocument::Pdf(_)) => pdf_page_view(state),
        Some(OpenDocument::Epub(_)) => epub_chapter_view(state),
        None => welcome_view(),
    }
}

fn pdf_page_view(state: &State) -> Element<'_, Message> {
    if let Some(rendered) = &state.rendered_page {
        let handle =
            image::Handle::from_rgba(rendered.width, rendered.height, rendered.pixels.clone());

        let img = image(handle)
            .width(Length::Fixed(rendered.width as f32))
            .height(Length::Fixed(rendered.height as f32));

        let page_container = container(img).width(Length::Fill).center_x(Length::Fill);

        scrollable(page_container)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        center(text("Rendering...").size(16))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn epub_chapter_view(state: &State) -> Element<'_, Message> {
    let font_size = state.font_size;
    let text_color = state.theme.text_color();
    let line_gap = state.font_size * state.line_spacing;

    let mut content_col = column![].spacing(line_gap).padding(20).width(Length::Fill);

    // Chapter title from the TOC if available.
    if let Some(OpenDocument::Epub(doc)) = &state.document
        && let Some(chapter) = doc.chapter(state.current_page)
        && let Some(title) = &chapter.title
    {
        content_col = content_col.push(text(title.clone()).size(font_size * 1.5).color(text_color));
    }

    for node in &state.chapter_content {
        content_col = content_col.push(render_content_node(node, font_size, text_color));
    }

    let padded = container(content_col)
        .max_width(800)
        .width(Length::Fill)
        .center_x(Length::Fill);

    let bg = state.theme.background();

    container(scrollable(padded).width(Length::Fill).height(Length::Fill))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            ..Default::default()
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn render_content_node<'a>(
    node: &ContentNode,
    font_size: f32,
    text_color: iced::Color,
) -> Element<'a, Message> {
    match node {
        ContentNode::Heading { level, text: t } => {
            let size = match level {
                1 => font_size * 2.0,
                2 => font_size * 1.6,
                3 => font_size * 1.3,
                4 => font_size * 1.1,
                _ => font_size,
            };
            text(t.clone()).size(size).color(text_color).into()
        }

        ContentNode::Paragraph(spans) => render_spans(spans, font_size, text_color),

        ContentNode::BlockQuote(children) => {
            let mut col = column![].spacing(8).padding(20);
            for child in children {
                col = col.push(render_content_node(child, font_size, text_color));
            }
            container(col).padding(16).width(Length::Fill).into()
        }

        ContentNode::UnorderedList(items) => {
            let mut col = column![].spacing(4);
            for item_spans in items {
                let bullet_text = "  \u{2022} ".to_string();
                let mut all_spans = vec![shosai_core::epub::render::TextSpan {
                    text: bullet_text,
                    bold: false,
                    italic: false,
                }];
                all_spans.extend(item_spans.iter().cloned());
                col = col.push(render_spans(&all_spans, font_size, text_color));
            }
            col.into()
        }

        ContentNode::OrderedList(items) => {
            let mut col = column![].spacing(4);
            for (i, item_spans) in items.iter().enumerate() {
                let num_text = format!("  {}. ", i + 1);
                let mut all_spans = vec![shosai_core::epub::render::TextSpan {
                    text: num_text,
                    bold: false,
                    italic: false,
                }];
                all_spans.extend(item_spans.iter().cloned());
                col = col.push(render_spans(&all_spans, font_size, text_color));
            }
            col.into()
        }

        ContentNode::Image { alt, .. } => {
            // TODO: load image from epub resources and display
            text(format!("[Image: {alt}]"))
                .size(font_size)
                .color(text_color)
                .into()
        }

        ContentNode::HorizontalRule => text("───────────────────")
            .size(font_size)
            .color(text_color)
            .into(),
    }
}

fn render_spans<'a>(
    spans: &[shosai_core::epub::render::TextSpan],
    font_size: f32,
    text_color: iced::Color,
) -> Element<'a, Message> {
    let rich_spans: Vec<iced::widget::text::Span<'a, Message>> = spans
        .iter()
        .map(|s| {
            let font = match (s.bold, s.italic) {
                (true, true) => Font {
                    weight: iced::font::Weight::Bold,
                    style: iced::font::Style::Italic,
                    ..Font::DEFAULT
                },
                (true, false) => Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::DEFAULT
                },
                (false, true) => Font {
                    style: iced::font::Style::Italic,
                    ..Font::DEFAULT
                },
                (false, false) => Font::DEFAULT,
            };
            span(s.text.clone())
                .size(font_size)
                .font(font)
                .color(text_color)
        })
        .collect();

    rich_text(rich_spans).into()
}

fn welcome_view<'a>() -> Element<'a, Message> {
    center(
        column![
            text("Shosai (書斎)").size(32),
            text("Open a PDF or EPUB file to start reading").size(16),
            button("Open File").on_press(Message::OpenFile),
        ]
        .spacing(20)
        .align_x(iced::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Title
// ---------------------------------------------------------------------------

pub fn title(state: &State) -> String {
    if let Some(path) = &state.file_path {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        format!("{filename} - Shosai")
    } else {
        "Shosai".to_string()
    }
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

pub fn subscription(_state: &State) -> Subscription<Message> {
    keyboard::listen().map(Message::KeyPressed)
}
