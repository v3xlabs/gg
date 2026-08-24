use super::{App, BODY, Message, SMALL, TITLE, icons, theme};
use iced::widget::text::Wrapping;
use iced::widget::{
    Column, Row, Space, button, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Element, Fill, Length};
use std::path::{Path, PathBuf};

const SIDEBAR_WIDTH: f32 = 200.0;
const ROW_ICON: f32 = 24.0;
const CHOICE_ICON: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Appearance,
    Repositories,
    About,
}

impl Category {
    const ALL: [Self; 3] = [Self::Appearance, Self::Repositories, Self::About];

    fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Repositories => "Repositories",
            Self::About => "About",
        }
    }
}

/// What the page holds while it is open, and nothing that outlives it.
#[derive(Debug, Default)]
pub struct Form {
    pub repository: String,
    pub repository_error: Option<String>,
    pub repository_completions: Vec<PathBuf>,
    pub directory: String,
    pub directory_error: Option<String>,
    pub directory_completions: Vec<PathBuf>,
    /// The row the pointer is over, so the whole bar can light up under it.
    pub hovered: Option<PathBuf>,
    pub discovered: Vec<PathBuf>,
    pub picker: Option<Picker>,
}

#[derive(Debug)]
pub struct Picker {
    pub repository: PathBuf,
    pub choices: Vec<PathBuf>,
}

pub fn view(app: &App, category: Category) -> Element<'_, Message> {
    let body = match category {
        Category::Appearance => appearance(app),
        Category::Repositories => repositories(app),
        Category::About => about(),
    };

    row![
        sidebar(category),
        container(scrollable(body.spacing(18).padding([0, 14])).height(Fill))
            .padding(20)
            .width(Fill)
            .height(Fill),
    ]
    .height(Fill)
    .into()
}

fn sidebar(current: Category) -> Element<'static, Message> {
    let categories = Category::ALL.map(|category| {
        button(text(category.title()).size(BODY))
            .on_press(Message::SettingsOpened(category))
            .style(if category == current {
                button::secondary
            } else {
                button::text
            })
            .width(Fill)
            .padding([5, 10])
            .into()
    });

    container(
        column![
            button(text("\u{2190}  Back to the repository").size(BODY))
                .on_press(Message::SettingsClosed)
                .style(button::secondary)
                .width(Fill)
                .padding([5, 10]),
            text("settings").size(BODY).style(text::secondary),
            column(categories).spacing(2),
        ]
        .spacing(10),
    )
    .padding(12)
    .width(Length::Fixed(SIDEBAR_WIDTH))
    .height(Fill)
    .style(theme::surface)
    .into()
}

/// A theme is picked by looking at it, so each one is offered as the colours it would paint
/// the window with.
fn theme_card(choice: theme::Choice, current: theme::Choice) -> Element<'static, Message> {
    let colours = theme::colours(choice);
    let swatch = |colour: iced::Color| {
        container(
            Space::new()
                .width(Length::Fixed(9.0))
                .height(Length::Fixed(9.0)),
        )
        .style(move |_: &iced::Theme| container::Style {
            background: Some(colour.into()),
            border: iced::Border {
                radius: 2.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
    };

    let sample = row![
        swatch(colours.focus),
        swatch(colours.added),
        swatch(colours.deleted),
        swatch(colours.renamed),
    ]
    .spacing(3);

    button(
        column![
            text(choice.label())
                .size(BODY)
                .color(colours.text)
                .wrapping(iced::widget::text::Wrapping::None),
            sample,
        ]
        .spacing(6),
    )
    .on_press(Message::ThemeChanged(choice))
    .style(move |_: &iced::Theme, status| {
        let chosen = choice == current;
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);

        button::Style {
            background: Some(colours.background.into()),
            text_color: colours.text,
            border: iced::Border {
                color: if chosen {
                    colours.focus
                } else if hovered {
                    colours.text_faint
                } else {
                    colours.guide
                },
                width: if chosen { 2.0 } else { 1.0 },
                radius: 5.0.into(),
            },
            ..button::Style::default()
        }
    })
    .padding([7, 10])
    .width(Length::Fixed(148.0))
    .into()
}

fn appearance(app: &App) -> Column<'_, Message> {
    let mut rows: Vec<Element<'static, Message>> = Vec::new();
    let mut line = Row::new().spacing(8);
    for (index, choice) in theme::Choice::ALL.into_iter().enumerate() {
        line = line.push(theme_card(choice, app.theme_choice));
        if index % 3 == 2 {
            rows.push(std::mem::replace(&mut line, Row::new().spacing(8)).into());
        }
    }
    rows.push(line.into());

    column![
        heading("Appearance"),
        column![
            text("Theme").size(BODY),
            Column::with_children(rows).spacing(8),
            note("System follows the desktop portal's colour scheme and accent colour."),
        ]
        .spacing(8),
        column![
            text("Interface scale").size(BODY),
            row![
                button(text("\u{2212}").size(BODY))
                    .on_press(Message::ZoomOut)
                    .style(button::secondary)
                    .padding([3, 12]),
                text(super::percentage(app.scale)).size(BODY),
                button(text("+").size(BODY))
                    .on_press(Message::ZoomIn)
                    .style(button::secondary)
                    .padding([3, 12]),
                button(text("reset").size(BODY))
                    .on_press(Message::ZoomReset)
                    .style(button::text)
                    .padding([3, 12]),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
            note("Ctrl and + zoom in, Ctrl and - zoom out, Ctrl and = go back to 100%."),
        ]
        .spacing(6),
    ]
}

fn about() -> Column<'static, Message> {
    column![
        heading("About"),
        text("gg").size(TITLE),
        text(env!("CARGO_PKG_VERSION")).size(BODY),
        text(env!("CARGO_PKG_DESCRIPTION")).size(BODY),
        note(env!("CARGO_PKG_REPOSITORY")),
    ]
}

