use super::{App, BODY, Message, TITLE, icons, settings};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Fill, Length};

pub const BAR_HEIGHT: f32 = 30.0;
/// A rough advance width for the interface font at [`BODY`]. The bar sets its own widths
/// from this, so nothing has to be measured to line up.
const TITLE_CHARACTER: f32 = 7.4;
const TITLE_PAD: f32 = 11.0;
const DROPDOWN_WIDTH: f32 = 250.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Repository,
    Commit,
    View,
    Help,
}

impl Menu {
    const ALL: [Self; 4] = [Self::Repository, Self::Commit, Self::View, Self::Help];

    fn title(self) -> &'static str {
        match self {
            Self::Repository => "Repository",
            Self::Commit => "Commit",
            Self::View => "View",
            Self::Help => "Help",
        }
    }

    /// Set here rather than measured, so a dropdown can be put under its title exactly.
    fn width(self) -> f32 {
        self.title().chars().count() as f32 * TITLE_CHARACTER + TITLE_PAD * 2.0
    }

    fn offset(self) -> f32 {
        Self::ALL
            .iter()
            .take_while(|menu| **menu != self)
            .map(|menu| menu.width())
            .sum()
    }
}

pub fn bar(open: Option<Menu>) -> Element<'static, Message> {
    let titles = Menu::ALL.map(|menu| {
        let chosen = open == Some(menu);

        button(
            container(text(menu.title()).size(BODY))
                .center_y(Fill)
                .center_x(Fill),
        )
        .on_press(Message::MenuToggled(menu))
        .style(move |theme: &iced::Theme, status| {
            let palette = theme.extended_palette();
            let lit = chosen || matches!(status, button::Status::Hovered | button::Status::Pressed);

            button::Style {
                background: lit.then(|| palette.background.strong.color.into()),
                text_color: palette.background.base.text,
                border: iced::Border {
                    radius: 4.0.into(),
                    ..iced::Border::default()
                },
                ..button::Style::default()
            }
        })
        .width(Length::Fixed(menu.width()))
        .height(Length::Fixed(BAR_HEIGHT - 6.0))
        .padding(0)
        .into()
    });

    container(
        row![
            row(titles).spacing(0),
            Space::new().width(Fill),
            bar_icon(icons::Glyph::Bell, Message::InboxToggled),
            bar_icon(
                icons::Glyph::Sliders,
                Message::SettingsOpened(settings::Category::Appearance),
            ),
        ]
        .padding([0, 3])
        .align_y(iced::Alignment::Center),
    )
    .width(Fill)
    .height(Length::Fixed(BAR_HEIGHT))
    .style(super::theme::surface)
    .into()
}

fn bar_icon(glyph: icons::Glyph, message: Message) -> Element<'static, Message> {
    button(container(icons::icon(glyph)).center_y(Fill))
        .on_press(message)
        .style(button::text)
        .height(Length::Fixed(BAR_HEIGHT - 6.0))
        .padding([0, 10])
        .into()
}

/// Sits under the open dropdown and over everything else, so a press anywhere outside the
/// dropdown closes it.
pub fn dismiss_layer() -> Element<'static, Message> {
    column![
        Space::new().height(Length::Fixed(BAR_HEIGHT)),
        button(Space::new())
            .on_press(Message::Dismissed)
            .style(button::text)
            .padding(0)
            .width(Fill)
            .height(Fill),
    ]
    .into()
}

pub fn dropdown(menu: Menu, app: &App) -> Element<'_, Message> {
    let has_commit = app.selected_commit().is_some();

    let items: Vec<Element<'_, Message>> = match menu {
        Menu::Repository => vec![
            item("Refresh", Some(Message::RefreshRequested)),
            item("Quit", Some(Message::QuitRequested)),
        ],
        Menu::Commit => vec![
            item("Copy hash", has_commit.then_some(Message::HashCopied)),
            item(
                "Copy message",
                has_commit.then_some(Message::CommitMessageCopied),
            ),
        ],
        Menu::View => vec![
            toggle(
                "Remote branches",
                app.show_remote_branches,
                Message::RemoteBranchesToggled,
            ),
            toggle("Tags", app.show_tags, Message::TagsToggled),
        ],
        Menu::Help => vec![item(
            "About gg",
            Some(Message::SettingsOpened(settings::Category::About)),
        )],
    };

    let panel = container(column(items).spacing(1))
        .padding(4)
        .width(Length::Fixed(DROPDOWN_WIDTH))
        .style(container::rounded_box);

    column![
        Space::new().height(Length::Fixed(BAR_HEIGHT)),
        row![Space::new().width(Length::Fixed(menu.offset())), panel],
    ]
    .into()
}

fn item(label: &'static str, message: Option<Message>) -> Element<'static, Message> {
    let control = button(text(label).size(BODY))
        .style(button::text)
        .width(Fill)
        .padding([4, 10]);

    match message {
        Some(message) => control.on_press(message).into(),
        None => control.into(),
    }
}

fn toggle(label: &'static str, on: bool, message: Message) -> Element<'static, Message> {
    let mark = if on { "\u{2713}" } else { " " };

    button(text(format!("{mark}  {label}")).size(BODY))
        .on_press(message)
        .style(button::text)
        .width(Fill)
        .padding([4, 10])
        .into()
}

pub fn inbox() -> Element<'static, Message> {
    let card = column![
        text("Inbox").size(TITLE),
        text("Nothing here yet. Review requests, comments and issue activity will land here once the forge clients exist.")
            .size(BODY)
            .style(text::secondary),
        button(text("Close").size(BODY))
            .on_press(Message::Dismissed)
            .style(button::secondary),
    ];

    container(
        container(card.spacing(14).width(Length::Fixed(380.0)))
            .padding(24)
            .style(container::rounded_box),
    )
    .center(Fill)
    .into()
}
