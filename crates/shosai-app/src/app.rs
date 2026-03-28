use iced::widget::{center, column, text};
use iced::{Element, Subscription, Task};

#[derive(Debug, Default)]
pub struct State {}

#[derive(Debug, Clone)]
pub enum Message {}

pub fn boot() -> (State, Task<Message>) {
    (State::default(), Task::none())
}

pub fn update(_state: &mut State, _message: Message) -> Task<Message> {
    Task::none()
}

pub fn view(_state: &State) -> Element<'_, Message> {
    center(
        column![text("Shōsai (書斎)").size(32),]
            .spacing(20)
            .align_x(iced::Center),
    )
    .into()
}

pub fn title(_state: &State) -> String {
    "Shōsai".to_string()
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