fn repositories(app: &App) -> Column<'_, Message> {
    let known = app.repositories.iter().map(|entry| {
        let removable = app.state.paths.contains(&entry.path);
        let icon = app.state.icon(&entry.path);
        let open = app.form.picker.as_ref().map(|picker| &picker.repository) == Some(&entry.path);

        known_repository(entry, icon.as_deref(), removable, open)
    });

    let mut adding = vec![
        text("Add a repository").size(BODY).into(),
        field(
            "/path/to/a/repository",
            &app.form.repository,
            &app.form.repository_completions,
            Message::RepositoryInputChanged,
            Message::RepositoryAdded,
        ),
    ];
    if let Some(error) = &app.form.repository_error {
        adding.push(problem(error));
    }

    let mut page = vec![
        heading("Repositories"),
        Column::with_children(known).spacing(4).into(),
    ];
    if let Some(picker) = &app.form.picker {
        page.push(icon_choices(picker));
    }
    page.push(Column::with_children(adding).spacing(6).into());
    page.push(directories(app).into());

    Column::with_children(page)
}

fn known_repository<'a>(
    entry: &'a super::Entry,
    icon: Option<&Path>,
    removable: bool,
    picking: bool,
) -> Element<'a, Message> {
    let trailing: Element<'_, Message> = if removable {
        button(text("Remove").size(BODY))
            .on_press(Message::RepositoryRemoved(entry.path.clone()))
            .style(button::danger)
            .padding([3, 10])
            .into()
    } else {
        note("from the configuration file")
    };

    container(
        row![
            button(icons::repository(icon, ROW_ICON))
                .on_press(Message::IconPickerToggled(entry.path.clone()))
                .style(if picking {
                    button::secondary
                } else {
                    button::text
                })
                .padding(4),
            column![
                text(entry.name.as_str()).size(BODY),
                text(entry.path.display().to_string())
                    .size(SMALL)
                    .style(text::secondary),
            ]
            .spacing(2)
            .width(Fill),
            trailing,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    )
    .padding([4, 8])
    .style(container::rounded_box)
    .into()
}

