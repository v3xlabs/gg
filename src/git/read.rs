use super::command;
use gix::ObjectId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Head {
    pub name: Option<String>,
    pub id: Option<ObjectId>,
}

#[derive(Clone)]
pub struct Worktree {
    pub name: String,
    pub branch: Option<String>,
}

#[derive(Clone)]
pub struct Reference {
    pub name: String,
    pub target: ObjectId,
}

#[derive(Clone, Default)]
pub struct References {
    pub local_branches: Vec<Reference>,
    pub remote_branches: Vec<Reference>,
    pub tags: Vec<Reference>,
    pub stashes: Vec<Reference>,
    /// The commit each pull request points at, by its number. Only the ones git has been
    /// told to fetch are here: `refs/pull/*/head` on GitHub, `refs/merge-requests/*/head`
    /// on GitLab. Nothing here reaches a forge to ask.
    pub pulls: HashMap<ObjectId, u32>,
}

pub enum SigningFormat {
    OpenPgp,
    Ssh,
    X509,
    Unrecognised(String),
}

pub struct Signing {
    pub signs_commits: bool,
    pub format: SigningFormat,
    pub key: Option<String>,
    /// Set from `SSH_AUTH_SOCK`. A window started from a desktop entry does not always
    /// inherit it, and an ssh signature cannot be made without it.
    pub ssh_agent: Option<PathBuf>,
}

#[derive(Clone)]
pub struct CommitSummary {
    pub id: ObjectId,
    pub parent: Option<ObjectId>,
    pub title: String,
    pub body: String,
    pub author: Signature,
    pub committer: Signature,
    /// The author date as `YYYY-MM-DD`.
    pub when: String,
}

/// git records two of these per commit, and they differ more often than people expect: a
/// rebase, a patch applied from a mailing list, a commit made on someone else's behalf.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the epoch, and the seconds east of UTC the clock was set to.
    pub seconds: i64,
    pub offset: i32,
}

#[derive(Clone, Default)]
pub struct Status {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
}

pub fn root(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    workdir(&gix::discover(path)?)
}

pub fn workdir(repository: &gix::Repository) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let workdir = repository.workdir().ok_or("this is a bare repository")?;

    Ok(std::path::absolute(workdir)?)
}

pub fn head(repository: &gix::Repository) -> Result<Head, Box<dyn std::error::Error>> {
    let head = repository.head()?;

    Ok(Head {
        name: head.referent_name().map(|name| name.shorten().to_string()),
        id: head.id().map(|id| id.detach()),
    })
}

pub fn references(repository: &gix::Repository) -> Result<References, Box<dyn std::error::Error>> {
    let platform = repository.references()?;
    let mut references = References::default();

    for reference in platform.all()?.filter_map(Result::ok) {
        let Ok(target) = reference.clone().into_fully_peeled_id() else {
            continue;
        };
        let full = reference.name().as_bstr().to_string();
        let Some((category, short)) = reference.name().category_and_short_name() else {
            continue;
        };

        let entry = Reference {
            name: short.to_string(),
            target: target.detach(),
        };

        match category {
            gix::reference::Category::LocalBranch => references.local_branches.push(entry),
            gix::reference::Category::RemoteBranch => references.remote_branches.push(entry),
            gix::reference::Category::Tag => references.tags.push(entry),
            _ => {
                if let Some(number) = pull_number(&full) {
                    references.pulls.insert(entry.target, number);
                }
            }
        }
    }

    references.stashes = stashes(repository);
    Ok(references)
}

/// The number in `refs/pull/12/head` or `refs/merge-requests/12/head`. Both forges keep
/// the same shape, and both only exist locally once someone has fetched them.
fn pull_number(full: &str) -> Option<u32> {
    let rest = full
        .strip_prefix("refs/pull/")
        .or_else(|| full.strip_prefix("refs/merge-requests/"))?;
    let (number, tail) = rest.split_once('/')?;

    (tail == "head").then(|| number.parse().ok())?
}

