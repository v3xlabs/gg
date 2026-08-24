mod changes;
mod diff;
mod graph;
mod icons;
mod launcher;
mod menu;
mod settings;
pub mod theme;
mod when;

use crate::avatar;
use crate::config;
use crate::git::read::ChangeKind;
use crate::git::{command, history, read};
use graph::{LANE_WIDTH, ROW_HEIGHT, lane_colour};
use iced::keyboard;
use iced::widget::image;
use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{
    Column, Space, button, canvas, column, container, mouse_area, row, scrollable, stack, text,
    text_editor, text_input, tooltip,
};
use iced::{Color, Element, Fill, Font, Length, Size, Subscription, Task, Theme, mouse, window};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

const HISTORY_LIMIT: usize = 2000;
const WINDOW_SIZE: Size = Size::new(1500.0, 950.0);
const FOOTER_HEIGHT: f32 = 24.0;
const SIDEBAR_ICON: f32 = 20.0;
const SIDEBAR_ROW_HEIGHT: f32 = 48.0;
const SIDEBAR_GLYPH: f32 = 13.0;
const SIDEBAR_SHARE: f32 = 0.16;
const RAIL_WIDTH: f32 = 58.0;
const SIDEBAR_WIDTH: f32 = 240.0;
/// Under this there is no room for a name beside an icon, however the sidebar came to be
/// that wide: dragged there by hand, or left there by a window that shrank.
const RAIL_THRESHOLD: f32 = 150.0;
const FACE_SIZE: f32 = 38.0;

const LABEL_ICON: f32 = 13.0;
/// The inset a heading and the cells under it share.
const CELL_PAD: u16 = 10;
const COLUMN_MIN_WIDTH: f32 = 24.0;
const CONNECTOR_WIDTH: f32 = 1.6;

/// A repository like nixpkgs has enough lanes to fill the window, so the graph is capped
/// until it is dragged wider.
const GRAPH_MAX_WIDTH: f32 = 220.0;
/// Enough for the heading over it, so the column cannot be dragged down to something with
/// no name on it.
const GRAPH_MIN_WIDTH: f32 = 58.0;
const DIVIDER_WIDTH: f32 = 5.0;
/// The gap is what can be grabbed; this line is all there is to see.
const DIVIDER_LINE: f32 = 1.0;
const HISTORY_HEADER_HEIGHT: f32 = 30.0;
const TOOLBAR_HEIGHT: f32 = 46.0;
const TOOLBAR_ICON: f32 = 26.0;
const TOOLBAR_GLYPH: f32 = 16.0;

/// Rows built beyond each edge of the viewport, so a fast scroll does not show a gap
/// before the next frame catches up.
const OVERSCAN: usize = 8;

/// Enough to pick from without the list becoming the page.
const COMPLETIONS_SHOWN: usize = 8;
const CONTEXT_WIDTH: f32 = 220.0;

/// Without this, a click that moves by a pixel reorders the list under the reader.
const HOLD_TO_DRAG: std::time::Duration = std::time::Duration::from_millis(180);

/// Three sizes and no others; everything on screen is one of them.
pub const SMALL: f32 = 11.0;
pub const BODY: f32 = 13.0;
pub const TITLE: f32 = 16.0;

/// The description opens at about three lines and grows to about ten before it scrolls.
const DESCRIPTION_MIN: f32 = 62.0;
const GRIP_HEIGHT: f32 = 7.0;
const DESCRIPTION_MAX: f32 = 180.0;

const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

const SCALE_STEP: f32 = 0.1;
const SCALE_MIN: f32 = 0.5;
const SCALE_MAX: f32 = 2.5;

pub fn run(
    config: config::Config,
    state: config::State,
    repositories: Vec<PathBuf>,
    opened: PathBuf,
) -> iced::Result {
    let start = move || App::start(config.clone(), state.clone(), &repositories, opened.clone());

    iced::application(start, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .scale_factor(App::scale_factor)
        .subscription(App::subscription)
        .window(window::Settings {
            size: WINDOW_SIZE,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "gg".to_owned(),
                ..Default::default()
            },
            ..window::Settings::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Repositories,
    History,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileView {
    Tree,
    Path,
}

/// Which side of the repository a clicked file should be diffed against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSource {
    Commit,
    Unstaged,
    Staged,
}

struct FileDiff {
    path: String,
    source: DiffSource,
    body: Result<diff::Body, String>,
    /// A diff does not change while it is open, so the marks beside the scrollbar are drawn
    /// once.
    overview: canvas::Cache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    WorkingTree,
    Commit(usize),
    /// Two commits chosen together and everything between them. `anchor` is the one that
    /// was already selected; `other` is the one ctrl or shift was held for.
    Range {
        anchor: usize,
        other: usize,
    },
}

impl Selection {
    fn holds(self, index: usize) -> bool {
        match self {
            Self::WorkingTree => false,
            Self::Commit(chosen) => chosen == index,
            Self::Range { anchor, other } => {
                (anchor.min(other)..=anchor.max(other)).contains(&index)
            }
        }
    }
}
struct Label {
    kind: LabelKind,
    /// The whole name, `origin/main` and all.
    name: String,
    /// What the chip shows: a remote branch without the remote in front of it.
    short: String,
    host: Option<String>,
    pull: Option<u32>,
    /// Whether HEAD is at this name, which is what the tick in front of it says.
    head: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelKind {
    Head,
    Branch,
    Remote,
    Tag,
    Stash,
}

impl LabelKind {
    fn glyph(self) -> icons::Glyph {
        match self {
            Self::Head => icons::Glyph::Head,
            Self::Branch => icons::Glyph::Branch,
            Self::Remote => icons::Glyph::Remote,
            Self::Tag => icons::Glyph::Tag,
            Self::Stash => icons::Glyph::Stash,
        }
    }
}

/// One line of the history list. Every slot is exactly ROW_HEIGHT tall, which is what lets
/// the pane work out which ones are on screen from the scroll offset alone.
enum Slot {
    WorkingTree,
    Separator(String),
    Commit(usize),
}

/// Below this a reader can see the whole history at once, so dating it adds noise rather
/// than orientation.
const SEPARATED_ABOVE: usize = 40;

/// The optional columns of the history; the title is always there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Columns {
    pub labels: bool,
    pub author: bool,
    pub when: bool,
    pub hash: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Self {
            labels: true,
            author: true,
            when: true,
            hash: true,
        }
    }
}

/// The widths of the fixed columns. The commit message takes whatever is left over, so it
/// has no width of its own.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Widths {
    pub labels: f32,
    /// `None` until the reader drags it, which is what lets a crowded graph be capped
    /// until they ask for more of it.
    pub graph: Option<f32>,
    pub author: f32,
    pub when: f32,
    pub hash: f32,
}

impl Default for Widths {
    fn default() -> Self {
        Self {
            labels: 190.0,
            graph: None,
            author: 92.0,
            when: 94.0,
            hash: 76.0,
        }
    }
}

/// A boundary, named for the column it resizes. The two left of the message resize what is
/// left of them; the three right of it resize what is right of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    Labels,
    Graph,
    Author,
    When,
    Hash,
}

/// A press carries no position, so the first move is what anchors the drag and every move
/// after that is measured from there.
#[derive(Debug, Clone, Copy)]
struct Drag {
    divider: Divider,
    anchor: Option<f32>,
    start: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryColumn {
    Labels,
    Author,
    When,
    Hash,
}

pub struct App {
    path: PathBuf,
    repository: Result<Repository, String>,
    repositories: Vec<Entry>,
    state: config::State,
    panes: pane_grid::State<Pane>,
    /// The split the sidebar is on, which is what collapsing it moves.
    sidebar_split: Option<pane_grid::Split>,
    /// A pane grid works in shares, so the width the sidebar comes out as depends on the
    /// window as well.
    sidebar_share: f32,
    window_width: f32,
    labels: HashMap<gix::ObjectId, Vec<Label>>,
    /// The same commits as `labels`; the graph needs nothing else about them.
    labelled: HashSet<gix::ObjectId>,
    selected: Option<Selection>,
    changed_files: Result<Vec<read::FileChange>, String>,
    change_tree: changes::Tree,
    expanded: HashSet<String>,
    diff: Option<FileDiff>,
    diff_mode: diff::Mode,
    file_view: FileView,
    menu: Option<menu::Menu>,
    page: Page,
    form: settings::Form,
    inbox: bool,
    theme_choice: theme::Choice,
    /// Both kept beside the choice: resolving one asks the desktop portal what it is set
    /// to, which is a round trip nobody should pay for on every frame.
    palette: Theme,
    colours: theme::Colours,
    scale: f32,
    show_remote_branches: bool,
    show_tags: bool,
    widths: Widths,
    drag: Option<Drag>,
    columns: Columns,
    columns_open: bool,
    history_offset: f32,
    history_height: f32,
    /// The pictures that have arrived, and the fingerprints already asked about, so a
    /// missing picture is not asked for again on every scroll.
    pictures: HashMap<[u8; 32], Faces>,
    asked: HashSet<[u8; 32]>,
    commit_message: String,
    commit_description: text_editor::Content,
    commit_error: Option<String>,
    /// The last reading of each repository opened, so going back to one is immediate.
    readings: HashMap<PathBuf, Reading>,
    /// The git directory's timestamps when it was last read.
    watched: Option<Vec<std::time::SystemTime>>,
    description_height: f32,
    /// Set while the grip is being pulled: the pointer anchor, and the height it started at.
    description_drag: Option<(f32, f32)>,
    /// The sidebar sections folded away, by the name of the section.
    folded: HashSet<String>,
    /// The divider the pointer is over, which is the only time one of them shows.
    hovered_divider: Option<Divider>,
    sidebar: Sidebar,
    /// Set while a repository is being dragged up or down the sidebar.
    reorder: Option<Reorder>,
    /// A press carries no modifiers of its own, so the held ones are kept here.
    modifiers: keyboard::Modifiers,
    context: Option<Context>,
    launcher: Option<launcher::Launcher>,
    /// True while a repository is being read on another thread.
    reading: bool,
}

/// Worked out before anything is changed, so the launcher's borrow of the window is over
/// by then.
enum Chosen {
    Repository(PathBuf),
    Theme(theme::Choice),
    ChangeTheme,
}

struct Faces {
    square: image::Handle,
    round: image::Handle,
}

impl Faces {
    fn read(path: &Path) -> Option<Self> {
        let faces = avatar::faces(path)?;

        Some(Self {
            square: image::Handle::from_rgba(faces.size, faces.size, faces.square),
            round: image::Handle::from_rgba(faces.size, faces.size, faces.round),
        })
    }
}

/// What a context menu was opened on, and all the menu is decided from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Commit(gix::ObjectId),
    Reference {
        kind: LabelKind,
        name: String,
        target: gix::ObjectId,
    },
    Repository(PathBuf),
}

/// A context menu, and the corner of the window it was opened at.
struct Context {
    at: (f32, f32),
    target: Target,
}

/// Kept outside the interface state: a message for every mouse move would rebuild the
/// window on every mouse move, and only a context menu needs this.
static POINTER: std::sync::Mutex<(f32, f32)> = std::sync::Mutex::new((0.0, 0.0));

fn pointer() -> (f32, f32) {
    POINTER.lock().map(|at| *at).unwrap_or_default()
}

/// A `None` message is an item that is drawn but not wired up yet.
fn context_items(target: &Target) -> Vec<(&'static str, Option<Message>)> {
    match target {
        Target::Commit(id) => vec![
            ("copy the sha", Some(Message::TextCopied(id.to_string()))),
            ("copy the message", Some(Message::CommitMessageCopied)),
            ("check out", None),
            ("revert", None),
            ("cherry-pick", None),
        ],
        Target::Reference { kind, name, target } => {
            let mut items = vec![
                ("copy the name", Some(Message::TextCopied(name.clone()))),
                (
                    "copy the sha",
                    Some(Message::TextCopied(target.to_string())),
                ),
            ];

            match kind {
                LabelKind::Tag => items.push(("delete the tag", None)),
                LabelKind::Stash => {
                    items.push(("apply", None));
                    items.push(("drop", None));
                }
                _ => {
                    items.push(("check out", None));
                    items.push(("merge into the current branch", None));
                    items.push(("rename", None));
                    items.push(("delete", None));
                }
            }

            items
        }
        Target::Repository(path) => vec![
            ("open", Some(Message::RepositoryOpened(path.clone()))),
            (
                "copy the path",
                Some(Message::TextCopied(path.display().to_string())),
            ),
            (
                "remove from the list",
                Some(Message::RepositoryRemoved(path.clone())),
            ),
        ],
    }
}

fn context_menu(context: &Context) -> Element<'_, Message> {
    let items = context_items(&context.target)
        .into_iter()
        .map(|(label, message)| {
            let enabled = message.is_some();

            button(text(label).size(BODY))
                .on_press_maybe(message)
                .style(move |theme: &Theme, status| {
                    let palette = theme.extended_palette();
                    let lit = matches!(status, button::Status::Hovered | button::Status::Pressed);

                    button::Style {
                        background: lit.then(|| palette.background.strong.color.into()),
                        text_color: if enabled {
                            palette.background.base.text
                        } else {
                            palette.background.weak.text
                        },
                        border: iced::Border {
                            radius: 4.0.into(),
                            ..iced::Border::default()
                        },
                        ..button::Style::default()
                    }
                })
                .padding([4, 10])
                .width(Fill)
                .into()
        });

    let panel = container(Column::with_children(items).spacing(1))
        .padding(4)
        .width(Length::Fixed(CONTEXT_WIDTH))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();

            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color::BLACK,
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 16.0,
                },
                ..container::Style::default()
            }
        });

    let (x, y) = context.at;

    column![
        Space::new().height(Length::Fixed(y)),
        row![Space::new().width(Length::Fixed(x)), panel],
    ]
    .into()
}

/// A repository being carried to a new place in the sidebar. `moved` tells a drag from a
/// plain click: a press that never left the row it started on opens that repository.
struct Reorder {
    path: PathBuf,
    /// A drag only starts once the pointer has been held for [`HOLD_TO_DRAG`].
    pressed: std::time::Instant,
    moved: bool,
}

impl Reorder {
    fn carrying(&self, path: &Path) -> bool {
        self.moved && self.path == path
    }

    fn armed(&self) -> bool {
        self.moved || self.pressed.elapsed() >= HOLD_TO_DRAG
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Repository,
    Settings(settings::Category),
}

/// Only what a sidebar row shows: reading a whole history for every one of these would make
/// the window take seconds to appear.
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    branch: Result<String, String>,
}

