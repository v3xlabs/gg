use super::{DiffSource, Message, icons, theme};
use crate::git::read::{ChangeKind, FileChange};
use iced::widget::{Column, Space, button, canvas, container, rich_text, row, span, stack, text};
use iced::{Color, Element, Fill, Font, Length, Point, Rectangle, Renderer, Theme, mouse};
use std::collections::{BTreeMap, HashSet};

/// A commit that touches a vendored directory would otherwise bury everything else under it.
const CHILDREN_SHOWN: usize = 10;

const ROW_HEIGHT: f32 = 24.0;
const NAME_SIZE: f32 = super::BODY;
const NOTE_SIZE: f32 = super::SMALL;
/// Where a parent's stem hangs, measured from that parent's own content edge.
const STEM_INSET: f32 = 7.0;
const STEP: f32 = 13.0;
const TICK_AIR: f32 = 4.0;
const PAD: f32 = 4.0;
const ICON_GAP: f32 = 5.0;
const NOTE_PAD: f32 = 14.0;
const STAGE_ICON: f32 = 12.0;
const BAR_WIDTH: f32 = 32.0;
const BAR_HEIGHT: f32 = 6.0;

const BAND_ALPHA: f32 = 0.14;

const STATUSES: [(ChangeKind, &str); 4] = [
    (ChangeKind::Added, "added"),
    (ChangeKind::Modified, "modified"),
    (ChangeKind::Deleted, "removed"),
    (ChangeKind::Renamed, "renamed"),
];

pub enum Node {
    Directory {
        name: String,
        path: String,
        children: Vec<Node>,
    },
    File {
        name: String,
        path: String,
        kind: ChangeKind,
        additions: usize,
        deletions: usize,
    },
}

/// The connector gutter is one cached canvas beside the rows, so the cache lives with the
/// nodes it was drawn from.
#[derive(Default)]
pub struct Tree {
    nodes: Vec<Node>,
    gutter: canvas::Cache,
}

impl Tree {
    pub fn build(changes: &[FileChange]) -> Self {
        Self {
            nodes: build_level(
                changes
                    .iter()
                    .map(|change| (change.path.as_str(), change))
                    .collect(),
                "",
            ),
            gutter: canvas::Cache::new(),
        }
    }

    pub fn redraw(&self) {
        self.gutter.clear();
    }

    pub fn rows<'a>(&'a self, expanded: &HashSet<String>) -> Vec<Line<'a>> {
        let mut rows = Vec::new();
        flatten(&self.nodes, None, expanded, &mut Vec::new(), &mut rows);
        rows
    }

    pub fn view<'a>(
        &'a self,
        rows: Vec<Line<'a>>,
        source: DiffSource,
        colours: theme::Colours,
        open: Option<&'a str>,
    ) -> Element<'a, Message> {
        let height = rows.len() as f32 * ROW_HEIGHT;
        let entries =
            Column::with_children(rows.iter().map(|line| entry(line, source, colours, open)))
                .width(Fill);

        // Over the rows rather than under them: a lit row paints its whole width, and
        // connectors drawn behind that fill would go once a row was hovered.
        stack![
            entries,
            canvas(Gutter {
                rows,
                cache: &self.gutter,
                colours,
            })
            .width(Fill)
            .height(Length::Fixed(height)),
        ]
        .into()
    }
}

fn build_level<'a>(entries: Vec<(&'a str, &'a FileChange)>, prefix: &str) -> Vec<Node> {
    let mut directories: BTreeMap<&str, Vec<(&str, &FileChange)>> = BTreeMap::new();
    let mut files = Vec::new();

    for (path, change) in entries {
        match path.split_once('/') {
            None => files.push(Node::File {
                name: path.to_owned(),
                path: change.path.clone(),
                kind: change.kind,
                additions: change.additions,
                deletions: change.deletions,
            }),
            Some((head, rest)) => directories.entry(head).or_default().push((rest, change)),
        }
    }

    let mut nodes: Vec<Node> = directories
        .into_iter()
        .map(|(name, entries)| {
            let path = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let children = build_level(entries, &path);

            Node::Directory {
                name: name.to_owned(),
                path,
                children,
            }
        })
        .collect();

    nodes.extend(files);
    nodes
}

