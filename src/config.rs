use crate::ui::theme;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Deeper than this under a configured directory is a vendored checkout of somebody else's
/// tree, not a repository the user meant to list.
const SCAN_DEPTH: usize = 4;
const NEVER_SCANNED: [&str; 5] = ["node_modules", "target", ".direnv", "result", ".git"];
const NEVER_AN_ICON: [&str; 8] = [
    "dist",
    "out",
    "build",
    "coverage",
    ".next",
    ".svelte-kit",
    ".output",
    "vendor",
];
const STORE: &str = "/nix/store";

/// `public/`, `static/`, `assets/img/` and `.github/` all sit within this of a root.
const ICON_DEPTH: usize = 3;
const ICON_LIMIT: usize = 24;
const ICON_EXTENSIONS: [&str; 6] = ["ico", "png", "svg", "jpg", "jpeg", "webp"];
const ICON_STEMS: [&str; 6] = [
    "favicon",
    "icon",
    "logo",
    "apple-touch-icon",
    "app-icon",
    "avatar",
];

#[derive(Debug)]
pub enum Error {
    Read(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
    Write(PathBuf, std::io::Error),
    Encode(toml::ser::Error),
    NoStateDirectory,
    Unreadable,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(path, error) => {
                write!(formatter, "could not read {}: {error}", path.display())
            }
            Self::Parse(path, error) => {
                write!(formatter, "could not parse {}: {error}", path.display())
            }
            Self::Write(path, error) => {
                write!(formatter, "could not write {}: {error}", path.display())
            }
            Self::Encode(error) => write!(formatter, "could not encode the state: {error}"),
            Self::Unreadable => write!(
                formatter,
                "the state file could not be read, so it is left as it is rather than written over"
            ),
            Self::NoStateDirectory => {
                write!(
                    formatter,
                    "neither XDG_DATA_HOME nor HOME is set, so there is nowhere to keep the state"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub theme: theme::Choice,
    pub paths: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: theme::Choice::System,
            paths: Vec::new(),
            directories: Vec::new(),
        }
    }
}

/// The name every file this program keeps is filed under. It is not the name of the
/// program: changing it would strand the configuration and repository list of everyone
/// who ran an earlier one.
pub const ON_DISK_NAME: &str = "gitgui";

impl Config {
    pub fn load() -> Result<Self, Error> {
        let home = home();
        let mut files = vec![PathBuf::from("/etc").join(ON_DISK_NAME).join("config.toml")];
        if let Some(directory) = config_home(home.as_deref()) {
            files.push(directory.join(ON_DISK_NAME).join("config.toml"));
        }

        Self::layered(&files, home.as_deref())
    }