impl Entry {
    fn read(path: PathBuf) -> Self {
        Self {
            name: name_of(&path),
            branch: branch_of(&path),
            path,
        }
    }
}

/// Anything git is told to ignore is left out: a checked-in logo is the project's, a built
/// one is a copy.
fn unignored(repository: &Path) -> Vec<PathBuf> {
    let found = config::icon_files(repository);
    let ignored = command::Git::in_work_dir(repository).ignored(&found);

    found
        .into_iter()
        .filter(|file| !ignored.contains(file))
        .collect()
}

/// The directories a half-typed path could still become. Only ever one level deep, so this
/// is a single directory listing however long the path is.
fn completions(typed: &str) -> Vec<PathBuf> {
    let typed = config::expanded(typed.trim());
    let (directory, prefix) = match typed.to_string_lossy().ends_with('/') {
        true => (typed.clone(), String::new()),
        false => (
            typed
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            typed
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        ),
    };

    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };

    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            name.starts_with(&prefix) && (!name.starts_with('.') || prefix.starts_with('.'))
        })
        .map(|entry| entry.path())
        .collect();

    found.sort();
    found.truncate(COMPLETIONS_SHOWN);
    found
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn branch_of(path: &Path) -> Result<String, String> {
    let handle = gix::open(path).map_err(|error| error.to_string())?;
    let head = read::head(&handle).map_err(|error| error.to_string())?;

    Ok(head.name.unwrap_or_else(|| "detached".to_owned()))
}

struct Repository {
    handle: gix::Repository,
    name: String,
    path: PathBuf,
    head: read::Head,
    references: read::References,
    status: read::Status,
    unstaged_tree: changes::Tree,
    staged_tree: changes::Tree,
    rows: Vec<history::GraphRow>,
    commits: Vec<read::CommitSummary>,
    lanes: usize,
    graph_cache: canvas::Cache,
    slots: Vec<Slot>,
    /// The top of each commit row, which the graph needs because separators take slots.
    tops: Vec<f32>,
    authors: Vec<graph::Author>,
    /// Remote name to the host its URL points at, so "origin/main" can show its forge.
    remotes: HashMap<String, String>,
    warnings: Vec<Warning>,
    /// The commits the stash reflog points at, so the graph draws a stash as one thing
    /// rather than as the three commits git keeps it in.
    stashes: HashSet<gix::ObjectId>,
    worktrees: Vec<read::Worktree>,
}

/// A `Repository` holds drawing caches that cannot leave the interface thread, so this is
/// what the reading thread hands back and the caches are made when it lands.
#[derive(Clone)]
pub struct Reading {
    name: String,
    path: PathBuf,
    head: read::Head,
    references: read::References,
    status: read::Status,
    rows: Vec<history::GraphRow>,
    commits: Vec<read::CommitSummary>,
    lanes: usize,
    authors: Vec<graph::Author>,
    remotes: HashMap<String, String>,
    warnings: Vec<Warning>,
    worktrees: Vec<read::Worktree>,
}

impl Reading {
    fn of(path: &Path) -> Result<Self, String> {
        Self::read(path).map_err(|error| error.to_string())
    }

    fn read(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let handle = gix::discover(path)?;
        let path = read::workdir(&handle)?;
        let name = name_of(&path);

        let rows = history::walk(&handle, HISTORY_LIMIT)?;
        let mut commits = Vec::with_capacity(rows.len());
        for row in &rows {
            commits.push(read::summary(&handle, row.commit)?);
        }

        let lanes = rows
            .iter()
            .map(|row| {
                row.lane
                    .max(row.through.iter().copied().max().unwrap_or(0))
                    .max(row.outgoing.iter().copied().max().unwrap_or(0))
            })
            .max()
            .map_or(1, |lane| lane + 1);

        let signing = read::signing(&handle);
        let mut warnings = Vec::new();
        if let Err(error) = command::Git::in_work_dir(&path).version() {
            warnings.push(Warning::GitUnusable(error.to_string()));
        }
        if signing.signs_commits
            && matches!(signing.format, read::SigningFormat::Ssh)
            && signing.ssh_agent.is_none()
        {
            warnings.push(Warning::SigningWithoutAgent(
                signing.key.unwrap_or_else(|| "that is not set".to_owned()),
            ));
        }

        Ok(Self {
            head: read::head(&handle)?,
            references: read::references(&handle)?,
            status: read::status(&handle)?,
            authors: commits.iter().map(author_of).collect(),
            name,
            path,
            rows,
            commits,
            lanes,
            remotes: remote_hosts(&handle),
            warnings,
            worktrees: read::worktrees(&handle),
        })
    }
}

impl Repository {
    fn is_dirty(&self) -> bool {
        !self.status.staged.is_empty() || !self.status.unstaged.is_empty()
    }

    /// The handle is opened again rather than carried over, which keeps a [`Reading`] to
    /// plain data.
    fn of(reading: Reading) -> Result<Self, String> {
        let handle = gix::open(&reading.path).map_err(|error| error.to_string())?;
        let dirty = !reading.status.staged.is_empty() || !reading.status.unstaged.is_empty();
        let (slots, tops) = build_slots(&reading.commits, dirty);

        Ok(Self {
            stashes: reading
                .references
                .stashes
                .iter()
                .map(|stash| stash.target)
                .collect(),
            unstaged_tree: changes::Tree::build(&reading.status.unstaged),
            staged_tree: changes::Tree::build(&reading.status.staged),
            graph_cache: canvas::Cache::new(),
            slots,
            tops,
            handle,
            name: reading.name,
            path: reading.path,
            head: reading.head,
            references: reading.references,
            status: reading.status,
            rows: reading.rows,
            commits: reading.commits,
            lanes: reading.lanes,
            authors: reading.authors,
            remotes: reading.remotes,
            warnings: reading.warnings,
            worktrees: reading.worktrees,
        })
    }
}

/// Checked when a repository opens rather than on commit: a missing ssh agent makes git
/// hang on a pinentry that has no terminal to draw on.
#[derive(Clone)]
enum Warning {
    GitUnusable(String),
    SigningWithoutAgent(String),
}

#[derive(Clone)]
pub enum Message {
    Checked,
    Refocused,
    RepositoryGrabbed(PathBuf),
    ContextOpened(Target),
    ModifiersChanged(keyboard::Modifiers),
    TextCopied(String),
    RepositoryOpened(PathBuf),
    SidebarClosed,
    SidebarToggled,
    WindowResized(f32),
    SectionFolded(String),
    DescriptionGrabbed,
    DescriptionDragged(f32),
    DescriptionDropped,
    BranchSelected(gix::ObjectId),
    RepositoryDropped,
    RepositoryRead(PathBuf, Result<Box<Reading>, String>),
    CommitSelected(usize),
    WorkingTreeSelected,
    FileSelected(DiffSource, String),
    DiffModeChanged(diff::Mode),
    DiffClosed,
    PaneResized(pane_grid::ResizeEvent),
    DirectoryExpanded(String),
    FileViewChanged(FileView),
    MenuToggled(menu::Menu),
    Dismissed,
    RefreshRequested,
    QuitRequested,
    HashCopied,
    CommitMessageCopied,
    RemoteBranchesToggled,
    TagsToggled,
    ThemeChanged(theme::Choice),
    SettingsOpened(settings::Category),
    SettingsClosed,
    InboxToggled,
    RepositoryInputChanged(String),
    RepositoryAdded,
    RepositoryPathAdded(PathBuf),
    RepositoryRemoved(PathBuf),
    DirectoryInputChanged(String),
    DirectoryAdded,
    DirectoryRemoved(PathBuf),
    LauncherToggled,
    LauncherTyped(String),
    LauncherMoved(i32),
    LauncherChosen(usize),
    RowHovered(PathBuf),
    RowLeft,
    IconPickerToggled(PathBuf),
    IconBrowsed(PathBuf),
    IconFilePicked(PathBuf, Option<PathBuf>),
    IconChosen(PathBuf, PathBuf),
    IconCleared(PathBuf),
    ZoomIn,
    ZoomOut,
    ZoomReset,
    DividerPressed(Divider),
    DividerHovered(Divider),
    DividerLeft,
    DividerMoved(f32),
    DividerReleased,
    ColumnMenuToggled,
    ColumnToggled(HistoryColumn),
    HistoryScrolled(f32, f32),
    FileStaged(String),
    EverythingStaged,
    CommitMessageChanged(String),
    CommitDescriptionActed(text_editor::Action),
    CommitRequested,
    FaceFetched([u8; 32], Option<PathBuf>),
}

impl App {
    fn start(
        config: config::Config,
        state: config::State,
        repositories: &[PathBuf],
        path: PathBuf,
    ) -> (Self, Task<Message>) {
        let panes = default_panes();
        let mut app = Self {
            repository: Err(String::new()),
            path,
            repositories: repositories.iter().cloned().map(Entry::read).collect(),
            panes: panes.0,
            sidebar_split: panes.1,
            labels: HashMap::new(),
            labelled: HashSet::new(),
            selected: None,
            changed_files: Ok(Vec::new()),
            change_tree: changes::Tree::default(),
            expanded: HashSet::new(),
            diff: None,
            diff_mode: diff::Mode::Highlighted,
            file_view: FileView::Tree,
            menu: None,
            page: Page::Repository,
            form: settings::Form::default(),
            inbox: false,
            palette: theme::resolve(state.theme.unwrap_or(config.theme)),
            colours: theme::colours(state.theme.unwrap_or(config.theme)),
            theme_choice: state.theme.unwrap_or(config.theme),
            scale: state.scale.unwrap_or(1.0).clamp(SCALE_MIN, SCALE_MAX),
            show_remote_branches: true,
            show_tags: true,
            widths: state.widths,
            drag: None,
            columns: state.columns,
            columns_open: false,
            commit_message: String::new(),
            commit_description: text_editor::Content::new(),
            commit_error: None,
            readings: HashMap::new(),
            watched: None,
            description_height: DESCRIPTION_MIN,
            description_drag: None,
            folded: HashSet::new(),
            hovered_divider: None,
            sidebar: Sidebar::Repositories,
            sidebar_share: SIDEBAR_SHARE,
            window_width: WINDOW_SIZE.width,
            reorder: None,
            modifiers: keyboard::Modifiers::empty(),
            context: None,
            launcher: None,
            reading: false,
            pictures: HashMap::new(),
            asked: HashSet::new(),
            history_offset: 0.0,
            // The real viewport arrives with the first scroll; too few rows until then
            // would leave the pane half empty.
            history_height: WINDOW_SIZE.height,
            state,
        };
        app.rescan_directories();
        let reading = app.read_repository();

        (app, reading)
    }