/// A row of the rendered tree, with the stems that pass through it. Index `level` says
/// whether the ancestor at that depth still has a sibling below this row.
pub struct Line<'a> {
    depth: usize,
    stems: Vec<bool>,
    entry: Entry<'a>,
}

enum Entry<'a> {
    Directory {
        name: &'a str,
        files: usize,
        truncated: bool,
    },
    File {
        name: &'a str,
        path: &'a str,
        kind: ChangeKind,
        additions: usize,
        deletions: usize,
    },
    More {
        path: &'a str,
        remaining: usize,
    },
}

impl Line<'_> {
    /// A directory carries no status of its own, so its connectors stay on the guide hue
    /// and it gets no band.
    fn status(&self) -> Option<ChangeKind> {
        match &self.entry {
            Entry::File { kind, .. } => Some(*kind),
            Entry::Directory { .. } | Entry::More { .. } => None,
        }
    }
}

pub fn flat_rows(changes: &[FileChange]) -> Vec<Line<'_>> {
    changes
        .iter()
        .map(|change| Line {
            depth: 0,
            stems: vec![false],
            entry: Entry::File {
                name: &change.path,
                path: &change.path,
                kind: change.kind,
                additions: change.additions,
                deletions: change.deletions,
            },
        })
        .collect()
}

fn flatten<'a>(
    nodes: &'a [Node],
    more: Option<(&'a str, usize)>,
    expanded: &HashSet<String>,
    stems: &mut Vec<bool>,
    rows: &mut Vec<Line<'a>>,
) {
    let siblings = nodes.len() + usize::from(more.is_some());

    for (index, node) in nodes.iter().enumerate() {
        stems.push(index + 1 < siblings);
        let depth = stems.len() - 1;

        match node {
            Node::File {
                name,
                path,
                kind,
                additions,
                deletions,
            } => rows.push(Line {
                depth,
                stems: stems.clone(),
                entry: Entry::File {
                    name,
                    path,
                    kind: *kind,
                    additions: *additions,
                    deletions: *deletions,
                },
            }),
            Node::Directory {
                name,
                path,
                children,
            } => {
                let shown = if expanded.contains(path) {
                    children.len()
                } else {
                    CHILDREN_SHOWN.min(children.len())
                };

                rows.push(Line {
                    depth,
                    stems: stems.clone(),
                    entry: Entry::Directory {
                        name,
                        files: count_files(children),
                        truncated: children.len() > shown,
                    },
                });

                let rest =
                    (children.len() > shown).then(|| (path.as_str(), children.len() - shown));

                flatten(&children[..shown], rest, expanded, stems, rows);
            }
        }

        stems.pop();
    }

    if let Some((path, remaining)) = more {
        stems.push(false);
        rows.push(Line {
            depth: stems.len() - 1,
            stems: stems.clone(),
            entry: Entry::More { path, remaining },
        });
        stems.pop();
    }
}

fn count_files(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| match node {
            Node::File { .. } => 1,
            Node::Directory { children, .. } => count_files(children),
        })
        .sum()
}

fn hue(kind: ChangeKind, colours: theme::Colours) -> Color {
    match kind {
        ChangeKind::Added => colours.added,
        ChangeKind::Deleted => colours.deleted,
        ChangeKind::Modified => colours.modified,
        ChangeKind::Renamed => colours.renamed,
    }
}

fn band(colour: Color) -> Color {
    Color {
        a: BAND_ALPHA,
        ..colour
    }
}

fn content_x(depth: usize) -> f32 {
    PAD + depth as f32 * (STEM_INSET + STEP)
}

/// Half a pixel keeps a one pixel stroke on the pixel rather than across two of them.
fn stem_x(level: usize) -> f32 {
    content_x(level - 1) + STEM_INSET + 0.5
}