    /// Later files win key by key: one that sets only `[ui] theme` leaves the earlier
    /// file's repositories alone.
    fn layered(files: &[PathBuf], home: Option<&Path>) -> Result<Self, Error> {
        let mut merged = Layer::default();
        for file in files {
            merged.absorb(read_layer(file)?);
        }

        let expand = |paths: Vec<PathBuf>| paths.iter().map(|path| expand(path, home)).collect();

        Ok(Self {
            theme: merged.ui.theme.unwrap_or(theme::Choice::System),
            paths: merged.repositories.paths.map_or_else(Vec::new, expand),
            directories: merged
                .repositories
                .directories
                .map_or_else(Vec::new, expand),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    #[serde(default)]
    ui: UiLayer,
    #[serde(default)]
    repositories: RepositoriesLayer,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiLayer {
    theme: Option<theme::Choice>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoriesLayer {
    paths: Option<Vec<PathBuf>>,
    directories: Option<Vec<PathBuf>>,
}

impl Layer {
    fn absorb(&mut self, later: Self) {
        self.ui.theme = later.ui.theme.or(self.ui.theme);
        self.repositories.paths = later.repositories.paths.or(self.repositories.paths.take());
        self.repositories.directories = later
            .repositories
            .directories
            .or(self.repositories.directories.take());
    }
}

fn read_layer(file: &Path) -> Result<Layer, Error> {
    match std::fs::read_to_string(file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Layer::default()),
        Err(error) => Err(Error::Read(file.to_owned(), error)),
        Ok(text) => toml::from_str(&text).map_err(|error| Error::Parse(file.to_owned(), error)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Icon {
    pub repository: PathBuf,
    /// Relative to `repository`, so the icon survives the checkout being moved.
    pub file: PathBuf,
}

/// A value this build has never heard of must not throw away the rest of the file.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = toml::Value::deserialize(deserializer)?;

    Ok(value.try_into().ok())
}

/// Kept apart from the configuration so a file a Nix module wrote is never rewritten here.
/// The scalar fields have to precede the arrays, or the TOML this serialises to puts keys
/// after the table they no longer belong to. Unknown fields are accepted so an older binary
/// still opens a file a newer one wrote.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub last_opened: Option<PathBuf>,
    #[serde(default, deserialize_with = "lenient")]
    pub theme: Option<theme::Choice>,
    #[serde(default, deserialize_with = "lenient")]
    pub scale: Option<f32>,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub directories: Vec<PathBuf>,
    #[serde(default)]
    pub columns: crate::ui::Columns,
    #[serde(default)]
    pub widths: crate::ui::Widths,
    #[serde(default)]
    pub icons: Vec<Icon>,
    /// Writing over a file nobody could parse would throw away a reader's repositories to
    /// save a window position.
    #[serde(skip)]
    unreadable: bool,
}

impl State {
    pub fn load() -> Self {
        match state_file(home().as_deref()) {
            None => Self::default(),
            Some(file) => Self::read(&file),
        }
    }

    /// A downgrade, a half-finished migration or a write cut short by a crash must not stop
    /// the window from opening, so anything unreadable is reported and then left behind.
    fn read(file: &Path) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                eprintln!("gg: {}", Error::Read(file.to_owned(), error));
                return Self::default();
            }
        };

        match toml::from_str(&text) {
            Ok(state) => state,
            Err(error) => {
                eprintln!(
                    "gg: {}, so this run starts from the defaults and leaves it alone",
                    Error::Parse(file.to_owned(), error)
                );
                Self {
                    unreadable: true,
                    ..Self::default()
                }
            }
        }
    }

    pub fn save(&self) -> Result<(), Error> {
        if self.unreadable {
            return Err(Error::Unreadable);
        }

        let file = state_file(home().as_deref()).ok_or(Error::NoStateDirectory)?;
        let text = toml::to_string(self).map_err(Error::Encode)?;

        if let Some(directory) = file.parent()
            && let Err(error) = std::fs::create_dir_all(directory)
        {
            return Err(Error::Write(directory.to_owned(), error));
        }

        std::fs::write(&file, text).map_err(|error| Error::Write(file, error))
    }

    pub fn icon(&self, repository: &Path) -> Option<PathBuf> {
        self.icons
            .iter()
            .find(|icon| icon.repository == repository)
            .map(|icon| repository.join(&icon.file))
    }

    pub fn set_icon(&mut self, repository: &Path, file: PathBuf) {
        self.icons.retain(|icon| icon.repository != repository);
        self.icons.push(Icon {
            repository: repository.to_owned(),
            file,
        });
    }
}

/// Explicit entries first and in the order they were written, then whatever the scan turns
/// up.
pub fn repositories(config: &Config, state: &State) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut found: Vec<PathBuf> = config
        .paths
        .iter()
        .chain(&state.paths)
        .filter(|path| seen.insert((*path).clone()))
        .cloned()
        .collect();

    for directory in &config.directories {
        for path in scan(directory) {
            if seen.insert(path.clone()) {
                found.push(path);
            }
        }
    }

    found
}

pub fn scan(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(directory, 0, &mut found);
    found
}

/// The files that look like they are meant to stand for the project, relative to the
/// repository root. Bounded the way [`scan`] is, so a deep tree cannot stall the picker.
pub fn icon_files(repository: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_for_icons(repository, repository, 0, &mut found);
    found
}

fn walk_for_icons(root: &Path, directory: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > ICON_DEPTH || found.len() >= ICON_LIMIT || directory.starts_with(STORE) {
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    let (mut files, mut children) = (Vec::new(), Vec::new());
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if NEVER_SCANNED.contains(&name.as_str()) || NEVER_AN_ICON.contains(&name.as_str()) {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => children.push(entry.path()),
            Ok(kind) if kind.is_file() && looks_like_an_icon(&name) => files.push(entry.path()),
            _ => {}
        }
    }
    files.sort();
    children.sort();

    for file in files {
        if found.len() >= ICON_LIMIT {
            return;
        }
        if let Ok(relative) = file.strip_prefix(root) {
            found.push(relative.to_owned());
        }
    }

    for child in children {
        walk_for_icons(root, &child, depth + 1, found);
    }
}

fn looks_like_an_icon(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    let stem = stem.to_ascii_lowercase();

    ICON_EXTENSIONS.contains(&extension.as_str())
        && ICON_STEMS.iter().any(|start| stem.starts_with(start))
}

fn walk(directory: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > SCAN_DEPTH || directory.starts_with(STORE) {
        return;
    }
    if directory.join(".git").exists() {
        found.push(directory.to_owned());
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    let mut children: Vec<PathBuf> = entries
        .flatten()
        // `DirEntry::file_type` reports the symlink itself rather than its target, so a
        // link back up the tree cannot turn the walk into a loop.
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| !NEVER_SCANNED.contains(&entry.file_name().to_string_lossy().as_ref()))
        .map(|entry| entry.path())
        .collect();
    children.sort();

    for child in children {
        walk(&child, depth + 1, found);
    }
}

pub fn expanded(path: &str) -> PathBuf {
    expand(Path::new(path), home().as_deref())
}

fn expand(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_owned(),
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn config_home(home: Option<&Path>) -> Option<PathBuf> {
    directory("XDG_CONFIG_HOME").or_else(|| home.map(|home| home.join(".config")))
}

fn state_file(home: Option<&Path>) -> Option<PathBuf> {
    directory("XDG_DATA_HOME")
        .or_else(|| home.map(|home| home.join(".local/share")))
        .map(|data| data.join(ON_DISK_NAME).join("state.toml"))
}

fn directory(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is before the epoch")
            .as_nanos();
        let path = PathBuf::from(".tmp/config-tests").join(format!("{name}-{unique}"));
        std::fs::create_dir_all(&path).expect("the scratch directory could be created");
        path
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("the file has a parent"))
            .expect("the parent directory could be created");
        std::fs::write(path, text).expect("the file could be written");
    }

