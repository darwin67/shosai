fn main() -> iced::Result {
    iced::application(app::boot, app::update, app::view)
        .title(app::title)
        .subscription(app::subscription)
        .run()
}

mod app;