fn icon_choices(picker: &Picker) -> Element<'_, Message> {
    let repository = &picker.repository;

    let mut choices: Vec<Element<'_, Message>> = vec![
        button(text("browse\u{2026}").size(BODY))
            .on_press(Message::IconBrowsed(repository.clone()))
            .style(button::primary)
            .padding([4, 10])
            .into(),
        button(text("no icon").size(BODY))
            .on_press(Message::IconCleared(repository.clone()))
            .style(button::secondary)
            .padding([4, 10])
            .into(),
    ];

    for file in &picker.choices {
        choices.push(
            button(
                row![
                    icons::repository(Some(&repository.join(file)), CHOICE_ICON),
                    text(file.display().to_string()).size(SMALL),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::IconChosen(repository.clone(), file.clone()))
            .style(button::secondary)
            .padding([4, 8])
            .into(),
        );
    }

    let title = if picker.choices.is_empty() {
        note("nothing in this repository looks like an icon, so browse for one")
    } else {
        text(format!("Icon for {}", repository.display()))
            .size(BODY)
            .into()
    };

    container(
        column![
            title,
            Row::with_children(choices)
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .wrap(),
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Fill)
    .style(container::bordered_box)
    .into()
}

/// The whole bar lights up under the pointer; the button on it is what acts.
fn bar<'a>(
    label: String,
    action: Element<'a, Message>,
    hovered: bool,
    over: Message,
) -> Element<'a, Message> {
    let line = container(
        row![
            text(label).size(BODY).width(Fill).wrapping(Wrapping::None),
            action,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    )
    .padding([2, 8])
    .width(Fill)
    .style(move |theme: &iced::Theme| {
        let palette = theme.extended_palette();

        container::Style {
            background: hovered.then(|| palette.background.weak.color.into()),
            border: iced::Border {
                radius: 4.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        }
    })
    .clip(true);

    mouse_area(line)
        .on_enter(over.clone())
        .on_exit(Message::RowLeft)
        .into()
}

fn directories(app: &App) -> Column<'_, Message> {
    let hovered = |path: &PathBuf| app.form.hovered.as_ref() == Some(path);

    let remembered = app.state.directories.iter().map(|directory| {
        bar(
            directory.display().to_string(),
            button(text("remove").size(BODY))
                .on_press(Message::DirectoryRemoved(directory.clone()))
                .style(button::danger)
                .padding([3, 10])
                .into(),
            hovered(directory),
            Message::RowHovered(directory.clone()),
        )
    });

    let suggestions = app.form.discovered.iter().map(|path| {
        bar(
            path.display().to_string(),
            button(text("add").size(BODY))
                .on_press(Message::RepositoryPathAdded(path.clone()))
                .style(button::primary)
                .padding([3, 10])
                .into(),
            hovered(path),
            Message::RowHovered(path.clone()),
        )
    });

    let mut section: Vec<Element<'_, Message>> = vec![
        text("Repository directories").size(BODY).into(),
        note(
            "A directory is scanned for suggestions only; nothing under it is listed until you add it.",
        ),
        Column::with_children(remembered).spacing(2).into(),
        field(
            "/path/to/a/directory",
            &app.form.directory,
            &app.form.directory_completions,
            Message::DirectoryInputChanged,
            Message::DirectoryAdded,
        ),
    ];

    if let Some(error) = &app.form.directory_error {
        section.push(problem(error));
    }
    if !app.state.directories.is_empty() {
        section.push(if app.form.discovered.is_empty() {
            note("every repository under these directories is already listed")
        } else {
            Column::with_children(suggestions).spacing(2).into()
        });
    }

    Column::with_children(section).spacing(6)
}

/// The directories a path could still become are listed under it, so a path can be walked
/// into rather than typed out.
fn field<'a>(
    placeholder: &'a str,
    typed: &'a str,
    completions: &'a [PathBuf],
    changed: impl Fn(String) -> Message + 'a,
    submitted: Message,
) -> Element<'a, Message> {
    let changed = std::rc::Rc::new(changed);
    let refill = changed.clone();

    let mut column = column![
        row![
            text_input(placeholder, typed)
                .on_input(move |value| changed(value))
                .on_submit(submitted.clone())
                .padding([5, 8])
                .size(BODY),
            button(text("add").size(BODY))
                .on_press(submitted)
                .style(button::primary)
                .padding([5, 14]),
        ]
        .spacing(6)
    ]
    .spacing(4);

    if !completions.is_empty() {
        let choices = completions.iter().map(|path| {
            let value = path.display().to_string();
            let refill = refill.clone();

            button(text(value.clone()).size(BODY).wrapping(Wrapping::None))
                .on_press(refill(format!("{value}/")))
                .style(button::text)
                .padding([2, 8])
                .width(Fill)
                .into()
        });

        column = column.push(
            container(Column::with_children(choices).spacing(1))
                .padding(4)
                .style(container::bordered_box),
        );
    }

    column.into()
}

fn heading(title: &'static str) -> Element<'static, Message> {
    text(title).size(TITLE).into()
}

fn note(line: &str) -> Element<'_, Message> {
    text(line).size(SMALL).style(text::secondary).into()
}

fn problem(line: &str) -> Element<'_, Message> {
    text(line).size(BODY).style(text::danger).into()
}