    fn scale_factor(&self) -> f32 {
        self.scale
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            // Every event, captured or not: a text field swallows escape to unfocus itself,
            // and the launcher still has to hear it.
            iced::event::listen_with(|event, _status, _window| match event {
                // Recorded rather than reported: see POINTER.
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    if let Ok(mut pointer) = POINTER.lock() {
                        *pointer = (position.x, position.y);
                    }
                    None
                }
                iced::Event::Keyboard(keyboard::Event::ModifiersChanged(held)) => {
                    Some(Message::ModifiersChanged(held))
                }
                iced::Event::Keyboard(event) => shortcut(event),
                iced::Event::Window(window::Event::Focused) => Some(Message::Refocused),
                iced::Event::Window(window::Event::Resized(size)) => {
                    Some(Message::WindowResized(size.width))
                }
                _ => None,
            }),
            // A repository is changed by other things than this window.
            ticker(),
        ])
    }

    /// Anything git does touches one of these, so four `stat` calls answer whether a re-read
    /// is worth it however large the repository is.
    fn git_fingerprint(&self) -> Option<Vec<std::time::SystemTime>> {
        let repository = self.repository.as_ref().ok()?;
        let git_dir = repository.handle.path();

        Some(
            ["HEAD", "index", "refs", "packed-refs"]
                .iter()
                .filter_map(|name| std::fs::metadata(git_dir.join(name)).ok())
                .filter_map(|data| data.modified().ok())
                .collect(),
        )
    }

    fn set_scale(&mut self, scale: f32) {
        // A tenth at a time drifts off the round numbers the footer shows without this.
        let wanted = ((scale * 10.0).round() / 10.0).clamp(SCALE_MIN, SCALE_MAX);
        if wanted == self.scale {
            return;
        }

        self.scale = wanted;
        self.state.scale = Some(wanted);
        self.save_state();
    }

    fn open_repository(&mut self, path: PathBuf) -> Task<Message> {
        if path == self.path {
            return Task::none();
        }

        self.path = path;
        self.diff = None;
        self.selected = None;
        self.changed_files = Ok(Vec::new());
        self.change_tree = changes::Tree::default();
        self.expanded.clear();
        self.history_offset = 0.0;

        // Put back up as it was and read again behind that, so going back to one is
        // immediate and still shows what is there now.
        match self.readings.get(&self.path).cloned() {
            Some(reading) => self.show(reading),
            None => {
                self.repository = Err(String::new());
                self.labels.clear();
                self.labelled.clear();
            }
        }

        self.state.last_opened = Some(self.path.clone());
        self.save_state();

        Task::batch([self.request_faces(), self.read_repository()])
    }

    /// Reading a repository the size of nixpkgs is felt, so it happens off the interface
    /// thread and whatever is on screen stays there until it lands.
    fn read_repository(&mut self) -> Task<Message> {
        self.reading = true;
        let path = self.path.clone();

        Task::perform(
            async move { (path.clone(), Reading::of(&path).map(Box::new)) },
            |(path, reading)| Message::RepositoryRead(path, reading),
        )
    }

    /// The selected commit keeps its place across a re-read: a reading landing behind the
    /// reader must not move them.
    fn show(&mut self, reading: Reading) {
        let selected = self.selected_commit().map(|commit| commit.id);
        let working_tree = self.selected == Some(Selection::WorkingTree);

        self.repository = Repository::of(reading);
        self.rebuild_labels();

        let Ok(repository) = &self.repository else {
            return;
        };

        let again =
            selected.and_then(|id| repository.commits.iter().position(|commit| commit.id == id));

        match (again, working_tree && repository.is_dirty()) {
            (Some(index), _) => self.select_commit(index),
            (None, true) => self.select_working_tree(),
            (None, false) => self.select_first(),
        }
    }

    /// The list is reordered as the drag happens, so the rows part around what is being
    /// carried rather than snapping into place when it is let go.
    fn carry_to(&mut self, over: &Path) {
        let Some(reorder) = &mut self.reorder else {
            return;
        };
        if reorder.path == over || !reorder.armed() {
            return;
        }

        let (Some(from), Some(to)) = (
            self.repositories
                .iter()
                .position(|entry| entry.path == reorder.path),
            self.repositories
                .iter()
                .position(|entry| entry.path == over),
        ) else {
            return;
        };

        reorder.moved = true;
        let carried = self.repositories.remove(from);
        self.repositories.insert(to, carried);
    }

    /// Only the repositories the state file owns can be kept in order; one the configuration
    /// lists is placed by that file.
    fn remember_order(&mut self) {
        self.state.paths = self
            .repositories
            .iter()
            .map(|entry| entry.path.clone())
            .filter(|path| self.state.paths.contains(path))
            .collect();

        self.save_state();
    }

    /// Puts a theme on without settling on it, which is what makes the launcher's list a
    /// preview.
    fn wear_theme(&mut self, choice: theme::Choice) {
        if self.theme_choice == choice {
            return;
        }

        self.theme_choice = choice;
        self.palette = theme::resolve(choice);
        self.colours = theme::colours(choice);
        self.redraw_trees();
    }

    /// The syntax colours are resolved when a file is read, so an open diff only follows
    /// the theme if it is read again.
    fn reread_diff(&mut self) -> Task<Message> {
        let Some(file) = &self.diff else {
            return Task::none();
        };

        let (source, path) = (file.source, file.path.clone());
        self.select_file(source, path)
    }

    /// No state says whether the sidebar is collapsed, because its width already does.
    fn sidebar_width(&self) -> f32 {
        self.sidebar_share * self.window_width
    }

    /// Too narrow for a name beside an icon, so it shows the icon alone.
    fn rail(&self) -> bool {
        self.sidebar_width() < RAIL_THRESHOLD
    }

    /// A pane grid works in shares of the window rather than in widths.
    fn set_sidebar_width(&mut self, width: f32) {
        let Some(split) = self.sidebar_split else {
            return;
        };

        self.sidebar_share = (width / self.window_width.max(1.0)).clamp(0.02, 0.5);
        self.panes.resize(split, self.sidebar_share);
    }

    fn save_state(&self) {
        if let Err(error) = self.state.save() {
            eprintln!("gg: {error}");
        }
    }

    /// Only the state file is ever written, so a repository the configuration lists cannot
    /// be removed here and one added here is remembered there.
    fn add_repository(&mut self, path: PathBuf) {
        if self.repositories.iter().any(|entry| entry.path == path) {
            self.form.repository_error = Some(format!("{} is already listed", path.display()));
            return;
        }

        self.repositories.push(Entry::read(path.clone()));
        self.state.paths.push(path);
        self.save_state();
        self.rescan_directories();
    }

    fn remove_repository(&mut self, path: &Path) {
        self.state.paths.retain(|known| known != path);
        self.state.icons.retain(|icon| icon.repository != path);
        if self.state.last_opened.as_deref() == Some(path) {
            self.state.last_opened = None;
        }
        self.repositories.retain(|entry| entry.path != path);
        if self
            .form
            .picker
            .as_ref()
            .is_some_and(|open| open.repository == path)
        {
            self.form.picker = None;
        }
        self.save_state();
        self.rescan_directories();
    }

    /// A remembered directory offers what it holds, so the suggestions drop whatever is
    /// already listed.
    fn rescan_directories(&mut self) {
        let known: HashSet<PathBuf> = self
            .repositories
            .iter()
            .map(|entry| entry.path.clone())
            .collect();

        let mut seen = HashSet::new();
        let found = self
            .state
            .directories
            .iter()
            .flat_map(|directory| config::scan(directory))
            .filter(|path| !known.contains(path) && seen.insert(path.clone()))
            .collect();

        self.form.discovered = found;
    }

    fn typed_repository(&self) -> Result<PathBuf, String> {
        let typed = self.form.repository.trim();
        if typed.is_empty() {
            return Err("type the path of a repository first".to_owned());
        }

        let handle = gix::open(typed)
            .map_err(|error| format!("{typed} is not a repository gg can open: {error}"))?;

        read::workdir(&handle).map_err(|error| format!("{typed}: {error}"))
    }

    fn title(&self) -> String {
        match &self.repository {
            Ok(repository) => format!("{} - gg", repository.name),
            Err(_) => "gg".to_owned(),
        }
    }

    fn theme(&self) -> Theme {
        self.palette.clone()
    }

    fn select_first(&mut self) {
        let Ok(repository) = &self.repository else {
            return;
        };

        if repository.is_dirty() {
            self.select_working_tree();
        } else {
            self.select_commit(0);
        }
    }

    fn select_working_tree(&mut self) {
        self.selected = Some(Selection::WorkingTree);
        self.expanded.clear();
        self.diff = None;
        self.changed_files = Ok(Vec::new());
        self.change_tree = changes::Tree::default();
    }

    /// Extends the selection to a run of commits while ctrl or shift is held.
    fn select_commit(&mut self, index: usize) {
        let extend = self.modifiers.command() || self.modifiers.shift();
        let selected = match (extend, self.selected) {
            (true, Some(Selection::Commit(anchor))) => Selection::Range {
                anchor,
                other: index,
            },
            (true, Some(Selection::Range { anchor, .. })) => Selection::Range {
                anchor,
                other: index,
            },
            _ => Selection::Commit(index),
        };

        self.show_selection(selected);
    }

    fn show_selection(&mut self, selected: Selection) {
        let Ok(repository) = &self.repository else {
            return;
        };
        let Some((from, id)) = self.comparison_of(selected) else {
            return;
        };

        self.selected = Some(selected);
        self.expanded.clear();
        self.diff = None;
        self.changed_files =
            read::between(&repository.handle, from, id).map_err(|error| error.to_string());
        self.change_tree = match &self.changed_files {
            Ok(files) => changes::Tree::build(files),
            Err(_) => changes::Tree::default(),
        };
    }

    /// What to diff from and what to diff to: one commit against its own parent, a run of
    /// them from the older end to the newer.
    fn comparison_of(&self, selected: Selection) -> Option<(Option<gix::ObjectId>, gix::ObjectId)> {
        let repository = self.repository.as_ref().ok()?;
        let commit = |index: usize| repository.commits.get(index);

        match selected {
            Selection::WorkingTree => None,
            Selection::Commit(index) => {
                let commit = commit(index)?;
                Some((commit.parent, commit.id))
            }
            Selection::Range { anchor, other } => {
                // The history runs newest first, so the larger index is the older commit.
                let (newer, older) = (anchor.min(other), anchor.max(other));
                Some((Some(commit(older)?.id), commit(newer)?.id))
            }
        }
    }

    fn comparison(&self) -> Option<(Option<gix::ObjectId>, gix::ObjectId)> {
        self.comparison_of(self.selected?)
    }

    fn select_file(&mut self, source: DiffSource, path: String) -> Task<Message> {
        let dark = self.colours.dark;
        let body = self
            .read_diff(source, &path)
            .map(|(raw, whole_file)| diff::prepare(raw, &whole_file, &path, dark));

        let start = body.as_ref().ok().and_then(diff::first_change);
        self.diff = Some(FileDiff {
            path,
            source,
            body,
            overview: canvas::Cache::new(),
        });

        // Opened at its first change rather than at its top.
        match start {
            None => Task::none(),
            Some(y) => iced::advanced::widget::operate(
                iced::advanced::widget::operation::scrollable::snap_to(
                    diff_scroll(),
                    iced::widget::scrollable::RelativeOffset {
                        x: Some(0.0),
                        y: Some(y),
                    },
                ),
            ),
        }
    }

    /// The tight-context diff and the whole-file one, which the two view modes need.
    fn read_diff(&self, source: DiffSource, path: &str) -> Result<(String, String), String> {
        let Ok(repository) = &self.repository else {
            return Err("no repository is open".to_owned());
        };

        let git = command::Git::in_work_dir(&repository.path);
        let result = match source {
            DiffSource::Unstaged => (
                git.worktree_diff(path, false),
                git.whole_file_worktree_diff(path, false),
            ),
            DiffSource::Staged => (
                git.worktree_diff(path, true),
                git.whole_file_worktree_diff(path, true),
            ),
            // The same two sides the file tree was built from, so a file opened from a run
            // shows what that whole run did to it.
            DiffSource::Commit => match self.comparison() {
                None => return Err("no commit is selected".to_owned()),
                Some((from, id)) => {
                    let base =
                        from.map_or_else(|| read::EMPTY_TREE.to_owned(), |from| from.to_string());
                    let target = id.to_string();
                    (
                        git.diff(&base, Some(&target), path),
                        git.whole_file_diff(&base, Some(&target), path),
                    )
                }
            },
        };

        match result {
            (Ok(raw), Ok(whole_file)) => Ok((raw, whole_file)),
            (Err(error), _) | (_, Err(error)) => Err(error.to_string()),
        }
    }

    /// The gutter geometry is cached per tree, so anything that changes which rows a tree
    /// shows has to drop it.
    fn redraw_trees(&self) {
        self.change_tree.redraw();
        if let Ok(repository) = &self.repository {
            repository.unstaged_tree.redraw();
            repository.staged_tree.redraw();
        }
    }

    fn rebuild_labels(&mut self) {
        let Ok(repository) = &self.repository else {
            self.labels.clear();
            return;
        };

        let mut labels: HashMap<gix::ObjectId, Vec<Label>> = HashMap::new();
        let pulls = &repository.references.pulls;
        let mut add = |target, kind, name: String, short: String, host| {
            labels.entry(target).or_default().push(Label {
                kind,
                name,
                short,
                host,
                pull: pulls.get(&target).copied(),
                head: false,
            });
        };

        for branch in &repository.references.local_branches {
            add(
                branch.target,
                LabelKind::Branch,
                branch.name.clone(),
                branch.name.clone(),
                None,
            );
        }
        if self.show_remote_branches {
            for branch in &repository.references.remote_branches {
                // "origin/main" names the remote before the branch; the chip shows the
                // forge in its place.
                let (remote, short) = branch
                    .name
                    .split_once('/')
                    .unwrap_or(("", branch.name.as_str()));

                add(
                    branch.target,
                    LabelKind::Remote,
                    branch.name.clone(),
                    short.to_owned(),
                    Some(
                        repository
                            .remotes
                            .get(remote)
                            .cloned()
                            .unwrap_or_else(|| remote.to_owned()),
                    ),
                );
            }
        }
        for stash in &repository.references.stashes {
            add(
                stash.target,
                LabelKind::Stash,
                stash.name.clone(),
                stash.name.clone(),
                None,
            );
        }
        if self.show_tags {
            for tag in &repository.references.tags {
                add(
                    tag.target,
                    LabelKind::Tag,
                    tag.name.clone(),
                    tag.name.clone(),
                    None,
                );
            }
        }

        // A tick on a name the commit already carries says where HEAD is in less room than
        // a chip of its own, which only a commit with no name at all needs.
        if let Some(id) = repository.head.id {
            let at_head = labels.entry(id).or_default();
            let attached = repository.head.name.as_deref();
            let ticked = at_head
                .iter()
                .position(|label| Some(label.name.as_str()) == attached)
                .or_else(|| (!at_head.is_empty()).then_some(0));

            match ticked {
                // The cell shows the first name and counts the rest, so the ticked one leads.
                Some(index) => {
                    at_head[index].head = true;
                    at_head[..=index].rotate_right(1);
                }
                None => at_head.push(Label {
                    kind: LabelKind::Head,
                    name: "HEAD".to_owned(),
                    short: "HEAD".to_owned(),
                    host: None,
                    pull: pulls.get(&id).copied(),
                    head: true,
                }),
            }
        }

        self.labelled = labels.keys().copied().collect();
        self.labels = labels;
    }

    /// Every write goes through git and changes nothing this side of it, so reading the
    /// repository again is what puts the result on screen. A failure keeps git's own message.
    fn write(
        &mut self,
        action: impl FnOnce(&command::Git) -> Result<(), command::Error>,
    ) -> Task<Message> {
        let Ok(repository) = &self.repository else {
            return Task::none();
        };

        let git = command::Git::in_work_dir(&repository.path);
        match action(&git) {
            Ok(()) => {
                self.commit_error = None;
                self.read_repository()
            }
            Err(error) => {
                self.commit_error = Some(error.to_string());
                Task::none()
            }
        }
    }

    /// A picture is asked for once per author and kept whichever repository is open, because
    /// the cache on disk is shared.
    fn request_faces(&mut self) -> Task<Message> {
        let Ok(repository) = &self.repository else {
            return Task::none();
        };

        let mut wanted = Vec::new();
        for slot in &repository.slots[self.visible_rows(repository.slots.len())] {
            let Slot::Commit(index) = slot else { continue };
            let (Some(commit), Some(author)) = (
                repository.commits.get(*index),
                repository.authors.get(*index),
            ) else {
                continue;
            };
            if self.asked.contains(&author.fingerprint) {
                continue;
            }

            self.asked.insert(author.fingerprint);
            wanted.push((
                author.fingerprint,
                avatar::Identity {
                    name: commit.author.name.clone(),
                    email: commit.author.email.clone(),
                },
            ));
        }

        // A face already on disk is a stat away, so it is taken here rather than queued
        // behind anything that reaches the network.
        let mut fetching = Vec::new();
        for (fingerprint, identity) in wanted {
            match avatar::cached(&identity) {
                Some(path) => {
                    if let Some(faces) = Faces::read(&path) {
                        self.pictures.insert(fingerprint, faces);
                    }
                }
                None => fetching.push((fingerprint, identity)),
            }
        }

        Task::batch(fetching.into_iter().map(|(fingerprint, identity)| {
            Task::perform(async move { avatar::fetch(&identity).ok() }, move |path| {
                Message::FaceFetched(fingerprint, path)
            })
        }))
    }

    fn selected_commit(&self) -> Option<&read::CommitSummary> {
        let repository = self.repository.as_ref().ok()?;
        match self.selected? {
            Selection::Commit(index) => repository.commits.get(index),
            // The newer end of a run is the commit its detail is written about.
            Selection::Range { anchor, other } => repository.commits.get(anchor.min(other)),
            Selection::WorkingTree => None,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RepositoryRead(path, reading) => {
                // A second click while the first was still reading wins; the first reading
                // is kept for when that repository is opened again.
                let landed = path == self.path;
                self.reading &= !landed;

                match reading {
                    Ok(reading) => {
                        self.readings.insert(path, (*reading).clone());
                        if landed {
                            self.show(*reading);
                            // Taken now rather than when the reading started, so a change
                            // made while it ran is still noticed by the next check.
                            self.watched = self.git_fingerprint();
                        }
                    }
                    Err(error) if landed => self.repository = Err(error),
                    Err(_) => {}
                }

                if !landed {
                    return Task::none();
                }
                return self.request_faces();
            }

            Message::CommitSelected(index) => self.select_commit(index),
            Message::WorkingTreeSelected => self.select_working_tree(),
            Message::FileSelected(source, path) => return self.select_file(source, path),
            Message::DiffModeChanged(mode) => self.diff_mode = mode,
            Message::DiffClosed => self.diff = None,
            Message::PaneResized(event) => {
                if Some(event.split) == self.sidebar_split {
                    self.sidebar_share = event.ratio;
                }
                self.panes.resize(event.split, event.ratio);
            }
            Message::DirectoryExpanded(path) => {
                self.expanded.insert(path);
                self.redraw_trees();
            }
            Message::FileViewChanged(view) => {
                self.file_view = view;
                self.redraw_trees();
            }
            Message::MenuToggled(menu) => {
                self.menu = if self.menu == Some(menu) {
                    None
                } else {
                    Some(menu)
                };
            }
            Message::Dismissed => {
                self.menu = None;
                self.inbox = false;
                self.context = None;

                // A theme worn while walking the list was never settled on.
                if let Some(launcher) = self.launcher.take()
                    && let launcher::Mode::Themes(previous) = launcher.mode
                {
                    self.wear_theme(previous);
                }
            }
            Message::Checked => {
                let now = self.git_fingerprint();
                if self.reading || now.is_none() || now == self.watched {
                    return Task::none();
                }

                self.watched = now;
                return self.read_repository();
            }
            Message::Refocused => {
                // A file saved in an editor changes nothing in the git directory, so
                // [`git_fingerprint`] cannot see it.
                if self.reading {
                    return Task::none();
                }

                return self.read_repository();
            }
            Message::RefreshRequested => {
                self.menu = None;
                for entry in &mut self.repositories {
                    entry.branch = branch_of(&entry.path);
                }
                return self.read_repository();
            }
            Message::QuitRequested => return iced::exit(),
            Message::HashCopied => {
                self.menu = None;
                if let Some(commit) = self.selected_commit() {
                    return iced::clipboard::write(commit.id.to_string());
                }
            }
            Message::CommitMessageCopied => {
                self.menu = None;
                if let Some(commit) = self.selected_commit() {
                    let message = if commit.body.is_empty() {
                        commit.title.clone()
                    } else {
                        format!("{}\n\n{}", commit.title, commit.body)
                    };
                    return iced::clipboard::write(message);
                }
            }
            Message::RemoteBranchesToggled => {
                self.show_remote_branches = !self.show_remote_branches;
                self.rebuild_labels();
            }
            Message::TagsToggled => {
                self.show_tags = !self.show_tags;
                self.rebuild_labels();
            }
            Message::SettingsOpened(category) => {
                self.menu = None;
                self.inbox = false;
                self.page = Page::Settings(category);
                self.rescan_directories();
            }
            Message::SettingsClosed => self.page = Page::Repository,
            Message::InboxToggled => {
                self.menu = None;
                self.inbox = !self.inbox;
            }
            Message::RepositoryInputChanged(typed) => {
                self.form.repository_completions = completions(&typed);
                self.form.repository = typed;
            }
            Message::RepositoryAdded => match self.typed_repository() {
                Err(error) => self.form.repository_error = Some(error),
                Ok(path) => {
                    self.form.repository_error = None;
                    self.add_repository(path);
                    if self.form.repository_error.is_none() {
                        self.form.repository.clear();
                    }
                }
            },
            Message::RepositoryPathAdded(path) => {
                self.form.repository_error = None;
                self.add_repository(path);
            }
            Message::RepositoryRemoved(path) => self.remove_repository(&path),
            Message::LauncherToggled => {
                if self.launcher.is_some() {
                    self.launcher = None;
                    return Task::none();
                }

                // What the remembered directories hold may have changed since the settings
                // page last looked.
                self.rescan_directories();
                self.launcher = Some(launcher::Launcher::default());
                return iced::advanced::widget::operate(
                    iced::advanced::widget::operation::focusable::focus(launcher::id()),
                );
            }
            Message::LauncherTyped(query) => {
                if let Some(launcher) = &mut self.launcher {
                    launcher.query = query;
                    launcher.selected = 0;
                }
            }
            Message::LauncherMoved(step) => {
                let Some(launcher) = &self.launcher else {
                    return Task::none();
                };

                let rows = launcher::matches(self, launcher);
                let last = rows.len().saturating_sub(1);
                let wanted = (launcher.selected as i32 + step).clamp(0, last as i32) as usize;
                let preview = match rows.get(wanted) {
                    Some(launcher::Row::Theme(choice)) => Some(*choice),
                    _ => None,
                };

                if let Some(launcher) = &mut self.launcher {
                    launcher.selected = wanted;
                }
                if let Some(choice) = preview {
                    self.wear_theme(choice);
                }
            }
            Message::LauncherChosen(index) => {
                let Some(launcher) = &self.launcher else {
                    return Task::none();
                };

                let chosen = match launcher::matches(self, launcher).into_iter().nth(index) {
                    Some(launcher::Row::Repository(find)) => {
                        Chosen::Repository(find.path().to_owned())
                    }
                    Some(launcher::Row::Theme(choice)) => Chosen::Theme(choice),
                    Some(launcher::Row::ChangeTheme) => Chosen::ChangeTheme,
                    None => return Task::none(),
                };

                match chosen {
                    Chosen::ChangeTheme => {
                        let current = self.theme_choice;
                        if let Some(launcher) = &mut self.launcher {
                            launcher.mode = launcher::Mode::Themes(current);
                            launcher.query.clear();
                            launcher.selected = theme::Choice::ALL
                                .iter()
                                .position(|choice| *choice == current)
                                .unwrap_or(0);
                        }
                    }
                    Chosen::Theme(choice) => {
                        self.launcher = None;
                        self.wear_theme(choice);
                        self.state.theme = Some(choice);
                        self.save_state();
                        return self.reread_diff();
                    }
                    Chosen::Repository(path) => {
                        self.launcher = None;
                        self.sidebar = Sidebar::Branches;

                        // Opening one that was only ever a suggestion lists it, which makes
                        // a remembered directory enough on its own.
                        if !self.repositories.iter().any(|entry| entry.path == path) {
                            self.add_repository(path.clone());
                        }
                        return self.open_repository(path);
                    }
                }
            }
            Message::ModifiersChanged(held) => self.modifiers = held,
            Message::ContextOpened(target) => {
                self.context = Some(Context {
                    at: pointer(),
                    target,
                });
            }
            Message::TextCopied(text) => {
                self.context = None;
                return iced::clipboard::write(text);
            }
            Message::RepositoryOpened(path) => {
                self.context = None;
                self.sidebar = Sidebar::Branches;
                return self.open_repository(path);
            }
            Message::WindowResized(width) => self.window_width = width,
            Message::SidebarToggled => {
                let width = if self.rail() {
                    SIDEBAR_WIDTH
                } else {
                    RAIL_WIDTH
                };
                self.set_sidebar_width(width);
            }
            Message::SidebarClosed => self.sidebar = Sidebar::Repositories,
            Message::DescriptionGrabbed => {
                self.description_drag = Some((0.0, self.description_height));
            }
            Message::DescriptionDragged(y) => {
                let Some((anchor, start)) = self.description_drag else {
                    return Task::none();
                };

                // A press carries no position, so the first move is what anchors it.
                if anchor == 0.0 {
                    self.description_drag = Some((y, start));
                    return Task::none();
                }

                self.description_height =
                    (start + y - anchor).clamp(DESCRIPTION_MIN, DESCRIPTION_MAX * 2.0);
            }
            Message::DescriptionDropped => self.description_drag = None,
            Message::SectionFolded(key) => {
                if !self.folded.remove(&key) {
                    self.folded.insert(key);
                }
            }
            Message::BranchSelected(target) => {
                let Ok(repository) = &self.repository else {
                    return Task::none();
                };
                let Some(index) = repository
                    .commits
                    .iter()
                    .position(|commit| commit.id == target)
                else {
                    return Task::none();
                };

                let offset = repository.tops.get(index).copied().unwrap_or_default();
                self.select_commit(index);

                return iced::advanced::widget::operate(
                    iced::advanced::widget::operation::scrollable::scroll_to(
                        history_scroll(),
                        iced::widget::scrollable::AbsoluteOffset {
                            x: Some(0.0),
                            y: Some((offset - ROW_HEIGHT * 3.0).max(0.0)),
                        },
                    ),
                );
            }
            Message::RepositoryGrabbed(path) => {
                self.reorder = Some(Reorder {
                    path,
                    pressed: std::time::Instant::now(),
                    moved: false,
                });
            }
            Message::RepositoryDropped => {
                let Some(reorder) = self.reorder.take() else {
                    return Task::none();
                };

                if !reorder.moved {
                    self.sidebar = Sidebar::Branches;
                    return self.open_repository(reorder.path);
                }

                self.remember_order();
            }
            Message::RowHovered(path) => {
                self.carry_to(&path);
                self.form.hovered = Some(path);
            }
            Message::RowLeft => self.form.hovered = None,
            Message::DirectoryInputChanged(typed) => {
                self.form.directory_completions = completions(&typed);
                self.form.directory = typed;
            }
            Message::DirectoryAdded => {
                let path = PathBuf::from(self.form.directory.trim());
                self.form.directory_error = if path.as_os_str().is_empty() {
                    Some("type the path of a directory first".to_owned())
                } else if !path.is_dir() {
                    Some(format!("{} is not a directory", path.display()))
                } else if self.state.directories.contains(&path) {
                    Some(format!("{} is already remembered", path.display()))
                } else {
                    self.state.directories.push(path);
                    self.form.directory.clear();
                    self.save_state();
                    self.rescan_directories();
                    None
                };
            }
            Message::DirectoryRemoved(path) => {
                self.state.directories.retain(|known| *known != path);
                self.save_state();
                self.rescan_directories();
            }
            Message::IconPickerToggled(path) => {
                let open = self
                    .form
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.repository == path);

                self.form.picker = (!open).then(|| settings::Picker {
                    choices: unignored(&path),
                    repository: path,
                });
            }
            Message::IconBrowsed(repository) => {
                return Task::perform(
                    // Blocking for as long as the reader leaves it open, so it runs off the
                    // thread drawing the window.
                    async move {
                        let file = rfd::FileDialog::new()
                            .set_title("Pick an icon for this repository")
                            .set_directory(&repository)
                            .add_filter("images", &["svg", "png", "ico", "jpg", "jpeg", "webp"])
                            .pick_file();

                        (repository, file)
                    },
                    |(repository, file)| Message::IconFilePicked(repository, file),
                );
            }
            Message::IconFilePicked(repository, file) => {
                let Some(file) = file else {
                    return Task::none();
                };

                // Relative when it is inside the repository; anywhere else there is nothing
                // to be relative to.
                let stored = file
                    .strip_prefix(&repository)
                    .map_or(file.clone(), Path::to_path_buf);

                self.state.set_icon(&repository, stored);
                self.form.picker = None;
                self.save_state();
            }

            Message::IconChosen(repository, file) => {
                self.state.set_icon(&repository, file);
                self.form.picker = None;
                self.save_state();
            }
            Message::IconCleared(repository) => {
                self.state
                    .icons
                    .retain(|icon| icon.repository != repository);
                self.form.picker = None;
                self.save_state();
            }
            Message::ZoomIn => self.set_scale(self.scale + SCALE_STEP),
            Message::ZoomOut => self.set_scale(self.scale - SCALE_STEP),
            Message::ZoomReset => self.set_scale(1.0),
            Message::DividerPressed(divider) => {
                self.drag = Some(Drag {
                    divider,
                    anchor: None,
                    start: 0.0,
                });
            }
            Message::DividerReleased => {
                self.drag = None;
                // Written when the drag ends rather than on every pixel of it.
                self.state.widths = self.widths;
                self.save_state();
            }
            Message::DividerHovered(divider) => self.hovered_divider = Some(divider),
            Message::DividerLeft => self.hovered_divider = None,
            Message::ColumnMenuToggled => self.columns_open = !self.columns_open,
            Message::ColumnToggled(column) => {
                let columns = &mut self.columns;
                match column {
                    HistoryColumn::Labels => columns.labels = !columns.labels,
                    HistoryColumn::Author => columns.author = !columns.author,
                    HistoryColumn::When => columns.when = !columns.when,
                    HistoryColumn::Hash => columns.hash = !columns.hash,
                }

                self.state.columns = self.columns;
                self.save_state();
            }
            Message::HistoryScrolled(offset, height) => {
                self.history_offset = offset;
                self.history_height = height;
                // The graph only draws the rows on screen, so which ones those are is part
                // of what was drawn.
                if let Ok(repository) = &self.repository {
                    repository.graph_cache.clear();
                }
                return self.request_faces();
            }
            Message::DividerMoved(x) => {
                let Some(drag) = self.drag else {
                    return Task::none();
                };

                match drag.anchor {
                    None => {
                        self.drag = Some(Drag {
                            anchor: Some(x),
                            start: self.width_of(drag.divider),
                            ..drag
                        });
                    }
                    Some(anchor) => self.resize(drag.divider, drag.start, x - anchor),
                }
            }

            Message::FileStaged(path) => return self.write(|git| git.add(&path)),
            Message::EverythingStaged => return self.write(command::Git::add_all),
            Message::CommitMessageChanged(typed) => self.commit_message = typed,
            Message::CommitDescriptionActed(action) => self.commit_description.perform(action),
            Message::CommitRequested => {
                let message = self.commit_message.trim().to_owned();
                let description = self.commit_description.text().trim().to_owned();
                if message.is_empty() {
                    self.commit_error = Some("type a message before committing".to_owned());
                } else {
                    let task = self.write(|git| git.commit(&message, &description));
                    if self.commit_error.is_none() {
                        self.commit_message.clear();
                        self.commit_description = text_editor::Content::new();
                    }
                    return task;
                }
            }
            Message::FaceFetched(fingerprint, path) => {
                if let Some(faces) = path.as_deref().and_then(Faces::read) {
                    self.pictures.insert(fingerprint, faces);
                    // The graph is drawn once and kept, so a late face needs it thrown away.
                    if let Ok(repository) = &self.repository {
                        repository.graph_cache.clear();
                    }
                }
            }
            Message::ThemeChanged(choice) => {
                self.wear_theme(choice);
                self.state.theme = Some(choice);
                self.save_state();
                return self.reread_diff();
            }
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let body = match self.page {
            Page::Settings(category) => settings::view(self, category),
            Page::Repository => self.repository_page(),
        };

        let page = column![menu::bar(self.menu), body, footer(self.scale)];

        // A row only hears the button let go of while the pointer is still on it, so a drag
        // that ends anywhere else is caught here. Always the same widget: wrapping it only
        // while a drag runs changes the shape of the tree, and every scroll position under
        // it is thrown away when that happens.
        let mut tracker = mouse_area(page);
        if self.reorder.is_some() {
            tracker = tracker
                .on_release(Message::RepositoryDropped)
                .on_exit(Message::RepositoryDropped);
        }
        let page: Element<'_, Message> = tracker.into();

        if self.menu.is_none() && !self.inbox && self.launcher.is_none() && self.context.is_none() {
            return page;
        }

        let mut layers = vec![page, menu::dismiss_layer()];
        if let Some(menu) = self.menu {
            layers.push(menu::dropdown(menu, self));
        }
        if self.inbox {
            layers.push(menu::inbox());
        }
        if let Some(launcher) = &self.launcher {
            layers.push(launcher::view(self, launcher));
        }
        if let Some(context) = &self.context {
            layers.push(context_menu(context));
        }

        stack(layers).into()
    }

    fn repository_page(&self) -> Element<'_, Message> {
        let panes = PaneGrid::new(&self.panes, |_id, pane, _maximised| {
            pane_grid::Content::new(match (pane, &self.repository) {
                (Pane::Repositories, _) => sidebar(self, &self.repositories),
                (Pane::History, Ok(repository)) => match &self.diff {
                    Some(file) => diff_pane(file, self.diff_mode),
                    None => self.history_pane(repository),
                },
                (Pane::Detail, Ok(repository)) => self.detail(repository),
                (Pane::History, Err(error)) => self.notice(error),
                (Pane::Detail, Err(_)) => container(Space::new())
                    .height(Fill)
                    .style(theme::surface)
                    .into(),
            })
        })
        .on_resize(8, Message::PaneResized)
        .spacing(1)
        .height(Fill);

        let top = toolbar(self, self.repository.as_ref().ok());

        match self.repository.as_ref().ok().and_then(warnings) {
            Some(strip) => column![top, strip, panes].into(),
            None => column![top, panes].into(),
        }
    }

    /// What the history pane says when there is no history to show.
    fn notice(&self, error: &str) -> Element<'_, Message> {
        let line = if self.reading {
            format!("reading {}", name_of(&self.path))
        } else {
            format!("gg: {error}")
        };

        container(text(line).size(BODY).style(text::secondary))
            .center(Fill)
            .into()
    }

    fn detail<'a>(&'a self, repository: &'a Repository) -> Element<'a, Message> {
        let inner = match self.selected {
            None => column![text("select a commit").size(BODY).style(text::secondary)],
            Some(Selection::WorkingTree) => self.working_tree_detail(repository),
            Some(Selection::Commit(_)) => self.commit_detail(),
            Some(Selection::Range { anchor, other }) => {
                self.range_detail(repository, anchor, other)
            }
        };

        // The same widget whether or not the description is being pulled, so the tree keeps
        // its shape and the pane keeps its scroll.
        let mut tracker = mouse_area(container(scrollable(inner.spacing(14))).padding(14));
        if self.description_drag.is_some() {
            tracker = tracker
                .on_move(|point| Message::DescriptionDragged(point.y))
                .on_release(Message::DescriptionDropped)
                .on_exit(Message::DescriptionDropped);
        }

        container(tracker).height(Fill).style(theme::surface).into()
    }

    fn working_tree_detail<'a>(&'a self, repository: &'a Repository) -> Column<'a, Message> {
        let branch = repository.head.name.as_deref().unwrap_or("detached");
        let status = &repository.status;
        let ready = !status.staged.is_empty() && !self.commit_message.trim().is_empty();

        let mut heading = column![
            row![
                text(format!("working tree on {branch}"))
                    .size(BODY)
                    .width(Fill),
                view_toggle("tree", FileView::Tree, self.file_view),
                view_toggle("path", FileView::Path, self.file_view),
            ]
            .spacing(6),
            row![
                message_field("commit message", &self.commit_message)
                    .on_input(Message::CommitMessageChanged)
                    .on_submit(Message::CommitRequested),
                action("commit", ready.then_some(Message::CommitRequested)),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
            // Enter belongs to the message field, so the description takes the lines it is
            // given and is committed from the button.
            message_area(&self.commit_description, self.description_height),
        ]
        .spacing(8);

        if let Some(error) = &self.commit_error {
            heading = heading.push(text(error.as_str()).size(BODY).style(text::danger));
        }

        column![
            heading,
            changes::summary(status.unstaged.iter().chain(&status.staged), self.colours),
            divider(),
            self.file_section(
                "unstaged",
                &status.unstaged,
                &repository.unstaged_tree,
                DiffSource::Unstaged,
            ),
            self.file_section(
                "staged",
                &status.staged,
                &repository.staged_tree,
                DiffSource::Staged,
            ),
        ]
    }

    fn file_section<'a>(
        &'a self,
        title: &'static str,
        files: &'a [read::FileChange],
        tree: &'a changes::Tree,
        source: DiffSource,
    ) -> Element<'a, Message> {
        let mut heading = row![
            text(format!("{title} ({})", files.len()))
                .size(BODY)
                .style(text::secondary)
                .width(Fill),
        ]
        .align_y(iced::Alignment::Center);

        if source == DiffSource::Unstaged && !files.is_empty() {
            heading = heading.push(action("stage all", Some(Message::EverythingStaged)));
        }

        container(
            column![
                heading,
                tree.view(
                    self.tree_rows(files, tree),
                    source,
                    self.colours,
                    self.open_file(source),
                ),
            ]
            .spacing(4),
        )
        .padding([6, 8])
        .width(Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.base.color.into()),
            border: iced::Border {
                radius: 5.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
    }

    /// The file the diff pane is showing, when it came from this side of the repository.
    fn open_file(&self, source: DiffSource) -> Option<&str> {
        self.diff
            .as_ref()
            .filter(|file| file.source == source)
            .map(|file| file.path.as_str())
    }

    fn tree_rows<'a>(
        &self,
        files: &'a [read::FileChange],
        tree: &'a changes::Tree,
    ) -> Vec<changes::Line<'a>> {
        match self.file_view {
            FileView::Path => changes::flat_rows(files),
            FileView::Tree => tree.rows(&self.expanded),
        }
    }

    /// Every change between the two ends of a run, as one diff and one file tree.
    fn range_detail<'a>(
        &'a self,
        repository: &'a Repository,
        anchor: usize,
        other: usize,
    ) -> Column<'a, Message> {
        let (newer, older) = (anchor.min(other), anchor.max(other));
        let count = older - newer + 1;
        let (Some(from), Some(to)) = (repository.commits.get(older), repository.commits.get(newer))
        else {
            return column![text("select a commit").size(BODY).style(text::secondary)];
        };

        let heading = column![
            text(format!("{count} commits")).size(TITLE),
            row![
                text(short_id(from.id)).size(SMALL).font(Font::MONOSPACE),
                text("\u{2192}").size(SMALL).style(text::secondary),
                text(short_id(to.id)).size(SMALL).font(Font::MONOSPACE),
                Space::new().width(Fill),
                view_toggle("tree", FileView::Tree, self.file_view),
                view_toggle("path", FileView::Path, self.file_view),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
            text(format!("{} \u{2026} {}", from.title, to.title))
                .size(SMALL)
                .style(text::secondary),
        ]
        .spacing(6);

        column![heading, self.changed_tree()]
    }

    /// The files a selection touched, however that selection was made.
    fn changed_tree(&self) -> Element<'_, Message> {
        match &self.changed_files {
            Err(error) => text(format!("could not read the changes: {error}"))
                .size(BODY)
                .style(text::danger)
                .into(),
            Ok(files) => column![
                changes::summary(files.iter(), self.colours),
                divider(),
                self.change_tree.view(
                    self.tree_rows(files, &self.change_tree),
                    DiffSource::Commit,
                    self.colours,
                    self.open_file(DiffSource::Commit),
                ),
            ]
            .spacing(8)
            .into(),
        }
    }

    fn commit_detail(&self) -> Column<'_, Message> {
        let Some(commit) = self.selected_commit() else {
            return column![text("select a commit").size(BODY).style(text::secondary)];
        };

        let heading = column![
            message_field("", &commit.title),
            row![
                // A hash has no spaces, so the default word wrapping cannot break it and it
                // runs under the toggles once the interface is scaled up.
                text(commit.id.to_string())
                    .size(SMALL)
                    .font(Font::MONOSPACE)
                    .wrapping(text::Wrapping::Glyph)
                    .width(Fill),
                view_toggle("tree", FileView::Tree, self.file_view),
                view_toggle("path", FileView::Path, self.file_view),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
            authorship(commit, self),
        ]
        .spacing(8);

        let files: Element<'_, Message> = match &self.changed_files {
            Err(error) => text(format!("could not read the changes: {error}"))
                .size(BODY)
                .style(text::danger)
                .into(),
            Ok(files) => column![
                changes::summary(files.iter(), self.colours),
                divider(),
                self.change_tree.view(
                    self.tree_rows(files, &self.change_tree),
                    DiffSource::Commit,
                    self.colours,
                    self.open_file(DiffSource::Commit),
                ),
            ]
            .spacing(8)
            .into(),
        };

        if commit.body.is_empty() {
            column![heading, files]
        } else {
            column![heading, text(&commit.body).size(BODY), files]
        }
    }
}