/// The linked worktrees only: the main worktree is the repository itself.
pub fn worktrees(repository: &gix::Repository) -> Vec<Worktree> {
    let Ok(found) = repository.worktrees() else {
        return Vec::new();
    };

    found
        .into_iter()
        .map(|proxy| Worktree {
            name: proxy.id().to_string(),
            branch: proxy
                .into_repo_with_possibly_inaccessible_worktree()
                .ok()
                .and_then(|repository| head(&repository).ok())
                .and_then(|head| head.name),
        })
        .collect()
}

pub fn signing(repository: &gix::Repository) -> Signing {
    let config = repository.config_snapshot();

    let format = match config.string("gpg.format") {
        None => SigningFormat::OpenPgp,
        Some(value) => match value.as_slice() {
            b"openpgp" => SigningFormat::OpenPgp,
            b"ssh" => SigningFormat::Ssh,
            b"x509" => SigningFormat::X509,
            other => SigningFormat::Unrecognised(String::from_utf8_lossy(other).into_owned()),
        },
    };

    Signing {
        signs_commits: config.boolean("commit.gpgSign").unwrap_or(false),
        format,
        key: config
            .string("user.signingKey")
            .map(|key| String::from_utf8_lossy(key.as_slice()).into_owned()),
        ssh_agent: std::env::var_os("SSH_AUTH_SOCK").map(PathBuf::from),
    }
}

pub fn status(repository: &gix::Repository) -> Result<Status, Box<dyn std::error::Error>> {
    let mut status = Status::default();

    let iterator = repository
        .status(gix::progress::Discard)?
        // gix collapses an untracked directory into a single entry. A file list has to
        // name the files, so ask for each one.
        .index_worktree_options_mut(|options| {
            if let Some(dirwalk) = options.dirwalk_options.as_mut() {
                dirwalk.set_emit_untracked(gix::dir::walk::EmissionMode::Matching);
            }
        })
        .index_worktree_submodules(gix::status::Submodule::AsConfigured { check_dirty: false })
        .into_iter(None)?;

    for item in iterator {
        let item = item?;
        let path = item.location().to_string();

        match &item {
            gix::status::Item::TreeIndex(change) => {
                use gix::diff::index::Change;

                let kind = match change {
                    Change::Addition { .. } => ChangeKind::Added,
                    Change::Deletion { .. } => ChangeKind::Deleted,
                    Change::Modification { .. } => ChangeKind::Modified,
                    Change::Rewrite { .. } => ChangeKind::Renamed,
                };

                status.staged.push(FileChange {
                    path,
                    kind,
                    additions: 0,
                    deletions: 0,
                });
            }
            gix::status::Item::IndexWorktree(change) => {
                use gix::status::index_worktree::Item;

                let kind = match change {
                    Item::DirectoryContents { .. } => ChangeKind::Added,
                    Item::Rewrite { .. } => ChangeKind::Renamed,
                    // An untracked file reads as an addition and a missing one as a
                    // deletion, rather than calling every worktree change a modification.
                    Item::Modification { status, .. } => {
                        use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

                        match status {
                            EntryStatus::Change(Change::Removed) => ChangeKind::Deleted,
                            EntryStatus::IntentToAdd => ChangeKind::Added,
                            _ => ChangeKind::Modified,
                        }
                    }
                };

                status.unstaged.push(FileChange {
                    path,
                    kind,
                    additions: 0,
                    deletions: 0,
                });
            }
        }
    }

    status
        .staged
        .sort_by(|left, right| left.path.cmp(&right.path));
    status
        .unstaged
        .sort_by(|left, right| left.path.cmp(&right.path));

    if let Some(workdir) = repository.workdir() {
        let git = command::Git::in_work_dir(workdir);

        if let Ok(numstat) = git.worktree_numstat(false) {
            apply_counts(&mut status.unstaged, &numstat);
        }
        if let Ok(numstat) = git.worktree_numstat(true) {
            apply_counts(&mut status.staged, &numstat);
        }
    }

    Ok(status)
}