    fn repository(path: &Path) {
        std::fs::create_dir_all(path.join(".git")).expect("the repository could be created");
    }

    #[test]
    fn a_theme_this_build_does_not_know_keeps_the_rest_of_the_state() {
        let state: State = toml::from_str(
            r#"
            theme = "some-theme-from-a-later-build"
            paths = ["/home/someone/work"]
            "#,
        )
        .expect("an unknown theme is not a reason to lose the repositories");

        assert_eq!(state.theme, None);
        assert_eq!(state.paths, [PathBuf::from("/home/someone/work")]);
    }

    #[test]
    fn missing_files_leave_the_defaults() {
        let directory = scratch("missing");
        let config = Config::layered(&[directory.join("nothing.toml")], None)
            .expect("a missing file is not an error");

        assert_eq!(config, Config::default());
        std::fs::remove_dir_all(&directory).expect("the scratch directory could be removed");
    }

    #[test]
    fn the_later_file_wins_key_by_key() {
        let directory = scratch("layering");
        let system = directory.join("etc.toml");
        let user = directory.join("user.toml");
        write(
            &system,
            "[ui]\ntheme = \"dark\"\n[repositories]\npaths = [\"/srv/one\"]\ndirectories = [\"/srv\"]\n",
        );
        write(
            &user,
            "[ui]\ntheme = \"light\"\n[repositories]\npaths = [\"/home/two\"]\n",
        );

        let config = Config::layered(&[system, user], None).expect("both files parse");

        assert_eq!(config.theme, theme::Choice::Light);
        assert_eq!(config.paths, [PathBuf::from("/home/two")]);
        assert_eq!(config.directories, [PathBuf::from("/srv")]);
        std::fs::remove_dir_all(&directory).expect("the scratch directory could be removed");
    }

    #[test]
    fn an_unparsable_file_names_itself() {
        let directory = scratch("broken");
        let file = directory.join("config.toml");
        write(&file, "[ui]\ntheme = \"chartreuse\"\n");

        let error = Config::layered(std::slice::from_ref(&file), None)
            .expect_err("the theme is not a choice");

        assert!(matches!(&error, Error::Parse(path, _) if *path == file));
        assert!(error.to_string().contains("config.toml"));
        std::fs::remove_dir_all(&directory).expect("the scratch directory could be removed");
    }

    #[test]
    fn a_leading_tilde_becomes_the_home_directory() {
        let home = Path::new("/home/someone");

        assert_eq!(expand(Path::new("~/dev"), Some(home)), home.join("dev"));
        assert_eq!(expand(Path::new("~"), Some(home)), home);
        assert_eq!(
            expand(Path::new("~other/dev"), Some(home)),
            PathBuf::from("~other/dev")
        );
        assert_eq!(
            expand(Path::new("/srv/dev"), Some(home)),
            PathBuf::from("/srv/dev")
        );
        assert_eq!(expand(Path::new("~/dev"), None), PathBuf::from("~/dev"));
    }

    #[test]
    fn configured_paths_are_expanded() {
        let directory = scratch("expansion");
        let file = directory.join("config.toml");
        write(
            &file,
            "[repositories]\npaths = [\"~/code/one\"]\ndirectories = [\"~/work\"]\n",
        );

        let config =
            Config::layered(&[file], Some(Path::new("/home/someone"))).expect("the file parses");

        assert_eq!(config.paths, [PathBuf::from("/home/someone/code/one")]);
        assert_eq!(config.directories, [PathBuf::from("/home/someone/work")]);
        std::fs::remove_dir_all(&directory).expect("the scratch directory could be removed");
    }