/// The next commit's message above the working tree, and a commit's own title on a commit.
fn message_field<'a>(placeholder: &'a str, value: &'a str) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(TITLE)
        .padding([8, 10])
        .style(field_style)
}

/// Grows with what is typed, up to [`DESCRIPTION_MAX`], after which it scrolls rather than
/// pushing the files off the pane.
fn message_area(content: &text_editor::Content, height: f32) -> Element<'_, Message> {
    let editor = text_editor(content)
        .placeholder("description (optional)")
        .on_action(Message::CommitDescriptionActed)
        .size(BODY)
        .padding([8, 10])
        .min_height(height)
        .max_height(DESCRIPTION_MAX.max(height))
        .style(editor_style);

    // How a reader gives the field more room before the lines are typed.
    let grip = mouse_area(
        container(
            container(Space::new().width(Length::Fixed(28.0)).height(Fill)).style(
                move |theme: &Theme| container::Style {
                    background: Some(theme.extended_palette().background.strong.color.into()),
                    border: iced::Border {
                        radius: 1.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                },
            ),
        )
        .center_x(Fill)
        .height(Length::Fixed(GRIP_HEIGHT))
        .padding([2, 0]),
    )
    .interaction(mouse::Interaction::ResizingVertically)
    .on_press(Message::DescriptionGrabbed);

    column![editor, grip].into()
}