fn delta(additions: usize, deletions: usize) -> String {
    match (additions, deletions) {
        (0, 0) => String::new(),
        (additions, 0) => format!("+{additions}"),
        (0, deletions) => format!("-{deletions}"),
        (additions, deletions) => format!("+{additions} -{deletions}"),
    }
}

/// Unstaged rows carry the button that stages them; elsewhere the slot is blank, so a file
/// keeps the same shape whichever side of the index it is on.
fn stage_slot<'a>(source: DiffSource, path: &'a str) -> Element<'a, Message> {
    if source != DiffSource::Unstaged {
        return Space::new().into();
    }

    button(icons::sized(icons::Glyph::Plus, STAGE_ICON))
        .on_press(Message::FileStaged(path.to_owned()))
        .style(button::text)
        .padding([0, 3])
        .into()
}

fn entry<'a>(
    line: &Line<'a>,
    source: DiffSource,
    colours: theme::Colours,
    open: Option<&'a str>,
) -> Element<'a, Message> {
    let indent = Space::new().width(Length::Fixed(content_x(line.depth)));

    match &line.entry {
        Entry::File {
            name,
            path,
            kind,
            additions,
            deletions,
        } => {
            let colour = hue(*kind, colours);
            // Only a span can be struck through, so a removed name goes through rich_text.
            let content = row![
                indent,
                icons::file(path),
                Space::new().width(Length::Fixed(ICON_GAP)),
                rich_text![
                    span::<(), _>(*name)
                        .color(colour)
                        .strikethrough(*kind == ChangeKind::Deleted)
                ]
                .size(NAME_SIZE)
                .font(Font::MONOSPACE)
                .width(Fill),
                text(delta(*additions, *deletions))
                    .size(NOTE_SIZE)
                    .font(Font::MONOSPACE)
                    .color(colour),
                stage_slot(source, path),
                Space::new().width(Length::Fixed(NOTE_PAD)),
            ]
            .height(Fill)
            .align_y(iced::Alignment::Center);

            let showing = open == Some(*path);

            button(content)
                .on_press(Message::FileSelected(source, (*path).to_owned()))
                .style(move |theme: &Theme, status| {
                    let palette = theme.extended_palette();
                    let lit = showing
                        || matches!(status, button::Status::Hovered | button::Status::Pressed);

                    button::Style {
                        background: Some(if lit {
                            palette.background.strong.color.into()
                        } else {
                            band(colour).into()
                        }),
                        text_color: palette.background.base.text,
                        ..button::Style::default()
                    }
                })
                .padding(0)
                .width(Fill)
                .height(Length::Fixed(ROW_HEIGHT))
                .into()
        }
        Entry::Directory {
            name,
            files,
            truncated,
        } => container(
            row![
                indent,
                icons::folder(!truncated),
                Space::new().width(Length::Fixed(ICON_GAP)),
                text(format!("{name}/"))
                    .size(NAME_SIZE)
                    .font(Font::MONOSPACE)
                    .color(colours.text_secondary)
                    .width(Fill),
                text(files.to_string())
                    .size(NOTE_SIZE)
                    .font(Font::MONOSPACE)
                    .color(colours.text_faint),
                Space::new().width(Length::Fixed(NOTE_PAD)),
            ]
            .height(Fill)
            .align_y(iced::Alignment::Center),
        )
        .height(Length::Fixed(ROW_HEIGHT))
        .into(),
        Entry::More { path, remaining } => container(
            row![
                indent,
                button(
                    text(format!("{remaining} unshown files"))
                        .size(NOTE_SIZE)
                        .font(Font::MONOSPACE)
                        .color(colours.text_faint)
                )
                .on_press(Message::DirectoryExpanded((*path).to_owned()))
                .style(button::text)
                .padding(0),
            ]
            .height(Fill)
            .align_y(iced::Alignment::Center),
        )
        .height(Length::Fixed(ROW_HEIGHT))
        .into(),
    }
}