impl std::fmt::Display for SigningFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenPgp => formatter.write_str("openpgp"),
            Self::Ssh => formatter.write_str("ssh"),
            Self::X509 => formatter.write_str("x509"),
            Self::Unrecognised(value) => write!(formatter, "{value} (not recognised)"),
        }
    }
}

fn signature(
    signature: gix::actor::SignatureRef<'_>,
) -> Result<Signature, Box<dyn std::error::Error>> {
    let time = signature.time()?;

    Ok(Signature {
        name: signature.name.to_string(),
        email: signature.email.to_string(),
        seconds: time.seconds,
        offset: time.offset,
    })
}

pub fn summary(
    repository: &gix::Repository,
    id: ObjectId,
) -> Result<CommitSummary, Box<dyn std::error::Error>> {
    let commit = repository.find_commit(id)?;
    let message = commit.message()?;
    let author = commit.author()?;
    let committer = commit.committer()?;

    Ok(CommitSummary {
        id,
        parent: commit.parent_ids().next().map(|parent| parent.detach()),
        title: message.summary().to_string(),
        body: message
            .body()
            .map(|body| body.to_string())
            .unwrap_or_default(),
        when: author.time()?.format(gix::date::time::format::SHORT)?,
        author: signature(author)?,
        committer: signature(committer)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
}

/// What git compares a root commit to, since it has no parent to diff against.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    pub additions: usize,
    pub deletions: usize,
}

/// A binary file is reported as `-\t-\t<path>`, and stays at zero here.
fn apply_counts(changes: &mut [FileChange], numstat: &str) {
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    let mut records = numstat.split('\0');

    while let Some(record) = records.next() {
        let Some((additions, rest)) = record.split_once('\t') else {
            continue;
        };
        let Some((deletions, path)) = rest.split_once('\t') else {
            continue;
        };

        // A rename leaves the path field empty and puts the two paths in the records
        // that follow, source first. Only the destination is a path gix reported.
        let path = if path.is_empty() {
            let (_source, Some(destination)) = (records.next(), records.next()) else {
                break;
            };
            destination
        } else {
            path
        };

        counts.insert(
            path,
            (
                additions.parse().unwrap_or_default(),
                deletions.parse().unwrap_or_default(),
            ),
        );
    }

    for change in changes {
        if let Some((additions, deletions)) = counts.get(change.path.as_str()) {
            change.additions = *additions;
            change.deletions = *deletions;
        }
    }
}

/// What happened between two commits, which is what a run of them selected together adds
/// up to. `from` is the older side; `None` is the empty tree, for a commit with no parent.
pub fn between(
    repository: &gix::Repository,
    from: Option<ObjectId>,
    id: ObjectId,
) -> Result<Vec<FileChange>, Box<dyn std::error::Error>> {
    let commit = repository.find_commit(id)?;
    let tree = commit.tree()?;
    let parent_tree = match from {
        Some(parent) => repository.find_commit(parent)?.tree()?,
        None => repository.empty_tree(),
    };

    let mut changes = Vec::new();
    parent_tree
        .changes()?
        .for_each_to_obtain_tree(&tree, |change| {
            use gix::object::tree::diff::Change;

            let (location, mode, kind) = match &change {
                Change::Addition {
                    location,
                    entry_mode,
                    ..
                } => (location, entry_mode, ChangeKind::Added),
                Change::Deletion {
                    location,
                    entry_mode,
                    ..
                } => (location, entry_mode, ChangeKind::Deleted),
                Change::Modification {
                    location,
                    entry_mode,
                    ..
                } => (location, entry_mode, ChangeKind::Modified),
                Change::Rewrite {
                    location,
                    entry_mode,
                    ..
                } => (location, entry_mode, ChangeKind::Renamed),
            };

            // The walk reports the directories it descends through as well as the blobs
            // inside them. Only the blobs are files the reader changed.
            if !mode.is_tree() {
                changes.push(FileChange {
                    path: location.to_string(),
                    kind,
                    additions: 0,
                    deletions: 0,
                });
            }

            Ok::<_, std::convert::Infallible>(gix::object::tree::diff::Action::Continue(()))
        })?;

    changes.sort_by(|left, right| left.path.cmp(&right.path));

    if let Some(workdir) = repository.workdir() {
        let base = from.map_or_else(|| EMPTY_TREE.to_owned(), |parent| parent.to_string());

        if let Ok(numstat) =
            command::Git::in_work_dir(workdir).numstat(&base, Some(&id.to_string()))
        {
            apply_counts(&mut changes, &numstat);
        }
    }

    Ok(changes)
}

/// `refs/stash` names only the newest stash. The older ones are reachable through its
/// reflog, which is where the stash@{n} numbering comes from.
pub fn stashes(repository: &gix::Repository) -> Vec<Reference> {
    let Ok(reference) = repository.find_reference("refs/stash") else {
        return Vec::new();
    };

    let mut platform = reference.log_iter();
    let Ok(Some(entries)) = platform.all() else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| ObjectId::from_hex(entry.new_oid).ok())
        .enumerate()
        .map(|(index, target)| Reference {
            name: format!("stash@{{{index}}}"),
            target,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::history;
    use std::process::Command;

    fn changed(repository: &gix::Repository, id: ObjectId) -> Vec<FileChange> {
        let parent = repository
            .find_commit(id)
            .expect("the commit is there")
            .parent_ids()
            .next()
            .map(|parent| parent.detach());

        between(repository, parent, id).expect("read the changes")
    }

    fn git(path: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(path)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(path: &std::path::Path, relative: &str, contents: &str) {
        let file = path.join(relative);
        std::fs::create_dir_all(file.parent().expect("a parent directory")).expect("create dirs");
        std::fs::write(file, contents).expect("write the file");
    }

    fn kinds(changes: &[FileChange]) -> Vec<(&str, ChangeKind)> {
        changes
            .iter()
            .map(|change| (change.path.as_str(), change.kind))
            .collect()
    }

    #[test]
    fn changes_report_files_only_never_the_directories_holding_them() {
        let path = std::env::temp_dir().join(format!("gg-{}-changes", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the scratch directory");

        git(&path, &["init", "--quiet", "--initial-branch=main"]);
        write(&path, "top.txt", "one\n");
        write(&path, "a/b/deep.txt", "deep\n");
        git(&path, &["add", "."]);
        git(&path, &["commit", "--quiet", "-m", "first"]);

        write(&path, "top.txt", "two\n");
        write(&path, "a/b/second.txt", "second\n");
        std::fs::remove_file(path.join("a/b/deep.txt")).expect("remove the file");
        git(&path, &["add", "-A"]);
        git(&path, &["commit", "--quiet", "-m", "second"]);

        let repository = gix::discover(&path).expect("open the scratch repository");
        let walk = history::walk(&repository, 10).expect("walk the history");
        assert_eq!(walk.len(), 2);

        let second = changed(&repository, walk[0].commit);
        assert_eq!(
            kinds(&second),
            [
                ("a/b/deep.txt", ChangeKind::Deleted),
                ("a/b/second.txt", ChangeKind::Added),
                ("top.txt", ChangeKind::Modified),
            ]
        );

        let first = changed(&repository, walk[1].commit);
        assert_eq!(
            kinds(&first),
            [
                ("a/b/deep.txt", ChangeKind::Added),
                ("top.txt", ChangeKind::Added),
            ],
            "an added directory must still yield the files inside it"
        );

        std::fs::remove_dir_all(&path).expect("clean up");
    }
}