fn editor_style(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let field = field_style(
        theme,
        match status {
            text_editor::Status::Focused { .. } => {
                text_input::Status::Focused { is_hovered: false }
            }
            _ => text_input::Status::Active,
        },
    );

    text_editor::Style {
        background: field.background,
        border: field.border,
        placeholder: field.placeholder,
        value: field.value,
        selection: field.selection,
    }
}

fn field_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.extended_palette();
    let focused = matches!(status, text_input::Status::Focused { .. });

    text_input::Style {
        background: palette.background.base.color.into(),
        border: iced::Border {
            color: if focused {
                palette.primary.base.color
            } else {
                palette.background.strong.color
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        icon: palette.background.strong.text,
        placeholder: palette.background.weak.text,
        value: palette.background.base.text,
        selection: palette.primary.weak.color,
    }
}

/// Without a message it is drawn dead, which is how "commit" reads with nothing to commit.
fn action(label: &'static str, message: Option<Message>) -> Element<'static, Message> {
    button(text(label).size(BODY))
        .on_press_maybe(message)
        .style(button::secondary)
        .padding([3, 10])
        .into()
}

/// The line is one and a half pixels rather than one: at the interface scales between
/// them a single pixel rounds away to nothing on some rows and shows on others.
fn divider() -> Element<'static, Message> {
    container(Space::new().width(Fill).height(Length::Fixed(1.5)))
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.strong.color.into()),
            snap: true,
            ..container::Style::default()
        })
        .into()
}

/// A sleeping thread rather than iced's own timer, which needs an async runtime feature
/// turned on.
fn ticker() -> Subscription<Message> {
    Subscription::run(|| {
        let (mut sender, receiver) = iced::futures::channel::mpsc::channel(1);

        std::thread::spawn(move || {
            loop {
                std::thread::sleep(WATCH_INTERVAL);
                // A full channel means the last tick has not been read yet, so this one is
                // dropped rather than piled on.
                if sender.is_closed() {
                    break;
                }
                let _ = sender.try_send(());
            }
        });

        iced::futures::StreamExt::map(receiver, |()| Message::Checked)
    })
}

/// `+` and `=` are the same physical key, so the unshifted one has to be told from the
/// shifted one. `modified_key` is the logical key with shift applied; `key` is not.
fn shortcut(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed {
        key,
        modified_key,
        modifiers,
        ..
    } = event
    else {
        return None;
    };

    if modifiers.command() {
        return match modified_key.as_ref() {
            keyboard::Key::Character("+") => Some(Message::ZoomIn),
            keyboard::Key::Character("-") => Some(Message::ZoomOut),
            keyboard::Key::Character("=") => Some(Message::ZoomReset),
            keyboard::Key::Character("k" | "t") => Some(Message::LauncherToggled),
            _ => None,
        };
    }

    // Only ever acted on while the launcher is open; the subscription cannot see that, and
    // a subscription that changed with the state would be rebuilt under the running one.
    match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::Dismissed),
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(Message::LauncherMoved(1)),
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(Message::LauncherMoved(-1)),
        _ => None,
    }
}

fn percentage(scale: f32) -> String {
    format!("{}%", (scale * 100.0).round())
}

fn footer(scale: f32) -> Element<'static, Message> {
    let step = |label: &'static str, message: Message| {
        button(text(label).size(BODY))
            .on_press(message)
            .style(button::text)
            .padding([0, 8])
    };

    container(
        row![
            Space::new().width(Fill),
            step("\u{2212}", Message::ZoomOut),
            text(percentage(scale)).size(BODY).style(text::secondary),
            step("+", Message::ZoomIn),
        ]
        .spacing(2)
        .align_y(iced::Alignment::Center),
    )
    .padding([0, 8])
    .width(Fill)
    .height(Length::Fixed(FOOTER_HEIGHT))
    .style(theme::surface)
    .into()
}

