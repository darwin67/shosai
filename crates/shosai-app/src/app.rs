use std::path::PathBuf;

use iced::keyboard;
use iced::widget::{
    button, center, column, container, image, rich_text, row, scrollable, span, text, text_input,
};
use iced::{Element, Font, Length, Subscription, Task};

use shosai_core::cbz::CbzDoc;
use shosai_core::document::{Document, RenderedPage};
use shosai_core::epub::EpubDoc;
use shosai_core::epub::render::{ContentNode, parse_chapter_xhtml};
use shosai_core::library::{Book, Library};
use shosai_core::pdf::PdfDoc;
use shosai_core::reading_state::{FileReadingState, ReadingStateStore};

// ---------------------------------------------------------------------------
// Open document wrapper
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum OpenDocument {
    Pdf(PdfDoc),
    Epub(EpubDoc),
    Cbz(CbzDoc),
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
// Screens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Library,
    Reader,
}

const LIBRARY_CARDS_PER_ROW_MIN: usize = 2;
const LIBRARY_CARDS_PER_ROW_MAX: usize = 8;
const LIBRARY_CARDS_PER_ROW_DEFAULT: usize = 5;
const LIBRARY_CARDS_PER_ROW_KEY: &str = "library.cards_per_row";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct State {
    screen: Screen,

    // -- Reader state --
    file_path: Option<PathBuf>,
    document: Option<OpenDocument>,
    current_page: usize,
    total_pages: usize,
    zoom: ZoomMode,
    rendered_page: Option<RenderedPage>,
    chapter_content: Vec<ContentNode>,
    page_input: String,
    error: Option<String>,
    font_size: f32,
    line_spacing: f32,
    theme: ReaderTheme,

    // -- Shared --
    reading_state: Option<ReadingStateStore>,
    library: Option<Library>,

    // -- Library state --
    library_books: Vec<Book>,
    library_search: String,
    library_filter: Option<shosai_core::library::BookFormat>,
    library_cards_per_row: usize,
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

    // Links
    LinkClicked(String),

    // Library
    ShowLibrary,
    RefreshLibrary,
    LibraryLoaded(Vec<Book>),
    ImportFile,
    ImportDirectory,
    OpenBook(String), // file_path
    #[allow(dead_code)]
    RemoveBook(i64),
    LibrarySearchChanged(String),
    LibraryFilterChanged(Option<shosai_core::library::BookFormat>),
    LibraryCardsPerRowIncrement,
    LibraryCardsPerRowDecrement,

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

    let library = reading_state
        .as_ref()
        .map(|store| Library::new(store.pool().clone()));

    let library_cards_per_row = reading_state
        .as_ref()
        .and_then(|store| store.get_pref_int(LIBRARY_CARDS_PER_ROW_KEY))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| {
            (*value >= LIBRARY_CARDS_PER_ROW_MIN) && (*value <= LIBRARY_CARDS_PER_ROW_MAX)
        })
        .unwrap_or(LIBRARY_CARDS_PER_ROW_DEFAULT);

    let state = State {
        screen: Screen::Library,

        file_path: None,
        document: None,
        current_page: 0,
        total_pages: 0,
        zoom: ZoomMode::default(),
        rendered_page: None,
        chapter_content: Vec::new(),
        page_input: String::new(),
        error: None,
        font_size: 16.0,
        line_spacing: 1.6,
        theme: ReaderTheme::default(),

        reading_state,
        library,

        library_books: Vec::new(),
        library_search: String::new(),
        library_filter: None,
        library_cards_per_row,
    };
    (state, Task::done(Message::RefreshLibrary))
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
                        .add_filter("Ebooks", &["pdf", "epub", "cbz"])
                        .add_filter("PDF", &["pdf"])
                        .add_filter("EPUB", &["epub"])
                        .add_filter("CBZ", &["cbz"])
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
            if matches!(
                state.document,
                Some(OpenDocument::Pdf(_)) | Some(OpenDocument::Cbz(_))
            ) {
                let new_scale = (state.zoom.scale() + 0.25).min(5.0);
                state.zoom = ZoomMode::Manual(new_scale);
                refresh_content(state);
                save_reading_state(state);
            }
        }

        Message::ZoomOut => {
            if matches!(
                state.document,
                Some(OpenDocument::Pdf(_)) | Some(OpenDocument::Cbz(_))
            ) {
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

        Message::LinkClicked(href) => {
            handle_link_click(state, &href);
        }

        // Library
        Message::ShowLibrary => {
            state.screen = Screen::Library;
            return Task::done(Message::RefreshLibrary);
        }

        Message::RefreshLibrary => {
            if let Some(lib) = state.library.clone() {
                let search = state.library_search.clone();
                let filter = state.library_filter;
                return Task::perform(
                    async move {
                        if !search.is_empty() {
                            lib.search(&search).await.unwrap_or_default()
                        } else if let Some(fmt) = filter {
                            lib.filter_by_format(fmt).await.unwrap_or_default()
                        } else {
                            lib.list_all().await.unwrap_or_default()
                        }
                    },
                    Message::LibraryLoaded,
                );
            }
        }

        Message::LibraryLoaded(books) => {
            state.library_books = books;
        }

        Message::ImportFile => {
            return Task::perform(
                async {
                    let file = rfd::AsyncFileDialog::new()
                        .add_filter("Ebooks", &["pdf", "epub", "cbz"])
                        .set_title("Import to Library")
                        .pick_file()
                        .await;
                    file.map(|f| f.path().to_path_buf())
                },
                |path| {
                    if let Some(p) = path {
                        Message::OpenBook(p.to_string_lossy().to_string())
                    } else {
                        Message::RefreshLibrary // no-op refresh
                    }
                },
            );
        }

        Message::ImportDirectory => {
            if let Some(lib) = state.library.clone() {
                return Task::perform(
                    async move {
                        let dir = rfd::AsyncFileDialog::new()
                            .set_title("Import Directory")
                            .pick_folder()
                            .await;
                        if let Some(d) = dir {
                            let _ = lib.import_directory(d.path()).await;
                        }
                        // Return the updated list.
                        lib.list_all().await.unwrap_or_default()
                    },
                    Message::LibraryLoaded,
                );
            }
        }

        Message::OpenBook(file_path) => {
            let path = PathBuf::from(&file_path);
            // Import to library if not already there.
            if let Some(lib) = state.library.clone() {
                let p = path.clone();
                // Fire-and-forget import.
                tokio::task::spawn(async move {
                    let _ = lib.import_file(&p).await;
                });
            }
            open_file(state, path);
            state.screen = Screen::Reader;
        }

        Message::RemoveBook(id) => {
            if let Some(lib) = state.library.clone() {
                return Task::perform(
                    async move {
                        let _ = lib.remove(id).await;
                        lib.list_all().await.unwrap_or_default()
                    },
                    Message::LibraryLoaded,
                );
            }
        }

        Message::LibrarySearchChanged(query) => {
            state.library_search = query;
            return Task::done(Message::RefreshLibrary);
        }

        Message::LibraryFilterChanged(filter) => {
            state.library_filter = filter;
            return Task::done(Message::RefreshLibrary);
        }

        Message::LibraryCardsPerRowIncrement => {
            if state.library_cards_per_row < LIBRARY_CARDS_PER_ROW_MAX {
                state.library_cards_per_row += 1;
                save_library_cards_per_row(state);
            }
        }

        Message::LibraryCardsPerRowDecrement => {
            if state.library_cards_per_row > LIBRARY_CARDS_PER_ROW_MIN {
                state.library_cards_per_row -= 1;
                save_library_cards_per_row(state);
            }
        }

        Message::KeyPressed(event) => {
            return handle_key_event(state, event);
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
        "cbz" => match CbzDoc::open(&path) {
            Ok(doc) => {
                state.total_pages = doc.page_count();
                state.document = Some(OpenDocument::Cbz(doc));
                Ok(())
            }
            Err(e) => Err(format!("Failed to open CBZ: {e}")),
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
                if matches!(
                    state.document,
                    Some(OpenDocument::Pdf(_)) | Some(OpenDocument::Cbz(_))
                ) {
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

fn handle_key_event(state: &State, event: keyboard::Event) -> Task<Message> {
    if let keyboard::Event::KeyPressed { key, modifiers, .. } = event {
        match key.as_ref() {
            // Escape: go back to library from reader
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                if state.screen == Screen::Reader {
                    return Task::done(Message::ShowLibrary);
                }
            }

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

/// Handle a link click from the EPUB reader.
fn handle_link_click(state: &mut State, href: &str) {
    // External links: open in system browser.
    if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("mailto:") {
        if let Err(e) = open::that(href) {
            eprintln!("warning: failed to open URL: {e}");
        }
        return;
    }

    // Internal EPUB links: navigate to the target chapter.
    if let Some(OpenDocument::Epub(doc)) = &state.document {
        // Split href into path and optional fragment (#anchor).
        let (target_path, _fragment) = match href.split_once('#') {
            Some((path, frag)) => (path, Some(frag)),
            None => (href, None),
        };

        // If the path is empty, it's a same-chapter fragment link — nothing to navigate.
        if target_path.is_empty() {
            return;
        }

        // Find the chapter whose path ends with the target.
        // Links may be relative to the current chapter's directory, so we
        // resolve against the current chapter's base path.
        let current_base = doc
            .chapter(state.current_page)
            .map(|ch| {
                ch.path
                    .rsplit_once('/')
                    .map(|(dir, _)| dir.to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let resolved = if !current_base.is_empty() && !target_path.starts_with('/') {
            format!("{current_base}/{target_path}")
        } else {
            target_path.to_string()
        };

        // Find the chapter index by matching the resolved path.
        if let Some(chapter_idx) = doc.content.chapters.iter().position(|ch| {
            ch.path == resolved || ch.path.ends_with(target_path) || ch.path.ends_with(&resolved)
        }) {
            state.current_page = chapter_idx;
            state.page_input = format!("{}", state.current_page + 1);
            refresh_content(state);
            save_reading_state(state);
        }
    }
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
                state.chapter_content =
                    parse_chapter_xhtml(&chapter.content, base_path, &doc.content.styles);
                state.error = None;
            } else {
                state.chapter_content = Vec::new();
                state.error = Some(format!("Chapter {} not found", state.current_page));
            }
        }
        Some(OpenDocument::Cbz(doc)) => {
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

    if let (Some(lib), Some(path)) = (state.library.clone(), state.file_path.clone()) {
        if state.total_pages > 0 {
            let progress = (state.current_page + 1) as f64 / state.total_pages as f64;
            let progress = progress.min(1.0).max(0.0);
            tokio::task::spawn(async move {
                let _ = lib.update_progress_by_path(&path, progress).await;
            });
        }
    }
}

fn save_library_cards_per_row(state: &State) {
    if let Some(store) = &state.reading_state {
        if let Err(e) = store.set_pref_int(
            LIBRARY_CARDS_PER_ROW_KEY,
            state.library_cards_per_row as i64,
        ) {
            eprintln!("warning: failed to save library layout: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(state: &State) -> Element<'_, Message> {
    match state.screen {
        Screen::Library => library_view(state),
        Screen::Reader => {
            let content = column![toolbar(state), content_view(state)]
                .spacing(0)
                .width(Length::Fill)
                .height(Length::Fill);
            content.into()
        }
    }
}

fn toolbar(state: &State) -> Element<'_, Message> {
    let open_btn = button("Open").on_press(Message::OpenFile);

    let has_doc = state.document.is_some();
    let is_pdf_or_cbz = matches!(
        state.document,
        Some(OpenDocument::Pdf(_)) | Some(OpenDocument::Cbz(_))
    );
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

    let nav_label = if is_epub { "Ch" } else { "Pg" };

    let page_input = text_input(nav_label, &state.page_input)
        .on_input(Message::PageInputChanged)
        .on_submit(Message::GoToPage)
        .width(60);

    let page_label = text(if has_doc {
        format!("/ {}", state.total_pages)
    } else {
        String::new()
    });

    let library_btn = button("Library").on_press(Message::ShowLibrary);

    let mut toolbar_items: Vec<Element<'_, Message>> = vec![
        library_btn.into(),
        open_btn.into(),
        prev_btn.into(),
        page_input.into(),
        page_label.into(),
        next_btn.into(),
    ];

    // PDF: zoom controls
    if is_pdf_or_cbz || !has_doc {
        let zoom_out_btn = if is_pdf_or_cbz {
            button("-").on_press(Message::ZoomOut)
        } else {
            button("-")
        };
        let zoom_label = text(state.zoom.label()).width(70);
        let zoom_in_btn = if is_pdf_or_cbz {
            button("+").on_press(Message::ZoomIn)
        } else {
            button("+")
        };
        let mut fit_w = button("W");
        let mut fit_p = button("P");
        if is_pdf_or_cbz {
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
        Some(OpenDocument::Pdf(_) | OpenDocument::Cbz(_)) => pdf_page_view(state),
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

    let resources = match &state.document {
        Some(OpenDocument::Epub(doc)) => &doc.content.resources,
        _ => &std::collections::HashMap::new(),
    };

    for node in &state.chapter_content {
        content_col = content_col.push(render_content_node(node, font_size, text_color, resources));
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
    resources: &std::collections::HashMap<String, Vec<u8>>,
) -> Element<'a, Message> {
    match node {
        ContentNode::Heading {
            level,
            text: t,
            style,
        } => {
            let base_size = match level {
                1 => font_size * 2.0,
                2 => font_size * 1.6,
                3 => font_size * 1.3,
                4 => font_size * 1.1,
                _ => font_size,
            };
            let size = style
                .font_size_multiplier
                .map(|m| base_size * m)
                .unwrap_or(base_size);
            let align = node_style_to_alignment(style);
            let heading = text(t.clone()).size(size).color(text_color);
            container(heading).width(Length::Fill).align_x(align).into()
        }

        ContentNode::Paragraph(spans, style) => {
            let size = style
                .font_size_multiplier
                .map(|m| font_size * m)
                .unwrap_or(font_size);
            let align = node_style_to_alignment(style);
            let rendered = render_spans(spans, size, text_color);
            let mut c = container(rendered).width(Length::Fill).align_x(align);
            if let Some(margin) = style.margin_left_em {
                c = c.padding(iced::Padding {
                    left: margin * font_size,
                    ..iced::Padding::ZERO
                });
            }
            c.into()
        }

        ContentNode::BlockQuote(children) => {
            let quote_color = iced::Color {
                a: 0.7,
                ..text_color
            };
            let bar_color = iced::Color {
                a: 0.3,
                ..text_color
            };
            let mut col = column![].spacing(8);
            for child in children {
                col = col.push(render_content_node(
                    child,
                    font_size,
                    quote_color,
                    resources,
                ));
            }
            row![
                container(column![])
                    .width(Length::Fixed(3.0))
                    .height(Length::Fill)
                    .style(move |_theme| container::Style {
                        background: Some(iced::Background::Color(bar_color)),
                        ..Default::default()
                    }),
                container(col).padding([4, 12]),
            ]
            .spacing(0)
            .width(Length::Fill)
            .into()
        }

        ContentNode::UnorderedList(items) => {
            let mut col = column![].spacing(4);
            for item_spans in items {
                let bullet_text = "  \u{2022} ".to_string();
                let mut all_spans = vec![shosai_core::epub::render::TextSpan {
                    text: bullet_text,
                    bold: false,
                    italic: false,
                    monospace: false,
                    link: None,
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
                    monospace: false,
                    link: None,
                }];
                all_spans.extend(item_spans.iter().cloned());
                col = col.push(render_spans(&all_spans, font_size, text_color));
            }
            col.into()
        }

        ContentNode::CodeBlock { code, language } => {
            render_code_block(code, language.as_deref(), font_size, text_color)
        }

        ContentNode::InlineCode(code_text) => {
            // Render as monospace span inline
            let mono_font = Font {
                family: iced::font::Family::Monospace,
                ..Font::DEFAULT
            };
            text(code_text.clone())
                .size(font_size * 0.9)
                .font(mono_font)
                .color(text_color)
                .into()
        }

        ContentNode::Image { src, alt } => {
            render_epub_image(src, alt, font_size, text_color, resources)
        }

        ContentNode::HorizontalRule => text("───────────────────")
            .size(font_size)
            .color(text_color)
            .into(),
    }
}

/// Render an EPUB image from the resource map, falling back to alt text.
fn render_epub_image<'a>(
    src: &str,
    alt: &str,
    font_size: f32,
    text_color: iced::Color,
    resources: &std::collections::HashMap<String, Vec<u8>>,
) -> Element<'a, Message> {
    if let Some(data) = resources.get(src) {
        // Try to decode the image and display as RGBA via iced::widget::image.
        if let Ok(img) = ::image::load_from_memory(data) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let handle = image::Handle::from_rgba(w, h, rgba.into_raw());

            return container(
                image(handle)
                    .content_fit(iced::ContentFit::ScaleDown)
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .center_x(Length::Fill)
            .into();
        }
    }

    // Fallback: show alt text placeholder.
    text(format!("[Image: {alt}]"))
        .size(font_size)
        .color(text_color)
        .into()
}

/// Render a code block with optional syntax highlighting.
fn render_code_block<'a>(
    code: &str,
    language: Option<&str>,
    font_size: f32,
    text_color: iced::Color,
) -> Element<'a, Message> {
    use shosai_core::highlight;

    let mono_font = Font {
        family: iced::font::Family::Monospace,
        ..Font::DEFAULT
    };
    let code_size = font_size * 0.85;

    // Try syntax highlighting.
    let theme_name = highlight::syntect_theme_for_reader(
        text_color.r > 0.5, // dark theme = light text
    );

    if let Some(highlighted_lines) = highlight::highlight_code(code, language, theme_name) {
        let bg_color = highlight::theme_background(theme_name)
            .map(|(r, g, b)| iced::Color::from_rgb8(r, g, b))
            .unwrap_or(iced::Color::from_rgb(0.15, 0.15, 0.18));

        let mut lines_col = column![].spacing(0);

        for line_spans in &highlighted_lines {
            let rich_spans: Vec<iced::widget::text::Span<'_, Message>> = line_spans
                .iter()
                .map(|hs| {
                    let (r, g, b) = hs.color;
                    let mut font = mono_font;
                    if hs.bold {
                        font.weight = iced::font::Weight::Bold;
                    }
                    if hs.italic {
                        font.style = iced::font::Style::Italic;
                    }
                    span(hs.text.clone())
                        .size(code_size)
                        .font(font)
                        .color(iced::Color::from_rgb8(r, g, b))
                })
                .collect();

            lines_col = lines_col.push(rich_text(rich_spans));
        }

        return container(lines_col)
            .padding(12)
            .width(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(bg_color)),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into();
    }

    // Fallback: plain monospace text.
    container(
        text(code.to_string())
            .size(code_size)
            .font(mono_font)
            .color(text_color),
    )
    .padding(12)
    .width(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgb(
            0.15, 0.15, 0.18,
        ))),
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

fn node_style_to_alignment(
    style: &shosai_core::epub::render::NodeStyle,
) -> iced::alignment::Horizontal {
    match style.text_align {
        Some(shosai_core::epub::style::TextAlignment::Center) => {
            iced::alignment::Horizontal::Center
        }
        Some(shosai_core::epub::style::TextAlignment::Right) => iced::alignment::Horizontal::Right,
        _ => iced::alignment::Horizontal::Left,
    }
}

const LINK_COLOR: iced::Color = iced::Color {
    r: 0.2,
    g: 0.5,
    b: 0.9,
    a: 1.0,
};

fn render_spans<'a>(
    spans: &[shosai_core::epub::render::TextSpan],
    font_size: f32,
    text_color: iced::Color,
) -> Element<'a, Message> {
    let rich_spans: Vec<iced::widget::text::Span<'a, String>> = spans
        .iter()
        .map(|s| {
            let is_link = s.link.is_some();
            let family = if s.monospace {
                iced::font::Family::Monospace
            } else {
                iced::font::Family::default()
            };
            let font = Font {
                family,
                weight: if s.bold {
                    iced::font::Weight::Bold
                } else {
                    iced::font::Weight::Normal
                },
                style: if s.italic {
                    iced::font::Style::Italic
                } else {
                    iced::font::Style::Normal
                },
                ..Font::DEFAULT
            };
            let color = if is_link { LINK_COLOR } else { text_color };
            let mut sp = span(s.text.clone()).size(font_size).font(font).color(color);
            if is_link {
                sp = sp.underline(true);
            }
            if let Some(href) = &s.link {
                sp = sp.link(href.clone());
            }
            sp
        })
        .collect();

    rich_text(rich_spans)
        .on_link_click(Message::LinkClicked)
        .into()
}

fn library_view(state: &State) -> Element<'_, Message> {
    let search_input = text_input("Search by title or author...", &state.library_search)
        .on_input(Message::LibrarySearchChanged)
        .width(300);

    let all_btn = button("All").on_press(Message::LibraryFilterChanged(None));
    let pdf_btn = button("PDF").on_press(Message::LibraryFilterChanged(Some(
        shosai_core::library::BookFormat::Pdf,
    )));
    let epub_btn = button("EPUB").on_press(Message::LibraryFilterChanged(Some(
        shosai_core::library::BookFormat::Epub,
    )));
    let cbz_btn = button("CBZ").on_press(Message::LibraryFilterChanged(Some(
        shosai_core::library::BookFormat::Cbz,
    )));
    let import_btn = button("Import File").on_press(Message::ImportFile);
    let import_dir_btn = button("Import Folder").on_press(Message::ImportDirectory);

    let mut per_row_down = button("-");
    if state.library_cards_per_row > LIBRARY_CARDS_PER_ROW_MIN {
        per_row_down = per_row_down.on_press(Message::LibraryCardsPerRowDecrement);
    }
    let mut per_row_up = button("+");
    if state.library_cards_per_row < LIBRARY_CARDS_PER_ROW_MAX {
        per_row_up = per_row_up.on_press(Message::LibraryCardsPerRowIncrement);
    }
    let per_row_label = text(format!("Per row: {}", state.library_cards_per_row)).size(14);

    let toolbar = row![
        text("Library").size(24),
        search_input,
        all_btn,
        pdf_btn,
        epub_btn,
        cbz_btn,
        per_row_down,
        per_row_label,
        per_row_up,
        import_btn,
        import_dir_btn,
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let header = container(toolbar).padding(12).width(Length::Fill);

    if state.library_books.is_empty() {
        let empty_msg = if state.library_search.is_empty() && state.library_filter.is_none() {
            "No books in library. Import files to get started."
        } else {
            "No books match your search or filter."
        };

        return column![
            header,
            center(
                column![
                    text("Shosai (書斎)").size(32),
                    text(empty_msg).size(16),
                    button("Import File").on_press(Message::ImportFile),
                    button("Import Folder").on_press(Message::ImportDirectory),
                ]
                .spacing(16)
                .align_x(iced::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .into();
    }

    // Grid of book covers (wrap flow).
    let cover_width = 150.0_f32;
    let cover_height = 200.0_f32;

    // Build grid as rows of cards.
    let cards_per_row = state.library_cards_per_row;
    let mut grid = column![].spacing(12);
    let mut current_row: Vec<Element<'_, Message>> = Vec::new();

    for book in &state.library_books {
        current_row.push(render_book_card(book, cover_width, cover_height));
        if current_row.len() >= cards_per_row {
            grid = grid.push(row(std::mem::take(&mut current_row)).spacing(12));
        }
    }
    if !current_row.is_empty() {
        grid = grid.push(row(current_row).spacing(12));
    }

    let content = scrollable(container(grid).padding(12).width(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill);

    column![header, content].into()
}

fn render_book_card<'a>(book: &Book, width: f32, height: f32) -> Element<'a, Message> {
    let file_path = book.file_path.clone();

    // Cover image or placeholder.
    let cover: Element<'_, Message> = if let Some(ref cover_data) = book.cover {
        if let Ok(img) = ::image::load_from_memory(cover_data) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let handle = image::Handle::from_rgba(w, h, rgba.into_raw());
            image(handle)
                .width(Length::Fixed(width))
                .height(Length::Fixed(height))
                .content_fit(iced::ContentFit::Cover)
                .into()
        } else {
            cover_placeholder(width, height, &book.title)
        }
    } else {
        cover_placeholder(width, height, &book.title)
    };

    let title_text = text(book.title.clone())
        .size(12)
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph);

    let format_label = text(book.format.as_str().to_uppercase()).size(10);

    let card = column![cover, title_text, format_label]
        .spacing(4)
        .width(Length::Fixed(width));

    button(card)
        .on_press(Message::OpenBook(file_path))
        .padding(4)
        .width(Length::Fixed(width + 8.0))
        .into()
}

fn cover_placeholder<'a>(width: f32, height: f32, title: &str) -> Element<'a, Message> {
    let label = text(title.chars().take(20).collect::<String>())
        .size(14)
        .color(iced::Color::WHITE);

    container(center(label))
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.3, 0.3, 0.4,
            ))),
            border: iced::Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn welcome_view<'a>() -> Element<'a, Message> {
    center(
        column![
            text("Shosai (書斎)").size(32),
            text("Open a PDF, EPUB, or CBZ file to start reading").size(16),
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
