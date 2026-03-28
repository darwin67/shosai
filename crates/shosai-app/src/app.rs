use std::path::PathBuf;

use iced::keyboard;
use iced::widget::{button, center, column, container, image, row, scrollable, text, text_input};
use iced::{Element, Length, Subscription, Task};

use shosai_core::document::{Document, RenderedPage};
use shosai_core::pdf::PdfDoc;
use shosai_core::reading_state::{FileReadingState, ReadingStateStore};

// ---------------------------------------------------------------------------
// Zoom
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    /// Manual zoom percentage (1.0 = 100%).
    Manual(f32),
    /// Fit page width to the window (not yet implemented — falls back to manual).
    FitWidth,
    /// Fit entire page in the window (not yet implemented — falls back to manual).
    FitPage,
}

impl ZoomMode {
    fn scale(&self) -> f32 {
        match self {
            ZoomMode::Manual(s) => *s,
            // TODO: calculate from window/page dimensions
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

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct State {
    /// Path to the currently opened file.
    file_path: Option<PathBuf>,
    /// The loaded PDF document.
    document: Option<PdfDoc>,
    /// Current page index (0-based).
    current_page: usize,
    /// Total number of pages.
    total_pages: usize,
    /// Current zoom mode.
    zoom: ZoomMode,
    /// The rendered page image (cached).
    rendered_page: Option<RenderedPage>,
    /// Text content of the page-number input field.
    page_input: String,
    /// Error message to display.
    error: Option<String>,
    /// Persisted reading state store.
    reading_state: ReadingStateStore,
}

impl Default for ZoomMode {
    fn default() -> Self {
        ZoomMode::Manual(1.0)
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

    // Zoom
    ZoomIn,
    ZoomOut,
    SetZoomFitWidth,
    SetZoomFitPage,

    // Keyboard
    KeyPressed(keyboard::Event),
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

pub fn boot() -> (State, Task<Message>) {
    let reading_state = ReadingStateStore::load().unwrap_or_default();
    let state = State {
        reading_state,
        ..State::default()
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
                        .add_filter("PDF", &["pdf"])
                        .set_title("Open PDF")
                        .pick_file()
                        .await;

                    file.map(|f| f.path().to_path_buf())
                },
                Message::FileSelected,
            );
        }

        Message::FileSelected(Some(path)) => {
            state.error = None;
            match PdfDoc::open(&path) {
                Ok(doc) => {
                    state.total_pages = doc.page_count();
                    state.document = Some(doc);

                    // Restore reading position if we've read this file before.
                    if let Some(saved) = state.reading_state.get(&path) {
                        state.current_page = saved.page.min(state.total_pages.saturating_sub(1));
                        state.zoom = ZoomMode::Manual(saved.zoom);
                    } else {
                        state.current_page = 0;
                        state.zoom = ZoomMode::Manual(1.0);
                    }

                    state.page_input = format!("{}", state.current_page + 1);
                    state.file_path = Some(path);
                    render_current_page(state);
                }
                Err(e) => {
                    state.error = Some(format!("Failed to open PDF: {e}"));
                }
            }
        }

        Message::FileSelected(None) => {
            // User cancelled the dialog.
        }

        Message::NextPage => {
            if state.document.is_some() && state.current_page + 1 < state.total_pages {
                state.current_page += 1;
                state.page_input = format!("{}", state.current_page + 1);
                render_current_page(state);
                save_reading_state(state);
            }
        }

        Message::PrevPage => {
            if state.document.is_some() && state.current_page > 0 {
                state.current_page -= 1;
                state.page_input = format!("{}", state.current_page + 1);
                render_current_page(state);
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
                render_current_page(state);
                save_reading_state(state);
            }
            // Reset input to current page
            state.page_input = format!("{}", state.current_page + 1);
        }

        Message::ZoomIn => {
            let current = state.zoom.scale();
            let new_scale = (current + 0.25).min(5.0);
            state.zoom = ZoomMode::Manual(new_scale);
            render_current_page(state);
            save_reading_state(state);
        }

        Message::ZoomOut => {
            let current = state.zoom.scale();
            let new_scale = (current - 0.25).max(0.25);
            state.zoom = ZoomMode::Manual(new_scale);
            render_current_page(state);
            save_reading_state(state);
        }

        Message::SetZoomFitWidth => {
            state.zoom = ZoomMode::FitWidth;
            render_current_page(state);
            save_reading_state(state);
        }

        Message::SetZoomFitPage => {
            state.zoom = ZoomMode::FitPage;
            render_current_page(state);
            save_reading_state(state);
        }

        Message::KeyPressed(event) => {
            return handle_key_event(event);
        }
    }

    Task::none()
}

fn handle_key_event(event: keyboard::Event) -> Task<Message> {
    if let keyboard::Event::KeyPressed { key, modifiers, .. } = event {
        match key.as_ref() {
            // Navigation
            keyboard::Key::Named(keyboard::key::Named::ArrowRight)
            | keyboard::Key::Named(keyboard::key::Named::PageDown) => {
                return Task::done(Message::NextPage);
            }
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft)
            | keyboard::Key::Named(keyboard::key::Named::PageUp) => {
                return Task::done(Message::PrevPage);
            }

            // Zoom
            keyboard::Key::Character(c) if c == "=" || c == "+" => {
                return Task::done(Message::ZoomIn);
            }
            keyboard::Key::Character("-") => {
                return Task::done(Message::ZoomOut);
            }

            // Open file
            keyboard::Key::Character(c) if c == "o" && modifiers.command() => {
                return Task::done(Message::OpenFile);
            }

            _ => {}
        }
    }
    Task::none()
}

fn render_current_page(state: &mut State) {
    if let Some(doc) = &state.document {
        let scale = state.zoom.scale();
        match doc.render_page(state.current_page, scale) {
            Ok(page) => {
                state.rendered_page = Some(page);
                state.error = None;
            }
            Err(e) => {
                state.error = Some(format!("Failed to render page: {e}"));
                state.rendered_page = None;
            }
        }
    }
}

/// Save the current reading position to disk.
fn save_reading_state(state: &mut State) {
    if let Some(path) = &state.file_path {
        state.reading_state.set(
            path,
            FileReadingState {
                page: state.current_page,
                zoom: state.zoom.scale(),
            },
        );
        // Best-effort save; don't crash the app on write failure.
        if let Err(e) = state.reading_state.save() {
            eprintln!("warning: failed to save reading state: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(state: &State) -> Element<'_, Message> {
    let content = column![toolbar(state), page_view(state)]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill);

    content.into()
}

fn toolbar(state: &State) -> Element<'_, Message> {
    let open_btn = button("Open").on_press(Message::OpenFile);

    let has_doc = state.document.is_some();
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

    let page_input = text_input("Page", &state.page_input)
        .on_input(Message::PageInputChanged)
        .on_submit(Message::GoToPage)
        .width(60);

    let page_label = text(if has_doc {
        format!("/ {}", state.total_pages)
    } else {
        String::new()
    });

    let zoom_out_btn = if has_doc {
        button("-").on_press(Message::ZoomOut)
    } else {
        button("-")
    };

    let zoom_label = text(state.zoom.label()).width(70);

    let zoom_in_btn = if has_doc {
        button("+").on_press(Message::ZoomIn)
    } else {
        button("+")
    };

    let mut fit_width_btn = button("W");
    let mut fit_page_btn = button("P");
    if has_doc {
        fit_width_btn = fit_width_btn.on_press(Message::SetZoomFitWidth);
        fit_page_btn = fit_page_btn.on_press(Message::SetZoomFitPage);
    }

    let toolbar_row = row![
        open_btn,
        prev_btn,
        page_input,
        page_label,
        next_btn,
        zoom_out_btn,
        zoom_label,
        zoom_in_btn,
        fit_width_btn,
        fit_page_btn,
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    container(toolbar_row).padding(8).width(Length::Fill).into()
}

fn page_view(state: &State) -> Element<'_, Message> {
    if let Some(error) = &state.error {
        return center(text(error).size(16))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

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
        // No document loaded — show welcome screen
        center(
            column![
                text("Shosai (書斎)").size(32),
                text("Open a PDF file to start reading").size(16),
                button("Open File").on_press(Message::OpenFile),
            ]
            .spacing(20)
            .align_x(iced::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
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