fn toggle(label: &'static str, message: Message, selected: bool) -> Element<'static, Message> {
    button(text(label).size(SMALL))
        .on_press(message)
        .style(if selected {
            button::secondary
        } else {
            button::text
        })
        .padding([1, 8])
        .into()
}

fn view_toggle(
    label: &'static str,
    view: FileView,
    current: FileView,
) -> Element<'static, Message> {
    toggle(label, Message::FileViewChanged(view), view == current)
}

/// The split between the sidebar and the rest is kept, because collapsing the sidebar is
/// moving it.
fn default_panes() -> (pane_grid::State<Pane>, Option<pane_grid::Split>) {
    let (mut state, repositories) = pane_grid::State::new(Pane::Repositories);
    let mut sidebar = None;

    if let Some((history, split)) =
        state.split(pane_grid::Axis::Vertical, repositories, Pane::History)
    {
        state.resize(split, SIDEBAR_SHARE);
        sidebar = Some(split);

        if let Some((_, split)) = state.split(pane_grid::Axis::Vertical, history, Pane::Detail) {
            state.resize(split, 0.62);
        }
    }

    (state, sidebar)
}

/// A remote whose URL will not parse has no host to show.
fn remote_hosts(handle: &gix::Repository) -> HashMap<String, String> {
    handle
        .remote_names()
        .iter()
        .filter_map(|name| {
            let name = name.to_string();
            let remote = handle.find_remote(name.as_str()).ok()?;
            let host = remote.url(gix::remote::Direction::Fetch)?.host()?;

            Some((name, host.to_owned()))
        })
        .collect()
}
fn warnings(repository: &Repository) -> Option<Element<'_, Message>> {
    if repository.warnings.is_empty() {
        return None;
    }

    let lines = repository.warnings.iter().map(|warning| {
        let line = match warning {
            Warning::GitUnusable(error) => {
                format!("git cannot run here, so nothing that writes will work: {error}")
            }
            Warning::SigningWithoutAgent(key) => format!(
                "commits are set to be signed with the ssh key {key}, but SSH_AUTH_SOCK is not set here"
            ),
        };
        text(line).size(BODY).into()
    });

    Some(
        container(Column::with_children(lines).spacing(2))
            .padding([6, 12])
            .width(Fill)
            .style(container::danger)
            .into(),
    )
}

/// The strip above the panes: which repository is open, and what is done to one as a whole.
fn toolbar<'a>(app: &'a App, repository: Option<&'a Repository>) -> Element<'a, Message> {
    let name = repository.map_or_else(|| name_of(&app.path), |repository| repository.name.clone());
    let branch = repository
        .and_then(|repository| repository.head.name.clone())
        .unwrap_or_else(|| "detached".to_owned());

    let open = row![
        icons::repository(app.state.icon(&app.path).as_deref(), TOOLBAR_ICON),
        column![
            clipped(text(name).size(BODY)),
            clipped(text(branch).size(SMALL).style(text::secondary)),
        ]
        .spacing(1),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    container(
        row![
            open,
            Space::new().width(Fill),
            toolbar_action("fetch", icons::Glyph::Fetch),
            toolbar_action("push", icons::Glyph::Push),
            toolbar_action("branch", icons::Glyph::Branch),
        ]
        .spacing(4)
        .height(Fill)
        .align_y(iced::Alignment::Center),
    )
    .padding([0, 12])
    .width(Fill)
    .height(Length::Fixed(TOOLBAR_HEIGHT))
    .align_y(iced::alignment::Vertical::Center)
    .clip(true)
    .style(theme::chrome)
    .into()
}

/// Drawn but not wired to anything yet, so it takes no press.
fn toolbar_action(label: &'static str, glyph: icons::Glyph) -> Element<'static, Message> {
    button(
        column![
            icons::sized(glyph, TOOLBAR_GLYPH),
            text(label).size(SMALL).style(text::secondary),
        ]
        .spacing(2)
        .align_x(iced::Alignment::Center),
    )
    .style(button::text)
    .padding([2, 10])
    .into()
}

/// Which of the two things the left pane is showing: every repository gg knows, or the
/// inside of the one that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sidebar {
    Repositories,
    Branches,
}

fn sidebar<'a>(app: &'a App, entries: &'a [Entry]) -> Element<'a, Message> {
    let rail_only = app.rail();
    let inner: Element<'_, Message> = if rail_only {
        rail(app, entries)
    } else {
        match (app.sidebar, &app.repository) {
            (Sidebar::Branches, Ok(repository)) => branches(app, repository).into(),
            _ => repositories(app, entries).into(),
        }
    };

    container(inner)
        .padding(if rail_only { 6 } else { 10 })
        .height(Fill)
        .style(theme::surface)
        .into()
}

/// The sidebar with no room for names: every repository as its icon, named in a tooltip.
fn rail<'a>(app: &'a App, entries: &'a [Entry]) -> Element<'a, Message> {
    let rows = entries.iter().map(|entry| {
        let open = entry.path == app.path;
        let note = match &entry.branch {
            Ok(branch) => format!("{}\n{branch}", entry.name),
            Err(_) => entry.name.clone(),
        };

        let icon = container(icons::repository(
            app.state.icon(&entry.path).as_deref(),
            SIDEBAR_ICON,
        ))
        .padding(6)
        .style(move |theme: &Theme| row_style(theme, open, false));

        tooltip(
            mouse_area(icon)
                .interaction(mouse::Interaction::Pointer)
                .on_right_press(Message::ContextOpened(Target::Repository(
                    entry.path.clone(),
                )))
                .on_press(Message::RepositoryOpened(entry.path.clone())),
            container(text(note).size(SMALL))
                .padding(6)
                .style(container::rounded_box),
            tooltip::Position::Right,
        )
        .into()
    });

    column![
        toggle_rail(true),
        scrollable(
            Column::with_children(rows)
                .spacing(4)
                .width(Fill)
                .align_x(iced::Alignment::Center)
        )
        .height(Fill),
    ]
    .spacing(8)
    .width(Fill)
    .align_x(iced::Alignment::Center)
    .into()
}

fn toggle_rail(rail: bool) -> Element<'static, Message> {
    button(icons::sized(
        if rail {
            icons::Glyph::Unfolded
        } else {
            icons::Glyph::Back
        },
        SIDEBAR_GLYPH,
    ))
    .on_press(Message::SidebarToggled)
    .style(button::text)
    .padding([2, 4])
    .into()
}

fn repositories<'a>(app: &'a App, entries: &'a [Entry]) -> Column<'a, Message> {
    let rows = entries.iter().map(|entry| {
        repository_row(
            entry,
            app.state.icon(&entry.path),
            entry.path == app.path,
            app.form.hovered.as_deref() == Some(entry.path.as_path()),
            app.reorder
                .as_ref()
                .is_some_and(|drag| drag.carrying(&entry.path)),
        )
    });

    column![
        row![heading_row("repositories", None), toggle_rail(false)]
            .align_y(iced::Alignment::Center),
        scrollable(Column::with_children(rows).spacing(2)).height(Fill),
    ]
    .spacing(8)
}

fn branches<'a>(app: &'a App, repository: &'a Repository) -> Column<'a, Message> {
    let head = repository.head.name.as_deref();
    let mut sections = Column::new().spacing(14);

    let folded = |key: &str| app.folded.contains(key);
    let mut local = Column::new().spacing(1);
    for branch in &repository.references.local_branches {
        local = local.push(branch_row(
            branch,
            &branch.name,
            LabelKind::Branch,
            head == Some(branch.name.as_str()),
            app.form.hovered.as_deref() == Some(Path::new(&branch.name)),
        ));
    }
    for worktree in &repository.worktrees {
        local = local.push(worktree_row(worktree));
    }
    let mut group = column![section(
        "local",
        "local",
        folded("local"),
        Some("new worktree")
    )];
    if !folded("local") {
        group = group.push(local);
    }
    sections = sections.push(group.spacing(4));

    let mut remotes: BTreeMap<&str, Vec<&read::Reference>> = BTreeMap::new();
    for branch in &repository.references.remote_branches {
        let (remote, _) = branch.name.split_once('/').unwrap_or(("", &branch.name));
        remotes.entry(remote).or_default().push(branch);
    }

    if !remotes.is_empty() {
        let mut all = Column::new().spacing(6);
        for (remote, branches) in remotes {
            let host = repository.remotes.get(remote);
            let mut rows = Column::new().spacing(1);
            for branch in branches {
                let short = branch
                    .name
                    .split_once('/')
                    .map_or(&*branch.name, |(_, s)| s);
                rows = rows.push(branch_row(
                    branch,
                    short,
                    LabelKind::Remote,
                    false,
                    app.form.hovered.as_deref() == Some(Path::new(&branch.name)),
                ));
            }

            let mut title = row![text(remote).size(SMALL).style(text::secondary)]
                .spacing(5)
                .align_y(iced::Alignment::Center);
            if let Some(host) = host {
                title = title.push(icons::forge(host, SMALL));
            }

            all = all.push(column![container(title).padding([0, 6]), rows].spacing(2));
        }

        let mut group = column![section(
            "remote",
            "remote",
            folded("remote"),
            Some("new remote")
        )];
        if !folded("remote") {
            group = group.push(all);
        }
        sections = sections.push(group.spacing(4));
    }

    column![
        heading_row(&repository.name, Some(Message::SidebarClosed)),
        scrollable(sections).height(Fill),
    ]
    .spacing(8)
}

/// With a message the line carries the arrow back to the list of repositories.
fn heading_row(title: &str, back: Option<Message>) -> Element<'_, Message> {
    let mut line = row![].spacing(4).align_y(iced::Alignment::Center);

    if let Some(back) = back {
        line = line.push(
            button(icons::sized(icons::Glyph::Back, SIDEBAR_GLYPH))
                .on_press(back)
                .style(button::text)
                .padding([2, 4]),
        );
    }

    container(line.push(text(title).size(SMALL).style(text::secondary)))
        .padding([0, 2])
        .width(Fill)
        .into()
}

/// Folds the rows under it away when it is pressed. The button on its right adds another of
/// whatever it heads.
fn section<'a>(
    title: &'a str,
    key: &'a str,
    folded: bool,
    add: Option<&'static str>,
) -> Element<'a, Message> {
    let mut line = row![
        icons::sized(
            if folded {
                icons::Glyph::Folded
            } else {
                icons::Glyph::Unfolded
            },
            SMALL,
        ),
        text(title).size(SMALL).style(text::secondary).width(Fill),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center);

    if let Some(what) = add {
        // Drawn but not wired: what it opens is a form that is not written yet.
        let plus: Element<'_, Message> = tooltip(
            button(icons::sized(icons::Glyph::Plus, SMALL))
                .style(button::text)
                .padding([0, 2]),
            container(text(what).size(SMALL))
                .padding(4)
                .style(container::rounded_box),
            tooltip::Position::Left,
        )
        .into();

        line = line.push(plus);
    }

    let folder = button(line)
        .on_press(Message::SectionFolded(key.to_owned()))
        .style(button::text)
        .padding([2, 6])
        .width(Fill);

    column![folder, divider()].spacing(3).into()
}

fn branch_row<'a>(
    branch: &'a read::Reference,
    label: &'a str,
    kind: LabelKind,
    current: bool,
    hovered: bool,
) -> Element<'a, Message> {
    let line = row![
        icons::sized(kind.glyph(), SIDEBAR_GLYPH),
        clipped(text(label).size(BODY)).width(Fill),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    mouse_area(
        container(line)
            .padding([3, 6])
            .width(Fill)
            .clip(true)
            .style(move |theme: &Theme| row_style(theme, current, hovered)),
    )
    .interaction(mouse::Interaction::Pointer)
    .on_enter(Message::RowHovered(PathBuf::from(&branch.name)))
    .on_exit(Message::RowLeft)
    .on_right_press(Message::ContextOpened(Target::Reference {
        kind,
        name: branch.name.clone(),
        target: branch.target,
    }))
    .on_press(Message::BranchSelected(branch.target))
    .into()
}

