use super::{App, BODY, Entry, Message, SMALL, TITLE, icons, theme};
use iced::widget::{Column, Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Length, Theme};
use std::path::Path;

const WIDTH: f32 = 620.0;
const ROW_HEIGHT: f32 = 42.0;
const ROWS_SHOWN: f32 = 8.0;
const ICON: f32 = 20.0;

const TOP: f32 = 90.0;

/// It opens on repositories and steps into a list of its own when a command that has one is
/// chosen.
pub enum Mode {
    Repositories,
    /// The theme in force when the list was opened, to go back to if it is closed without
    /// settling on one.
    Themes(theme::Choice),
}

pub enum Row<'a> {
    Repository(Find<'a>),
    Theme(theme::Choice),
    ChangeTheme,
}

/// One that is already listed, or one found under a remembered directory and never added.
pub enum Find<'a> {
    Listed(&'a Entry),
    Found(&'a Path),
}

impl Find<'_> {
    pub fn path(&self) -> &Path {
        match self {
            Self::Listed(entry) => &entry.path,
            Self::Found(path) => path,
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Listed(entry) => entry.name.clone(),
            Self::Found(path) => super::name_of(path),
        }
    }
}

impl Row<'_> {
    fn name(&self) -> String {
        match self {
            Self::Repository(find) => find.name(),
            Self::Theme(choice) => choice.label().to_owned(),
            Self::ChangeTheme => "change theme".to_owned(),
        }
    }

    fn note(&self) -> String {
        match self {
            Self::Repository(find) => find.path().display().to_string(),
            Self::Theme(_) => String::new(),
            Self::ChangeTheme => "pick a theme, previewed as you move".to_owned(),
        }
    }
}

pub struct Launcher {
    pub query: String,
    pub selected: usize,
    pub mode: Mode,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            mode: Mode::Repositories,
        }
    }
}

pub fn id() -> iced::widget::Id {
    iced::widget::Id::new("launcher")
}

/// A name that matches comes before a path that matches, because a name is what people type.
pub fn matches<'a>(app: &'a App, launcher: &Launcher) -> Vec<Row<'a>> {
    let query = launcher.query.trim().to_lowercase();

    let mut all: Vec<Row<'a>> = match launcher.mode {
        Mode::Themes(_) => theme::Choice::ALL.into_iter().map(Row::Theme).collect(),
        Mode::Repositories => app
            .repositories
            .iter()
            .map(|entry| Row::Repository(Find::Listed(entry)))
            .chain(
                app.form
                    .discovered
                    .iter()
                    .map(|path| Row::Repository(Find::Found(path.as_path()))),
            )
            .chain([Row::ChangeTheme])
            .collect(),
    };

    if query.is_empty() {
        return all;
    }

    all.retain(|row| {
        row.name().to_lowercase().contains(&query) || row.note().to_lowercase().contains(&query)
    });
    all.sort_by_key(|row| !row.name().to_lowercase().contains(&query));
    all
}

pub fn view<'a>(app: &'a App, launcher: &'a Launcher) -> Element<'a, Message> {
    let found = matches(app, launcher);
    let selected = launcher.selected.min(found.len().saturating_sub(1));

    let rows = found
        .iter()
        .enumerate()
        .map(|(index, row)| entry(app, row, index, index == selected));

    let list: Element<'_, Message> = if found.is_empty() {
        container(
            text("nothing here by that name")
                .size(BODY)
                .style(text::secondary),
        )
        .padding(12)
        .into()
    } else {
        let shown = (found.len() as f32).min(ROWS_SHOWN);

        scrollable(Column::with_children(rows).spacing(2))
            .height(Length::Fixed(shown * ROW_HEIGHT + 2.0))
            .into()
    };

    let placeholder = match launcher.mode {
        Mode::Repositories => "open a repository, or type a command",
        Mode::Themes(_) => "pick a theme",
    };

    let panel = container(
        column![
            text_input(placeholder, &launcher.query)
                .id(id())
                .on_input(Message::LauncherTyped)
                .on_submit(Message::LauncherChosen(selected))
                .padding([9, 12])
                .size(TITLE),
            list,
            text("enter opens, up and down move, escape closes")
                .size(SMALL)
                .style(text::secondary),
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Length::Fixed(WIDTH))
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();

        container::Style {
            background: Some(palette.background.weak.color.into()),
            border: iced::Border {
                color: palette.background.strong.color,
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: iced::Shadow {
                color: iced::Color::BLACK,
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        }
    });

    column![
        Space::new().height(Length::Fixed(TOP)),
        row![Space::new().width(Fill), panel, Space::new().width(Fill)],
    ]
    .into()
}

fn entry<'a>(app: &'a App, row: &Row<'a>, index: usize, selected: bool) -> Element<'a, Message> {
    let mark: Element<'_, Message> = match row {
        Row::Repository(find) => icons::repository(app.state.icon(find.path()).as_deref(), ICON),
        Row::Theme(choice) => swatch(*choice),
        Row::ChangeTheme => icons::sized(icons::Glyph::Sliders, ICON),
    };

    let note = row.note();
    let mut lines = column![text(row.name()).size(BODY)].spacing(1);
    if !note.is_empty() {
        lines = lines.push(
            text(note)
                .size(SMALL)
                .style(text::secondary)
                .wrapping(text::Wrapping::None),
        );
    }

    let line = row![mark, lines.width(Fill)]
        .spacing(10)
        .align_y(iced::Alignment::Center);

    button(line)
        .on_press(Message::LauncherChosen(index))
        .style(move |theme: &Theme, status| {
            let palette = theme.extended_palette();
            let lit =
                selected || matches!(status, button::Status::Hovered | button::Status::Pressed);

            button::Style {
                background: lit.then(|| palette.background.strong.color.into()),
                text_color: palette.background.base.text,
                border: iced::Border {
                    radius: 5.0.into(),
                    ..iced::Border::default()
                },
                ..button::Style::default()
            }
        })
        .padding([4, 8])
        .width(Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
}

/// Its own ground, with the three hues the interface leans on hardest laid over it.
fn swatch(choice: theme::Choice) -> Element<'static, Message> {
    let colours = theme::colours(choice);
    let bar = |colour: iced::Color| {
        container(Space::new().width(Fill).height(Fill)).style(move |_: &Theme| container::Style {
            background: Some(colour.into()),
            border: iced::Border {
                radius: 1.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
    };

    container(row![bar(colours.focus), bar(colours.added), bar(colours.deleted),].spacing(2))
        .width(Length::Fixed(ICON))
        .height(Length::Fixed(ICON))
        .padding(3)
        .style(move |_: &Theme| container::Style {
            background: Some(colours.background.into()),
            border: iced::Border {
                color: colours.guide,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