    #[test]
    fn the_scan_stops_at_a_repository_and_skips_the_noise() {
        let root = scratch("scan");
        repository(&root.join("alpha"));
        repository(&root.join("alpha/vendor/inner"));
        repository(&root.join("group/beta"));
        repository(&root.join("group/beta/node_modules/package"));
        repository(&root.join("loose/node_modules/package"));
        repository(&root.join("loose/target/debug/checkout"));
        repository(&root.join("one/two/three/four/five/deep"));
        std::fs::create_dir_all(root.join("empty")).expect("the directory could be created");

        let found = scan(&root);

        assert_eq!(found, [root.join("alpha"), root.join("group/beta")]);
        std::fs::remove_dir_all(&root).expect("the scratch directory could be removed");
    }

    #[test]
    fn a_configured_directory_that_is_itself_a_repository_is_one_entry() {
        let root = scratch("itself");
        repository(&root);
        repository(&root.join("sub"));

        assert_eq!(scan(&root), [root.clone()].as_slice());
        std::fs::remove_dir_all(&root).expect("the scratch directory could be removed");
    }

    #[test]
    fn a_state_file_a_newer_binary_wrote_still_opens() {
        let directory = scratch("newer-state");
        let file = directory.join("state.toml");
        write(
            &file,
            "last_opened = \"/srv/one\"\npaths = [\"/srv/one\"]\nfrom_the_future = 7\n",
        );

        let state = State::read(&file);

        assert_eq!(state.last_opened, Some(PathBuf::from("/srv/one")));
        assert_eq!(state.paths, [PathBuf::from("/srv/one")]);
        std::fs::remove_dir_all(&directory).expect("the scratch directory could be removed");
    }

    #[test]
    fn a_state_file_that_is_not_toml_falls_back_to_the_defaults() {
        let directory = scratch("broken-state");
        let file = directory.join("state.toml");
        write(&file, "last_opened = \"/srv/one\"\npaths = [\"/srv");

        let state = State::read(&file);

        assert_eq!(state.last_opened, None);
        assert!(state.paths.is_empty());
        assert!(
            matches!(state.save(), Err(Error::Unreadable)),
            "a file nobody could parse still holds someone's repositories, and writing over it would drop them"
        );
        std::fs::remove_dir_all(&directory).expect("the scratch directory could be removed");
    }

    #[test]
    fn the_icon_scan_finds_the_conventional_places_and_nothing_else() {
        let root = scratch("icons");
        write(&root.join("favicon.ico"), "");
        write(&root.join("public/logo-dark.svg"), "");
        write(&root.join(".github/apple-touch-icon.png"), "");
        write(&root.join("src/main.rs"), "");
        write(&root.join("docs/screenshot.png"), "");
        write(&root.join("node_modules/thing/logo.png"), "");
        write(&root.join("one/two/three/four/icon.png"), "");

        assert_eq!(
            icon_files(&root),
            [
                PathBuf::from("favicon.ico"),
                PathBuf::from(".github/apple-touch-icon.png"),
                PathBuf::from("public/logo-dark.svg"),
            ]
        );
        std::fs::remove_dir_all(&root).expect("the scratch directory could be removed");
    }

    #[test]
    fn an_icon_is_remembered_against_its_repository() {
        let mut state = State::default();
        state.set_icon(Path::new("/srv/one"), PathBuf::from("public/logo.svg"));
        state.set_icon(Path::new("/srv/one"), PathBuf::from("favicon.png"));

        assert_eq!(state.icons.len(), 1);
        assert_eq!(
            state.icon(Path::new("/srv/one")),
            Some(PathBuf::from("/srv/one/favicon.png"))
        );
        assert_eq!(state.icon(Path::new("/srv/two")), None);
    }

    #[test]
    fn explicit_paths_come_first_and_repeats_are_dropped() {
        let root = scratch("dedupe");
        repository(&root.join("alpha"));
        repository(&root.join("beta"));

        let config = Config {
            theme: theme::Choice::System,
            paths: vec![root.join("beta")],
            directories: vec![root.clone()],
        };
        let state = State {
            paths: vec![root.join("gamma"), root.join("beta")],
            ..State::default()
        };

        assert_eq!(
            repositories(&config, &state),
            [root.join("beta"), root.join("gamma"), root.join("alpha"),]
        );
        std::fs::remove_dir_all(&root).expect("the scratch directory could be removed");
    }
}