fn worktree_row(worktree: &read::Worktree) -> Element<'_, Message> {
    let label = match &worktree.branch {
        Some(branch) => format!("{} \u{2192} {branch}", worktree.name),
        None => worktree.name.clone(),
    };

    container(
        row![
            icons::sized(icons::Glyph::Worktree, SIDEBAR_GLYPH),
            clipped(text(label).size(BODY).style(text::secondary)).width(Fill),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .padding([3, 6])
    .width(Fill)
    .clip(true)
    .into()
}

fn row_style(theme: &Theme, current: bool, hovered: bool) -> container::Style {
    let palette = theme.extended_palette();
    let background = match (current, hovered) {
        (true, _) => Some(palette.background.strong.color),
        (false, true) => Some(palette.background.weaker.color),
        (false, false) => None,
    };

    container::Style {
        background: background.map(Into::into),
        text_color: Some(palette.background.base.text),
        border: iced::Border {
            radius: 4.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

/// A row is not a button: a button reports a press only once it is let go of, and dragging
/// one of these has to start the moment it is pressed.
fn repository_row(
    entry: &Entry,
    icon: Option<PathBuf>,
    open: bool,
    hovered: bool,
    dragged: bool,
) -> Element<'_, Message> {
    // Secondary text is the same grey as the highlight behind the open entry, so the open
    // one has to keep the ordinary colour to stay readable.
    let branch = match &entry.branch {
        Ok(branch) if open => text(branch).size(BODY),
        Ok(branch) => text(branch).size(BODY).style(text::secondary),
        Err(error) => text(error.as_str()).size(SMALL).style(text::danger),
    };

    let line = row![
        icons::repository(icon.as_deref(), SIDEBAR_ICON),
        column![clipped(text(&entry.name).size(BODY)), clipped(branch)]
            .spacing(2)
            .width(Fill),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let row = container(line)
        .padding(8)
        .width(Fill)
        .height(Length::Fixed(SIDEBAR_ROW_HEIGHT))
        .clip(true)
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            let background = match (open, hovered || dragged) {
                (true, _) => Some(palette.background.strong.color),
                (false, true) => Some(palette.background.weak.color),
                (false, false) => None,
            };

            container::Style {
                background: background.map(Into::into),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    color: if dragged {
                        palette.primary.base.color
                    } else {
                        iced::Color::TRANSPARENT
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }
        });

    mouse_area(row)
        .interaction(mouse::Interaction::Pointer)
        .on_press(Message::RepositoryGrabbed(entry.path.clone()))
        .on_right_press(Message::ContextOpened(Target::Repository(
            entry.path.clone(),
        )))
        .on_enter(Message::RowHovered(entry.path.clone()))
        .on_exit(Message::RowLeft)
        .on_release(Message::RepositoryDropped)
        .into()
}

impl App {
    /// Capped at [`GRAPH_MAX_WIDTH`] until the reader drags it, after which they get exactly
    /// what they ask for.
    fn graph_width(&self, repository: &Repository) -> f32 {
        let natural = graph::GRAPH_INSET + repository.lanes as f32 * LANE_WIDTH + LANE_WIDTH / 2.0;

        self.widths
            .graph
            .unwrap_or_else(|| natural.min(GRAPH_MAX_WIDTH))
            .max(GRAPH_MIN_WIDTH)
    }

    fn width_of(&self, divider: Divider) -> f32 {
        match divider {
            Divider::Labels => self.widths.labels,
            Divider::Graph => match &self.repository {
                Ok(repository) => self.graph_width(repository),
                Err(_) => GRAPH_MIN_WIDTH,
            },
            Divider::Author => self.widths.author,
            Divider::When => self.widths.when,
            Divider::Hash => self.widths.hash,
        }
    }

    /// A divider left of the message moves with the column it ends, one right of it with the
    /// column it starts, so the boundary follows the pointer either way.
    fn resize(&mut self, divider: Divider, start: f32, delta: f32) {
        let widths = &mut self.widths;

        match divider {
            Divider::Labels => widths.labels = (start + delta).max(COLUMN_MIN_WIDTH),
            Divider::Graph => widths.graph = Some((start + delta).max(GRAPH_MIN_WIDTH)),
            Divider::Author => widths.author = (start - delta).max(COLUMN_MIN_WIDTH),
            Divider::When => widths.when = (start - delta).max(COLUMN_MIN_WIDTH),
            Divider::Hash => widths.hash = (start - delta).max(COLUMN_MIN_WIDTH),
        }
    }

    /// Every row is exactly ROW_HEIGHT tall, so which are on screen follows from the scroll
    /// offset alone and the rest can be left as empty space.
    fn visible_rows(&self, total: usize) -> Range<usize> {
        let first = (self.history_offset / ROW_HEIGHT).floor().max(0.0) as usize;
        let shown = (self.history_height / ROW_HEIGHT).ceil() as usize;
        let start = first.saturating_sub(OVERSCAN);

        start..(first + shown + OVERSCAN).min(total)
    }

    fn history_pane<'a>(&'a self, repository: &'a Repository) -> Element<'a, Message> {
        let dirty = repository.is_dirty();
        let total = repository.slots.len();
        let height = Length::Fixed(total as f32 * ROW_HEIGHT);
        let range = self.visible_rows(total);
        let graph_width = self.graph_width(repository);

        let lanes = canvas(graph::Graph {
            rows: &repository.rows,
            cache: &repository.graph_cache,
            working_tree: dirty,
            tops: &repository.tops,
            authors: &repository.authors,
            pictures: &self.pictures,
            stashes: &repository.stashes,
            labelled: &self.labelled,
            range: commit_range(&repository.slots, &range),
            colours: self.colours,
        })
        .width(Length::Fixed(graph_width))
        .height(height);

        // Standing in for the slots that were skipped, so the scrollbar still measures the
        // whole history and every row keeps the y it would have had.
        let spacer = |rows: usize| Space::new().height(Length::Fixed(rows as f32 * ROW_HEIGHT));

        let joint_lit = self.hovered_divider == Some(Divider::Labels)
            || self
                .drag
                .is_some_and(|drag| drag.divider == Divider::Labels);

        let mut names = Column::new().push(spacer(range.start));
        let mut joint = Column::new().push(spacer(range.start));
        let mut shades = Column::new().push(spacer(range.start));
        let mut edge = Column::new().push(spacer(range.start));
        let mut entries = Column::new().push(spacer(range.start));
        for slot in &repository.slots[range.clone()] {
            match slot {
                Slot::WorkingTree => {
                    let selected = self.selected == Some(Selection::WorkingTree);
                    names = names.push(blank_cell(self.widths.labels));
                    joint = joint.push(joint_cell(None, joint_lit));
                    shades = shades.push(shade(
                        lane_colour(self.colours, 0),
                        selected,
                        0,
                        graph_width,
                    ));
                    edge = edge.push(edge_cell(Some(lane_colour(self.colours, 0))));
                    entries = entries.push(working_tree_row(&repository.status, selected));
                }
                Slot::Separator(label) => {
                    names = names.push(blank_cell(self.widths.labels));
                    joint = joint.push(joint_cell(None, joint_lit));
                    shades = shades.push(blank_row());
                    edge = edge.push(edge_cell(None));
                    entries = entries.push(separator_row(label));
                }
                Slot::Commit(index) => {
                    let Some(commit) = repository.commits.get(*index) else {
                        continue;
                    };
                    let selected = self.selected.is_some_and(|selected| selected.holds(*index));
                    let lane = repository.rows.get(*index).map_or(0, |row| row.lane);
                    let tint = lane_colour(self.colours, lane);
                    names = names.push(label_cell(
                        self.labels.get(&commit.id),
                        tint,
                        self.widths.labels,
                        commit.id,
                    ));
                    joint = joint.push(joint_cell(
                        self.labelled.contains(&commit.id).then_some(tint),
                        joint_lit,
                    ));
                    shades = shades.push(shade(tint, selected, lane, graph_width));
                    edge = edge.push(edge_cell(Some(tint)));
                    entries = entries.push(commit_row(
                        *index,
                        commit,
                        self.columns,
                        self.widths,
                        selected,
                    ));
                }
            }
        }
        let tail = total - range.end;
        names = names.push(spacer(tail));
        joint = joint.push(spacer(tail)).width(Length::Fixed(DIVIDER_WIDTH));
        shades = shades.push(spacer(tail)).width(Fill);
        edge = edge.push(spacer(tail)).width(Length::Fixed(DIVIDER_WIDTH));
        entries = entries.push(spacer(tail));

        let graph = stack![lanes].push_under(shades);
        let mut columns = row![].align_y(iced::Alignment::Start);
        if self.columns.labels {
            columns = columns.push(names).push(
                mouse_area(joint)
                    .interaction(mouse::Interaction::ResizingHorizontally)
                    .on_enter(Message::DividerHovered(Divider::Labels))
                    .on_exit(Message::DividerLeft)
                    .on_press(Message::DividerPressed(Divider::Labels)),
            );
        }

        // The coloured strip beside the graph is that divider, so it is dragged from here.
        let handle = mouse_area(edge)
            .interaction(mouse::Interaction::ResizingHorizontally)
            .on_press(Message::DividerPressed(Divider::Graph));

        let body = scrollable(columns.push(graph).push(handle).push(entries))
            .id(history_scroll())
            .on_scroll(|viewport| {
                Message::HistoryScrolled(viewport.absolute_offset().y, viewport.bounds().height)
            })
            .height(Fill);

        // The pointer leaves a thin divider the moment it moves, so a drag is followed from
        // the whole pane, headings included, and only while one is actually running.
        let pane = column![self.header(graph_width), body].width(Fill);
        // The same widget whether or not a divider is being pulled: swapping one in and out
        // would change the shape of the tree and reset the scroll under it.
        let mut tracker = mouse_area(pane);
        if self.drag.is_some() {
            tracker = tracker
                .on_move(|point| Message::DividerMoved(point.x))
                .on_release(Message::DividerReleased)
                .on_exit(Message::DividerReleased);
        }
        let tracked: Element<'_, Message> = tracker.into();

        if !self.columns_open {
            return tracked;
        }

        stack![
            tracked,
            button(Space::new())
                .on_press(Message::ColumnMenuToggled)
                .style(button::text)
                .padding(0)
                .width(Fill)
                .height(Fill),
            column![
                Space::new().height(Length::Fixed(HISTORY_HEADER_HEIGHT)),
                row![Space::new().width(Fill), column_menu(self.columns)],
            ],
        ]
        .into()
    }

    fn header(&self, graph_width: f32) -> Element<'_, Message> {
        let lit = |divider: Divider| {
            self.hovered_divider == Some(divider)
                || self.drag.is_some_and(|drag| drag.divider == divider)
        };
        let mut line = row![].align_y(iced::Alignment::Center);

        if self.columns.labels {
            line = line
                .push(heading("branch / tag", Length::Fixed(self.widths.labels)))
                .push(divider_handle(Divider::Labels, lit(Divider::Labels)));
        }
        line = line
            .push(heading("graph", Length::Fixed(graph_width)))
            .push(divider_handle(Divider::Graph, lit(Divider::Graph)))
            .push(heading("commit message", Fill));

        if self.columns.author {
            line = line
                .push(divider_handle(Divider::Author, lit(Divider::Author)))
                .push(heading("author", Length::Fixed(self.widths.author)));
        }
        if self.columns.when {
            line = line
                .push(divider_handle(Divider::When, lit(Divider::When)))
                .push(heading("date", Length::Fixed(self.widths.when)));
        }
        if self.columns.hash {
            line = line
                .push(divider_handle(Divider::Hash, lit(Divider::Hash)))
                .push(heading("sha", Length::Fixed(self.widths.hash)));
        }

        let settings = row![
            Space::new().width(Fill),
            button(icons::sized(icons::Glyph::Sliders, SIDEBAR_GLYPH))
                .on_press(Message::ColumnMenuToggled)
                .style(button::text)
                .padding([0, 6]),
        ]
        .height(Fill)
        .align_y(iced::Alignment::Center);

        container(stack![line, settings])
            .width(Fill)
            .height(Length::Fixed(HISTORY_HEADER_HEIGHT))
            .clip(true)
            .style(theme::surface)
            .into()
    }
}

/// Slots and commits are not the same list: the working tree and the date breaks take slots
/// of their own, and the graph is drawn by commit.
fn commit_range(slots: &[Slot], range: &Range<usize>) -> Range<usize> {
    let mut first = None;
    let mut last = 0;

    for slot in &slots[range.clone()] {
        if let Slot::Commit(index) = slot {
            first.get_or_insert(*index);
            last = *index;
        }
    }

    match first {
        Some(first) => first..last + 1,
        None => 0..0,
    }
}

fn heading(label: &'static str, width: Length) -> Element<'static, Message> {
    container(clipped(text(label).size(SMALL).style(text::secondary)))
        .width(width)
        .height(Fill)
        .padding([0, CELL_PAD])
        .align_y(iced::alignment::Vertical::Center)
        .clip(true)
        .into()
}

fn column_menu(columns: Columns) -> Element<'static, Message> {
    let item = |label: &'static str, on: bool, column: HistoryColumn| {
        button(text(format!("{}  {label}", if on { "\u{2713}" } else { " " })).size(BODY))
            .on_press(Message::ColumnToggled(column))
            .style(button::text)
            .width(Fill)
            .padding([3, 8])
            .into()
    };

    container(
        Column::with_children(vec![
            text("Columns").size(SMALL).style(text::secondary).into(),
            item("Labels", columns.labels, HistoryColumn::Labels),
            item("Author", columns.author, HistoryColumn::Author),
            item("Date", columns.when, HistoryColumn::When),
            item("Hash", columns.hash, HistoryColumn::Hash),
        ])
        .spacing(1),
    )
    .padding(6)
    .width(Length::Fixed(160.0))
    .style(container::rounded_box)
    .into()
}

/// The first name is shown and the rest collapse into a count, so twenty tags on one commit
/// take the same room as one. The full list is in the tooltip.
fn label_cell(
    labels: Option<&Vec<Label>>,
    tint: Color,
    width: f32,
    target: gix::ObjectId,
) -> Element<'_, Message> {
    let labels = labels.map(Vec::as_slice).unwrap_or_default();
    let Some((first, rest)) = labels.split_first() else {
        return blank_cell(width);
    };

    let mut line = row![chip(first, tint)]
        .spacing(4)
        .align_y(iced::Alignment::Center);
    if !rest.is_empty() {
        line = line.push(
            container(text(format!("+{}", rest.len())).size(BODY))
                .padding([2, 7])
                .style(move |theme: &Theme| chip_style(theme, tint)),
        );
    }

    // Runs from the last chip to the edge of the column; the graph carries the line the
    // rest of the way to the node.
    let cell = container(
        line.push(
            container(
                Space::new()
                    .width(Fill)
                    .height(Length::Fixed(CONNECTOR_WIDTH)),
            )
            .style(move |_: &Theme| container::Style {
                background: Some(tint.into()),
                ..container::Style::default()
            }),
        ),
    )
    .width(Length::Fixed(width))
    .height(Length::Fixed(ROW_HEIGHT))
    .align_y(iced::alignment::Vertical::Center)
    .padding(iced::Padding::default().left(f32::from(CELL_PAD)))
    .clip(true);

    // The menu belongs to the first name on the row, which is the one the chip shows.
    let clicked = mouse_area(cell).on_right_press(Message::ContextOpened(Target::Reference {
        kind: first.kind,
        name: first.name.clone(),
        target,
    }));

    if rest.is_empty() {
        return clicked.into();
    }

    tooltip(clicked, label_list(labels), tooltip::Position::Bottom).into()
}

/// A list rather than chips: chips would each be a different width and no two names would
/// start in the same place.
fn label_list(labels: &[Label]) -> Element<'_, Message> {
    // The tick keeps a column of its own on every line, or the one name that carries it
    // would start further along than the rest.
    let ticked = labels.iter().any(|label| label.head);
    let lines = labels.iter().map(move |label| {
        let mut line = row![].spacing(6).align_y(iced::Alignment::Center);
        if ticked {
            let mark: Element<'_, Message> = if label.head {
                icons::sized(icons::Glyph::Check, LABEL_ICON)
            } else {
                Space::new().width(Length::Fixed(LABEL_ICON)).into()
            };
            line = line.push(mark);
        }
        line = line
            .push(icons::sized(label.kind.glyph(), LABEL_ICON))
            .push(text(label.name.as_str()).size(BODY));

        if let Some(host) = &label.host {
            line = line.push(icons::forge(host, LABEL_ICON));
        }
        if let Some(pull) = label.pull {
            line = line
                .push(icons::sized(icons::Glyph::PullRequest, LABEL_ICON))
                .push(text(format!("#{pull}")).size(SMALL).style(text::secondary));
        }

        line.into()
    });

    container(Column::with_children(lines).spacing(4))
        .padding(8)
        .style(container::rounded_box)
        .into()
}