/// What the whole selection did, in one line above the tree.
pub fn summary<'a>(
    files: impl Iterator<Item = &'a FileChange>,
    colours: theme::Colours,
) -> Element<'static, Message> {
    let mut counts = [0usize; STATUSES.len()];
    let mut additions = 0;
    let mut deletions = 0;

    for file in files {
        for (slot, (kind, _)) in STATUSES.iter().enumerate() {
            if *kind == file.kind {
                counts[slot] += 1;
            }
        }
        additions += file.additions;
        deletions += file.deletions;
    }

    let mut line = row![].spacing(12).align_y(iced::Alignment::Center);
    for ((kind, label), count) in STATUSES.into_iter().zip(counts) {
        if count > 0 {
            line = line.push(
                text(format!("{count} {label}"))
                    .size(NOTE_SIZE)
                    .font(Font::MONOSPACE)
                    .color(hue(kind, colours)),
            );
        }
    }

    let mut totals = row![].spacing(6).align_y(iced::Alignment::Center);
    if additions > 0 {
        totals = totals.push(total(format!("+{additions}"), colours.added));
    }
    if deletions > 0 {
        totals = totals.push(total(format!("-{deletions}"), colours.deleted));
    }
    if additions + deletions > 0 {
        totals = totals.push(ratio(additions, deletions, colours));
    }

    line.push(Space::new().width(Fill))
        .push(totals)
        .push(Space::new().width(Length::Fixed(NOTE_PAD)))
        .into()
}

fn total(label: String, colour: Color) -> Element<'static, Message> {
    text(label)
        .size(NOTE_SIZE)
        .font(Font::MONOSPACE)
        .color(colour)
        .into()
}

/// A side that changed at all keeps a visible sliver, so a lopsided ratio never reads as all
/// of one colour.
fn ratio(additions: usize, deletions: usize, colours: theme::Colours) -> Element<'static, Message> {
    let green = match (additions, deletions) {
        (0, _) => 0.0,
        (_, 0) => BAR_WIDTH,
        _ => (BAR_WIDTH * additions as f32 / (additions + deletions) as f32)
            .clamp(1.0, BAR_WIDTH - 1.0),
    };

    row![
        segment(green, colours.added),
        segment(BAR_WIDTH - green, colours.deleted)
    ]
    .into()
}

fn segment(width: f32, colour: Color) -> Element<'static, Message> {
    container(
        Space::new()
            .width(Length::Fixed(width))
            .height(Length::Fixed(BAR_HEIGHT)),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(colour.into()),
        ..container::Style::default()
    })
    .into()
}

struct Gutter<'a> {
    rows: Vec<Line<'a>>,
    cache: &'a canvas::Cache,
    colours: theme::Colours,
}

impl canvas::Program<Message> for Gutter<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            for (index, line) in self.rows.iter().enumerate() {
                let top = index as f32 * ROW_HEIGHT;
                let centre = top + ROW_HEIGHT / 2.0;
                let colour = line
                    .status()
                    .map_or(self.colours.guide, |kind| hue(kind, self.colours));

                for level in 1..=line.depth {
                    let x = stem_x(level);

                    if level < line.depth {
                        if line.stems[level] {
                            stroke(
                                frame,
                                Point::new(x, top),
                                Point::new(x, top + ROW_HEIGHT),
                                self.colours.guide,
                            );
                        }
                        continue;
                    }

                    let end = if line.stems[level] {
                        top + ROW_HEIGHT
                    } else {
                        centre
                    };
                    stroke(frame, Point::new(x, top), Point::new(x, end), colour);
                    stroke(
                        frame,
                        Point::new(x, centre),
                        Point::new(content_x(level) - TICK_AIR, centre),
                        colour,
                    );
                }
            }
        });

        vec![geometry]
    }
}

fn stroke(frame: &mut canvas::Frame, from: Point, to: Point, colour: Color) {
    frame.stroke(
        &canvas::Path::line(from, to),
        canvas::Stroke::default().with_color(colour).with_width(1.0),
    );
}
