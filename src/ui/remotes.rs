use super::{BODY, Message, SMALL, TITLE, icons, theme};
use iced::widget::{Column, Space, button, checkbox, column, container, row, text};
use iced::{Element, Fill, Length, Theme};
use std::collections::HashSet;

const WIDTH: f32 = 420.0;

/// A remote a push would go to, and what it would send there. Worked out when the dialog
/// opens, because a reader deciding where to push is deciding on these numbers.
pub struct Destination {
    pub remote: String,
    /// Commits the remote has no ref for. `None` when git could not count them.
    pub commits: Option<usize>,
    /// False when the remote carries no branch by this name, so the push would create one.
    pub exists: bool,
}

/// Which of a repository's two remembered answers a dialog is about.
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Fetch,
    Push,
}

/// Which remotes a fetch or a push reaches. A push always asks; a fetch asks once there is
/// more than one remote to choose between.
pub enum Dialog {
    Fetch {
        remotes: Vec<String>,
        chosen: HashSet<String>,
    },
    Push {
        branch: String,
        destinations: Vec<Destination>,
        chosen: HashSet<String>,
        upstream: bool,
    },
}

impl Dialog {
    /// Opens on the answer this repository gave last time. With no answer to go back to
    /// every remote is checked, because a fetch writes nothing outside `refs/remotes` and
    /// the wide default costs nothing.
    pub fn fetch(remotes: Vec<String>, before: &[String]) -> Self {
        let chosen = match remembered(&remotes, before) {
            Some(chosen) => chosen,
            None => remotes.iter().cloned().collect(),
        };

        Self::Fetch { remotes, chosen }
    }

    /// The same, except that with no answer to go back to nothing is checked: a push writes
    /// into somebody else's repository, so the first one is named rather than assumed.
    pub fn push(branch: String, destinations: Vec<Destination>, before: &[String]) -> Self {
        let remotes: Vec<String> = destinations
            .iter()
            .map(|destination| destination.remote.clone())
            .collect();

        Self::Push {
            branch,
            destinations,
            chosen: remembered(&remotes, before).unwrap_or_default(),
            upstream: false,
        }
    }

    pub fn toggle(&mut self, remote: &str) {
        let chosen = match self {
            Self::Fetch { chosen, .. } | Self::Push { chosen, .. } => chosen,
        };

        if !chosen.remove(remote) {
            chosen.insert(remote.to_owned());
        }
    }

    /// In the order the remotes are listed in, so the reader's eye and the commands run
    /// agree on which comes first.
    pub fn taken(&self) -> Vec<String> {
        match self {
            Self::Fetch { remotes, chosen } => remotes
                .iter()
                .filter(|remote| chosen.contains(*remote))
                .cloned()
                .collect(),
            Self::Push {
                destinations,
                chosen,
                ..
            } => destinations
                .iter()
                .map(|destination| &destination.remote)
                .filter(|remote| chosen.contains(*remote))
                .cloned()
                .collect(),
        }
    }
}

/// The remembered answer, narrowed to the remotes that are still there. `None` when that
/// leaves nothing, so a repository whose remotes were all renamed opens on its default
/// rather than on an empty dialog with a dead button.
fn remembered(remotes: &[String], before: &[String]) -> Option<HashSet<String>> {
    let chosen: HashSet<String> = remotes
        .iter()
        .filter(|remote| before.contains(remote))
        .cloned()
        .collect();

    (!chosen.is_empty()).then_some(chosen)
}

pub fn view(
    dialog: &Dialog,
    hosts: &std::collections::HashMap<String, String>,
) -> Element<'static, Message> {
    let taken = dialog.taken();

    let (title, note, rows): (String, String, Vec<Element<'static, Message>>) = match dialog {
        Dialog::Fetch { remotes, chosen } => (
            "fetch".to_owned(),
            "new commits and remote branches are read in. nothing local moves.".to_owned(),
            remotes
                .iter()
                .map(|remote| {
                    line(
                        remote,
                        hosts.get(remote).cloned(),
                        None,
                        chosen.contains(remote),
                    )
                })
                .collect(),
        ),
        Dialog::Push {
            branch,
            destinations,
            chosen,
            ..
        } => (
            format!("push {branch}"),
            format!("{branch} is sent to every remote you check here."),
            destinations
                .iter()
                .map(|destination| {
                    line(
                        &destination.remote,
                        hosts.get(&destination.remote).cloned(),
                        Some(sending(destination, branch)),
                        chosen.contains(&destination.remote),
                    )
                })
                .collect(),
        ),
    };

    let mut card = column![
        text(title).size(TITLE),
        text(note).size(SMALL).style(text::secondary),
        Column::with_children(rows).spacing(2),
    ]
    .spacing(10);

    if let Dialog::Push { upstream, .. } = dialog {
        card = card.push(
            checkbox(*upstream)
                .label("track the remote branch as this branch's upstream")
                .on_toggle(|_| Message::UpstreamToggled)
                .size(15)
                .text_size(BODY),
        );
    }

    let confirm = match dialog {
        Dialog::Fetch { .. } => "fetch",
        Dialog::Push { .. } => "push",
    };

    card = card.push(
        row![
            Space::new().width(Fill),
            button(text("cancel").size(BODY))
                .on_press(Message::Dismissed)
                .style(button::text)
                .padding([4, 12]),
            button(text(confirm).size(BODY))
                .on_press_maybe((!taken.is_empty()).then_some(Message::DialogConfirmed))
                .style(button::primary)
                .padding([4, 12]),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    );

    container(
        container(card.width(Length::Fixed(WIDTH)))
            .padding(20)
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
            }),
    )
    .center(Fill)
    .into()
}

/// What the push would do to this remote, in the words the reader needs before checking it.
fn sending(destination: &Destination, branch: &str) -> String {
    let commits = match destination.commits {
        Some(1) => "1 commit".to_owned(),
        Some(commits) => format!("{commits} commits"),
        None => "commits git would not count".to_owned(),
    };

    if destination.exists {
        format!("{commits} onto {}/{branch}", destination.remote)
    } else {
        format!("{commits}, creating {}/{branch}", destination.remote)
    }
}

/// Drawn rather than taken from `checkbox`. The whole row is the button that toggles it, so
/// a `checkbox` here would be one with no `on_toggle`, and iced draws that disabled: an
/// unchecked disabled box has no visible edge in any palette gg ships.
fn mark(checked: bool) -> Element<'static, Message> {
    let inside: Element<'static, Message> = match checked {
        true => icons::sized(icons::Glyph::Check, 11.0),
        false => Space::new().into(),
    };

    container(inside)
        .width(Length::Fixed(15.0))
        .height(Length::Fixed(15.0))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();

            container::Style {
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

fn line(
    remote: &str,
    host: Option<String>,
    sending: Option<String>,
    checked: bool,
) -> Element<'static, Message> {
    let name = remote.to_owned();
    let mut lines = column![text(remote.to_owned()).size(BODY)].spacing(1);

    if let Some(sending) = sending {
        lines = lines.push(text(sending).size(SMALL).style(text::secondary));
    } else if let Some(host) = host {
        lines = lines.push(text(host).size(SMALL).style(text::secondary));
    }

    button(
        row![mark(checked), lines, Space::new().width(Fill)]
            .spacing(8)
            .align_y(iced::Alignment::Center),
    )
    .on_press(Message::RemoteToggled(name))
    .style(theme::row)
    .width(Fill)
    .padding([6, 8])
    .into()
}