fn chip(label: &Label, tint: Color) -> Element<'_, Message> {
    // A remote branch leads with nothing: the forge mark on its right already says where it
    // lives, and a globe beside it would say it twice.
    let mut line = row![].spacing(4).align_y(iced::Alignment::Center);
    if label.head {
        line = line.push(icons::sized(icons::Glyph::Check, LABEL_ICON));
    }
    if label.host.is_none() {
        line = line.push(icons::sized(label.kind.glyph(), LABEL_ICON));
    }
    line = line.push(clipped(text(label.short.as_str()).size(BODY)));

    if let Some(host) = &label.host {
        line = line.push(icons::forge(host, LABEL_ICON));
    }
    if label.pull.is_some() {
        line = line.push(icons::coloured(icons::Glyph::PullRequest, LABEL_ICON, tint));
    }

    container(line)
        .padding([2, 7])
        .style(move |theme: &Theme| chip_style(theme, tint))
        .into()
}

/// The colour of the lane the commit sits on, which ties the name to its dot.
fn chip_style(theme: &Theme, tint: Color) -> container::Style {
    container::Style {
        background: Some(Color { a: 0.22, ..tint }.into()),
        text_color: Some(theme.extended_palette().background.base.text),
        border: iced::Border {
            color: Color { a: 0.55, ..tint },
            width: 1.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    }
}

/// Every row here is one line tall, and a wrapped one would spill into its neighbour.
fn clipped<'a>(label: iced::widget::Text<'a>) -> iced::widget::Text<'a> {
    label.wrapping(iced::widget::text::Wrapping::None)
}

fn working_tree_row(status: &read::Status, selected: bool) -> Element<'_, Message> {
    let mut line = row![
        container(text("// WIP").size(BODY).font(Font::MONOSPACE))
            .padding([0, 6])
            .style(container::rounded_box),
    ]
    .spacing(10)
    .height(Fill)
    .align_y(iced::Alignment::Center);

    for (letter, count, style) in kind_counts(status) {
        line = line.push(
            row![
                text(letter).size(SMALL).font(Font::MONOSPACE).style(style),
                text(format!("{count}")).size(BODY).style(text::secondary),
            ]
            .spacing(3)
            .align_y(iced::Alignment::Center),
        );
    }

    button(line.push(Space::new().width(Fill)))
        .on_press(Message::WorkingTreeSelected)
        .style(if selected {
            button::secondary
        } else {
            button::text
        })
        .padding([0, 10])
        .width(Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
}

type KindStyle = fn(&Theme) -> text::Style;

fn kind_counts(status: &read::Status) -> Vec<(&'static str, usize, KindStyle)> {
    let kinds: [(ChangeKind, &'static str, KindStyle); 4] = [
        (ChangeKind::Added, "A", text::success),
        (ChangeKind::Modified, "M", text::primary),
        (ChangeKind::Deleted, "D", text::danger),
        (ChangeKind::Renamed, "R", text::secondary),
    ];

    kinds
        .into_iter()
        .filter_map(|(kind, letter, style)| {
            let count = status
                .staged
                .iter()
                .chain(&status.unstaged)
                .filter(|file| file.kind == kind)
                .count();

            (count > 0).then_some((letter, count, style))
        })
        .collect()
}

fn commit_row(
    index: usize,
    commit: &read::CommitSummary,
    columns: Columns,
    widths: Widths,
    selected: bool,
) -> Element<'_, Message> {
    // On a selected row the highlight is the same grey as text::secondary, which makes
    // the trailing columns vanish. They take the ordinary colour there instead.
    let muted: fn(&Theme) -> text::Style = if selected {
        text::default
    } else {
        text::secondary
    };

    let mut line =
        row![body_cell(text(&commit.title).size(BODY), Fill)].align_y(iced::Alignment::Center);

    if columns.author {
        line = line.push(gap()).push(body_cell(
            text(&commit.author.name).size(BODY).style(muted),
            Length::Fixed(widths.author),
        ));
    }
    if columns.when {
        line = line.push(gap()).push(body_cell(
            text(&commit.when).size(BODY).style(muted),
            Length::Fixed(widths.when),
        ));
    }
    if columns.hash {
        line = line.push(gap()).push(body_cell(
            text(short_id(commit.id))
                .size(BODY)
                .font(Font::MONOSPACE)
                .style(muted),
            Length::Fixed(widths.hash),
        ));
    }

    let row = button(line)
        .on_press(Message::CommitSelected(index))
        .style(move |theme: &Theme, status| commit_style(theme, selected, status))
        .padding(0)
        .width(Fill)
        .height(Length::Fixed(ROW_HEIGHT));

    // A button hears the left press only, so the right one is heard by the area around it.
    mouse_area(row)
        .on_right_press(Message::ContextOpened(Target::Commit(commit.id)))
        .into()
}

/// One cell of a row, in the same width and with the same inset as the heading over it.
fn body_cell(label: iced::widget::Text<'_>, width: Length) -> Element<'_, Message> {
    container(clipped(label))
        .width(width)
        .height(Fill)
        .align_y(iced::alignment::Vertical::Center)
        .padding([0, CELL_PAD])
        .clip(true)
        .into()
}

/// Where a divider runs, so a row is cut the same way its heading is.
fn gap() -> Element<'static, Message> {
    Space::new().width(Length::Fixed(DIVIDER_WIDTH)).into()
}

fn short_id(id: gix::ObjectId) -> String {
    id.to_hex_with_len(8).to_string()
}

fn history_scroll() -> iced::widget::Id {
    iced::widget::Id::new("history")
}

/// git records an author and a committer; the committer is shown only when it differs, which
/// is when it is the interesting part.
fn authorship<'a>(commit: &'a read::CommitSummary, app: &'a App) -> Element<'a, Message> {
    let same = commit.author.name == commit.committer.name
        && commit.author.email == commit.committer.email
        && commit.author.seconds == commit.committer.seconds;

    let mut lines = column![text(&commit.author.name).size(BODY)].spacing(2);
    lines = lines.push(
        text(format!(
            "authored {}",
            when::stamp(commit.author.seconds, commit.author.offset)
        ))
        .size(SMALL)
        .style(text::secondary),
    );

    if !same {
        lines = lines.push(
            text(format!(
                "committed by {} {}",
                commit.committer.name,
                when::stamp(commit.committer.seconds, commit.committer.offset)
            ))
            .size(SMALL)
            .style(text::secondary),
        );
    }

    for name in co_authors(&commit.body) {
        lines = lines.push(
            row![
                icons::sized(icons::Glyph::PullRequest, SMALL),
                text(format!("co-authored by {name}")).size(SMALL),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        );
    }

    row![face(app, &commit.author), lines.width(Fill)]
        .spacing(10)
        .align_y(iced::Alignment::Start)
        .into()
}

fn face<'a>(app: &'a App, author: &'a read::Signature) -> Element<'a, Message> {
    let identity = avatar::Identity {
        name: author.name.clone(),
        email: author.email.clone(),
    };

    match app.pictures.get(&identity.fingerprint()) {
        Some(faces) => iced::widget::image(faces.square.clone())
            .width(Length::Fixed(FACE_SIZE))
            .height(Length::Fixed(FACE_SIZE))
            .into(),
        None => container(Space::new())
            .width(Length::Fixed(FACE_SIZE))
            .height(Length::Fixed(FACE_SIZE))
            .style(move |theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.strong.color.into()),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    }
}

/// `Co-authored-by: Name <address>` is the convention every forge reads.
fn co_authors(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("Co-authored-by:"))
        .map(|rest| rest.split('<').next().unwrap_or(rest).trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

fn diff_scroll() -> iced::widget::Id {
    iced::widget::Id::new("diff")
}

fn diff_pane(file: &FileDiff, mode: diff::Mode) -> Element<'_, Message> {
    let header = row![
        cell(text(&file.path).size(BODY).font(Font::MONOSPACE)),
        toggle(
            "highlighted",
            Message::DiffModeChanged(diff::Mode::Highlighted),
            mode == diff::Mode::Highlighted,
        ),
        toggle(
            "raw",
            Message::DiffModeChanged(diff::Mode::Raw),
            mode == diff::Mode::Raw,
        ),
        button(icons::icon(icons::Glyph::Cross))
            .on_press(Message::DiffClosed)
            .style(button::text)
            .padding([1, 6]),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let body: Element<'_, Message> = match &file.body {
        Err(error) => text(format!("could not read the diff: {error}"))
            .size(BODY)
            .style(text::danger)
            .into(),
        Ok(body) => diff::view(body, mode, diff_scroll(), &file.overview),
    };

    container(column![header, body].spacing(10))
        .padding(14)
        .height(Fill)
        .style(theme::surface)
        .into()
}

/// The lane colour bleeds out of the node and fades away to the left of it, so the eye can
/// carry a face back to the branch it belongs to.
fn shade(tint: Color, selected: bool, lane: usize, width: f32) -> Element<'static, Message> {
    let strength = if selected { 0.40 } else { 0.12 };
    // Nothing left of the face is shaded, because the node is where the commit begins.
    let start = (graph::lane_x(lane) / width).clamp(0.0, 1.0);
    let gradient = iced::gradient::Linear::new(std::f32::consts::FRAC_PI_2)
        .add_stop(start, Color { a: 0.0, ..tint })
        .add_stop(
            1.0,
            Color {
                a: strength,
                ..tint
            },
        );

    container(blank_row())
        .style(move |_: &Theme| container::Style {
            background: Some(iced::Background::Gradient(gradient.into())),
            ..container::Style::default()
        })
        .into()
}

/// The grab area between two headings. It stays as wide as the gap the columns leave for
/// it, so everything below lines up, but the line drawn in it is a hairline.
fn divider_handle(divider: Divider, lit: bool) -> Element<'static, Message> {
    mouse_area(
        container(rule(lit))
            .width(Length::Fixed(DIVIDER_WIDTH))
            .height(Fill)
            .center_x(Length::Fixed(DIVIDER_WIDTH)),
    )
    .interaction(mouse::Interaction::ResizingHorizontally)
    .on_enter(Message::DividerHovered(divider))
    .on_exit(Message::DividerLeft)
    .on_press(Message::DividerPressed(divider))
    .into()
}

fn rule(lit: bool) -> Element<'static, Message> {
    container(Space::new().width(Length::Fixed(DIVIDER_LINE)).height(Fill))
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();

            container::Style {
                background: Some(if lit {
                    palette.primary.base.color.into()
                } else {
                    palette.background.strong.color.into()
                }),
                ..container::Style::default()
            }
        })
        .into()
}

/// The gap between the labels and the graph: empty unless the row carries a label, whose
/// line crosses it, or the divider is being pointed at or pulled.
fn joint_cell(tint: Option<Color>, lit: bool) -> Element<'static, Message> {
    let carried: Element<'_, Message> = match tint {
        None => Space::new().width(Fill).height(Fill).into(),
        // The colour goes on the line itself and not on the cell holding it, or the whole
        // gap between the columns is painted.
        Some(tint) => container(
            container(
                Space::new()
                    .width(Fill)
                    .height(Length::Fixed(CONNECTOR_WIDTH)),
            )
            .style(move |_: &Theme| container::Style {
                background: Some(tint.into()),
                ..container::Style::default()
            }),
        )
        .height(Fill)
        .align_y(iced::alignment::Vertical::Center)
        .into(),
    };

    let cell = container(carried)
        .width(Length::Fixed(DIVIDER_WIDTH))
        .height(Length::Fixed(ROW_HEIGHT));

    if !lit {
        return cell.into();
    }

    stack![cell]
        .push(container(rule(true)).center_x(Length::Fixed(DIVIDER_WIDTH)))
        .into()
}

/// The strip beside the graph, in the colour of the lane its row sits on, which is also the
/// divider between the graph and the messages.
fn edge_cell(tint: Option<Color>) -> Element<'static, Message> {
    container(blank_row())
        .style(move |theme: &Theme| container::Style {
            background: Some(
                tint.unwrap_or_else(|| theme.extended_palette().background.strong.color)
                    .into(),
            ),
            ..container::Style::default()
        })
        .into()
}

fn blank_row() -> Element<'static, Message> {
    Space::new()
        .width(Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
}

fn commit_style(theme: &Theme, selected: bool, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match (selected, status) {
        (true, _) => Some(palette.background.strong.color.into()),
        (false, button::Status::Hovered | button::Status::Pressed) => {
            Some(palette.background.weak.color.into())
        }
        (false, _) => None,
    };

    button::Style {
        background,
        text_color: palette.background.base.text,
        ..button::Style::default()
    }
}

/// Setting the text to one line is not enough on its own: it still paints past its own
/// bounds and over the column beside it.
fn cell(label: iced::widget::Text<'_>) -> Element<'_, Message> {
    container(clipped(label)).width(Fill).clip(true).into()
}

/// The working tree if there is one, then the commits, with a date separator wherever the
/// bucket changes. Each entry is one slot of ROW_HEIGHT.
fn build_slots(commits: &[read::CommitSummary], dirty: bool) -> (Vec<Slot>, Vec<f32>) {
    let mut slots = Vec::with_capacity(commits.len() + 1);
    let mut tops = Vec::with_capacity(commits.len());

    if dirty {
        slots.push(Slot::WorkingTree);
    }

    let dated = commits.len() > SEPARATED_ABOVE;
    let today = when::today();
    let mut bucket: Option<String> = None;

    for (index, commit) in commits.iter().enumerate() {
        if dated && let Some(days) = when::days_ago(&commit.when, today) {
            let label = when::bucket(days);
            if bucket.as_deref() != Some(label.as_str()) {
                slots.push(Slot::Separator(label.clone()));
                bucket = Some(label);
            }
        }

        tops.push(slots.len() as f32 * ROW_HEIGHT);
        slots.push(Slot::Commit(index));
    }

    (slots, tops)
}

/// Worked out once when the history is read, because a portrait is drawn from the identity
/// rather than from the commit.
fn author_of(commit: &read::CommitSummary) -> graph::Author {
    let identity = avatar::Identity {
        name: commit.author.name.clone(),
        email: commit.author.email.clone(),
    };

    graph::Author {
        fingerprint: identity.fingerprint(),
        generated: avatar::generated(&identity),
    }
}

/// The break between one run of dates and the next, with the date at the right end of the
/// row and out of the way of the messages.
fn separator_row(label: &str) -> Element<'_, Message> {
    container(
        row![
            container(divider())
                .width(Fill)
                .height(Length::Fixed(ROW_HEIGHT))
                .align_y(iced::alignment::Vertical::Center),
            container(text(label).size(SMALL).style(text::secondary))
                .padding([1, 6])
                .style(|theme: &Theme| container::Style {
                    background: Some(theme.extended_palette().background.strong.color.into()),
                    border: iced::Border {
                        radius: 3.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .height(Length::Fixed(ROW_HEIGHT))
    .padding([0, CELL_PAD])
    .into()
}

/// The label column on a row that carries none, so what is below it keeps its place.
fn blank_cell(width: f32) -> Element<'static, Message> {
    Space::new()
        .width(Length::Fixed(width))
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
}
