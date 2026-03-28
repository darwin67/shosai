mod app;

fn main() -> iced::Result {
    iced::application(app::boot, app::update, app::view)
        .title(app::title)
        .subscription(app::subscription)
        .centered()
        .window_size((900.0, 700.0))
        .run()
}
