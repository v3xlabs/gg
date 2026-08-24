use std::collections::HashSet;
use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const TIGHT_CONTEXT: &str = "-U3";
/// Larger than any file worth reading in a window, so git reports the whole file.
const WHOLE_FILE_CONTEXT: &str = "-U1000000";

/// The only place in gg that spawns `git`. Everything that changes a repository goes
/// through here, so the user's hooks, credential helpers, and signing keep working.
pub struct Git {
    work_dir: PathBuf,
}

#[derive(Debug)]
pub enum Error {
    Spawn(std::io::Error),
    Failed { code: Option<i32>, stderr: String },
    NotUtf8,
}

impl Git {
    pub fn in_work_dir(work_dir: impl Into<PathBuf>) -> Self {
        Self {
            work_dir: work_dir.into(),
        }
    }

    pub fn version(&self) -> Result<String, Error> {
        self.run(&["--version"])
    }

    /// Covers a file that was deleted or never tracked as well.
    pub fn add(&self, path: &str) -> Result<(), Error> {
        self.run(&["add", "--", path]).map(drop)
    }

    /// Every change in the work tree, deletions and untracked files included.
    pub fn add_all(&self) -> Result<(), Error> {
        self.run(&["add", "--all"]).map(drop)
    }

    /// Two `--message` arguments are what git turns into a title and a body with a blank
    /// line between them, which is the shape every other tool expects.
    pub fn commit(&self, message: &str, description: &str) -> Result<(), Error> {
        let mut args = vec!["commit", "--message", message];
        if !description.is_empty() {
            args.push("--message");
            args.push(description);
        }

        self.run(&args).map(drop)
    }

    /// Only the remote-tracking refs under `refs/remotes` move: no local branch, no commit
    /// and no working tree file is touched by this.
    pub fn fetch(&self, remotes: &[String]) -> Result<(), Error> {
        let mut args = vec!["fetch", "--multiple"];
        args.extend(remotes.iter().map(String::as_str));

        self.run(&args).map(drop)
    }

    pub fn push(&self, remote: &str, branch: &str, upstream: bool) -> Result<(), Error> {
        let mut args = vec!["push"];
        if upstream {
            args.push("--set-upstream");
        }
        args.push(remote);
        args.push(branch);

        self.run(&args).map(drop)
    }

    /// The commits on `branch` that no ref of `remote` reaches, which is what a push would
    /// send. `None` when git could not answer, so the dialog leaves the count off rather
    /// than claiming a number it does not have.
    pub fn unpushed(&self, remote: &str, branch: &str) -> Option<usize> {
        let scope = format!("--remotes={remote}");

        self.run(&["rev-list", "--count", branch, "--not", &scope])
            .ok()?
            .parse()
            .ok()
    }

    /// Creates the branch where HEAD is and moves onto it.
    pub fn branch(&self, name: &str) -> Result<(), Error> {
        self.run(&["checkout", "-b", name]).map(drop)
    }

    /// Which of these paths git is told to ignore. Asked in one call with the list on
    /// stdin, because `check-ignore` is the only thing that reads every `.gitignore`,
    /// `.git/info/exclude` and core.excludesFile the way git itself does.
    pub fn ignored(&self, paths: &[PathBuf]) -> HashSet<PathBuf> {
        if paths.is_empty() {
            return HashSet::new();
        }

        let mut input = Vec::new();
        for path in paths {
            input.extend_from_slice(path.as_os_str().as_encoded_bytes());
            input.push(0);
        }

        let Ok(mut child) = Command::new("git")
            .current_dir(&self.work_dir)
            .args(["check-ignore", "--stdin", "-z"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return HashSet::new();
        };

        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(&input);
        }
        drop(child.stdin.take());

        let Ok(output) = child.wait_with_output() else {
            return HashSet::new();
        };

        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|line| !line.is_empty())
            .map(|line| PathBuf::from(String::from_utf8_lossy(line).into_owned()))
            .collect()
    }

    /// Text for one path, as a unified diff. `--no-ext-diff` matters: a user with a
    /// difftool configured must not have it launched behind their back by a click.
    pub fn diff(&self, base: &str, target: Option<&str>, path: &str) -> Result<String, Error> {
        self.diff_with_context(TIGHT_CONTEXT, base, target, path)
    }

    /// The same diff with the whole file as context, so every line of the file arrives
    /// already tagged as added, removed or unchanged and nothing has to diff it again.
    pub fn whole_file_diff(
        &self,
        base: &str,
        target: Option<&str>,
        path: &str,
    ) -> Result<String, Error> {
        self.diff_with_context(WHOLE_FILE_CONTEXT, base, target, path)
    }

    /// Unstaged when `staged` is false, index against HEAD when it is true.
    pub fn worktree_diff(&self, path: &str, staged: bool) -> Result<String, Error> {
        self.worktree_diff_with_context(TIGHT_CONTEXT, path, staged)
    }

    /// The worktree counterpart of [`Git::whole_file_diff`].
    pub fn whole_file_worktree_diff(&self, path: &str, staged: bool) -> Result<String, Error> {
        self.worktree_diff_with_context(WHOLE_FILE_CONTEXT, path, staged)
    }

    fn diff_with_context(
        &self,
        context: &str,
        base: &str,
        target: Option<&str>,
        path: &str,
    ) -> Result<String, Error> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", context, base];
        if let Some(target) = target {
            args.push(target);
        }
        args.push("--");
        args.push(path);

        self.run_raw(&args)
    }

    fn worktree_diff_with_context(
        &self,
        context: &str,
        path: &str,
        staged: bool,
    ) -> Result<String, Error> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", context];
        if staged {
            args.push("--cached");
        }
        args.push("--");
        args.push(path);

        let diff = self.run_raw(&args)?;
        if !staged && diff.is_empty() && !self.is_tracked(path)? {
            return self.untracked_diff(context, path);
        }

        Ok(diff)
    }

    fn is_tracked(&self, path: &str) -> Result<bool, Error> {
        Ok(!self.run_raw(&["ls-files", "-z", "--", path])?.is_empty())
    }

    /// git has nothing to say about a path it does not track, so the file is diffed
    /// against nothing and every line of it comes back added.
    fn untracked_diff(&self, context: &str, path: &str) -> Result<String, Error> {
        let output = Command::new("git")
            .current_dir(&self.work_dir)
            .args([
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-index",
                context,
                "--",
                "/dev/null",
                path,
            ])
            .output()
            .map_err(Error::Spawn)?;

        // `--no-index` exits 1 when the two sides differ, which is the point of asking.
        // It also exits 1 when it cannot read the path, and then it says nothing.
        if !output.status.success() && output.stdout.is_empty() {
            return Err(Error::Failed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        String::from_utf8(output.stdout).map_err(|_| Error::NotUtf8)
    }

    /// Added and deleted line counts for every path the range touches, in one call.
    ///
    /// `-z` keeps the paths byte for byte; without it git quotes anything outside ascii
    /// and the path stops matching the one gix reported.
    pub fn numstat(&self, base: &str, target: Option<&str>) -> Result<String, Error> {
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--numstat",
            "-z",
            base,
        ];
        if let Some(target) = target {
            args.push(target);
        }

        self.run_raw(&args)
    }

    /// Unstaged when `staged` is false, index against HEAD when it is true.
    pub fn worktree_numstat(&self, staged: bool) -> Result<String, Error> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "--numstat", "-z"];
        if staged {
            args.push("--cached");
        }

        self.run_raw(&args)
    }

    fn run(&self, args: &[&str]) -> Result<String, Error> {
        let output = Command::new("git")
            .current_dir(&self.work_dir)
            .args(args)
            .output()
            .map_err(Error::Spawn)?;

        if !output.status.success() {
            return Err(Error::Failed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        String::from_utf8(output.stdout)
            .map(|stdout| stdout.trim().to_owned())
            .map_err(|_| Error::NotUtf8)
    }

    /// Like `run`, but keeps the output exactly as git produced it. A diff carries
    /// meaningful leading and trailing whitespace.
    fn run_raw(&self, args: &[&str]) -> Result<String, Error> {
        let output = Command::new("git")
            .current_dir(&self.work_dir)
            .args(args)
            .output()
            .map_err(Error::Spawn)?;

        if !output.status.success() {
            return Err(Error::Failed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        String::from_utf8(output.stdout).map_err(|_| Error::NotUtf8)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "could not run git: {error}"),
            Self::Failed {
                code: Some(code),
                stderr,
            } => write!(formatter, "git exited with {code}: {stderr}"),
            Self::Failed { code: None, stderr } => {
                write!(formatter, "git was killed by a signal: {stderr}")
            }
            Self::NotUtf8 => write!(formatter, "git printed something that is not utf-8"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asked of git itself rather than of the environment, because what matters is what
    /// git can reach, not which variables happen to be set.
    ///
    /// These tests commit, clone, fetch and push. On a developer's machine git would read
    /// their global configuration, and through it their signing key and their agent, so a
    /// test run could make a real key sign something. `scripts/test.sh` and
    /// `nix flake check` each run this binary in a sandbox where none of that exists.
    /// Anywhere else the tests stop here.
    fn sandboxed() {
        let global = Command::new("git")
            .args(["config", "--global", "--list"])
            .output()
            .expect("run git config");

        assert!(
            global.stdout.is_empty(),
            "git can read a global configuration, so this is not a sandbox. \
             Run scripts/test.sh or `nix flake check`.",
        );
        assert!(
            std::env::var_os("SSH_AUTH_SOCK").is_none(),
            "an ssh agent is reachable, so this is not a sandbox. \
             Run scripts/test.sh or `nix flake check`.",
        );

        let home = std::env::var_os("HOME").map(PathBuf::from);
        let occupied = home
            .as_deref()
            .and_then(|home| std::fs::read_dir(home).ok())
            .is_some_and(|mut entries| entries.next().is_some());

        assert!(
            !occupied,
            "the home directory {home:?} has something in it, so this is not a sandbox. \
             Run scripts/test.sh or `nix flake check`.",
        );
        assert!(
            !std::path::Path::new("/home").exists(),
            "/home exists, so a path from the machine reaches in here. \
             Run scripts/test.sh or `nix flake check`.",
        );
    }

    /// Under [`sandboxed`] the whole filesystem outside the store is throwaway, so the
    /// temporary directory is the right place for these and nothing survives the run.
    fn scratch_repository(name: &str) -> PathBuf {
        sandboxed();

        let path = std::env::temp_dir().join(format!("gg-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the scratch directory");

        let status = Command::new("git")
            .args(["init", "--quiet", "--initial-branch=main"])
            .current_dir(&path)
            .status()
            .expect("run git init");
        assert!(status.success(), "git init failed");

        path
    }

    #[test]
    fn reports_the_git_version() {
        let path = scratch_repository("version");

        let version = Git::in_work_dir(&path).version().expect("read the version");
        assert!(version.starts_with("git version"), "got {version:?}");

        std::fs::remove_dir_all(&path).expect("clean up");
    }

    #[test]
    fn an_untracked_file_reads_as_an_added_file() {
        let path = scratch_repository("untracked");
        std::fs::write(path.join("a.txt"), "hello\n").expect("write the file");

        let diff = Git::in_work_dir(&path)
            .worktree_diff("a.txt", false)
            .expect("read the diff");

        assert!(diff.contains("+hello"), "got {diff:?}");

        std::fs::remove_dir_all(&path).expect("clean up");
    }

    /// A sandbox has no global configuration, so the identity a commit needs is given here.
    fn git_in(path: &PathBuf, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.email=gg@test.invalid",
                "-c",
                "user.name=gg test",
            ])
            .args(args)
            .current_dir(path)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn commit_in(path: &PathBuf, file: &str, body: &str) {
        std::fs::write(path.join(file), body).expect("write the file");
        git_in(path, &["add", "--", file]);
        git_in(path, &["commit", "--quiet", "--message", body]);
    }

    fn head_of(path: &PathBuf) -> String {
        Git::in_work_dir(path)
            .run(&["rev-parse", "HEAD"])
            .expect("read HEAD")
    }

    /// A pair of repositories, the second cloned from the first, which is the shape every
    /// remote test below needs.
    fn cloned(name: &str) -> (PathBuf, PathBuf) {
        let origin = scratch_repository(name);
        commit_in(&origin, "a.txt", "one");

        let clone = std::env::temp_dir().join(format!("gg-{}-{name}-clone", std::process::id()));
        let _ = std::fs::remove_dir_all(&clone);
        let status = Command::new("git")
            .args(["clone", "--quiet"])
            .arg(&origin)
            .arg(&clone)
            .status()
            .expect("run git clone");
        assert!(status.success(), "git clone failed");

        (origin, clone)
    }

    #[test]
    fn a_fetch_moves_the_remote_ref_and_leaves_the_local_branch_where_it_was() {
        let (origin, clone) = cloned("fetch");
        let before = head_of(&clone);
        commit_in(&origin, "a.txt", "two");

        Git::in_work_dir(&clone)
            .fetch(&["origin".to_owned()])
            .expect("fetch");

        let git = Git::in_work_dir(&clone);
        assert_eq!(head_of(&clone), before, "the local branch moved");
        assert_eq!(
            git.run(&["rev-parse", "refs/remotes/origin/main"])
                .expect("read the remote ref"),
            head_of(&origin),
            "the remote ref did not move",
        );

        std::fs::remove_dir_all(&origin).expect("clean up");
        std::fs::remove_dir_all(&clone).expect("clean up");
    }

    #[test]
    fn a_push_sends_the_commits_the_remote_was_missing() {
        let (origin, clone) = cloned("push");
        // git refuses a push onto the branch a non-bare repository has checked out, and a
        // scratch repository is the only kind here.
        git_in(&origin, &["config", "receive.denyCurrentBranch", "ignore"]);
        commit_in(&clone, "b.txt", "mine");

        let git = Git::in_work_dir(&clone);
        assert_eq!(git.unpushed("origin", "main"), Some(1));

        git.push("origin", "main", false).expect("push");

        assert_eq!(git.unpushed("origin", "main"), Some(0));
        assert_eq!(
            git.run(&["rev-parse", "refs/remotes/origin/main"])
                .expect("read the remote ref"),
            head_of(&clone),
        );

        std::fs::remove_dir_all(&origin).expect("clean up");
        std::fs::remove_dir_all(&clone).expect("clean up");
    }

    #[test]
    fn a_new_branch_is_the_one_that_is_checked_out() {
        let path = scratch_repository("branch");
        commit_in(&path, "a.txt", "one");

        let git = Git::in_work_dir(&path);
        git.branch("feat/dialogs").expect("create the branch");

        assert_eq!(
            git.run(&["rev-parse", "--abbrev-ref", "HEAD"])
                .expect("read the branch"),
            "feat/dialogs",
        );

        std::fs::remove_dir_all(&path).expect("clean up");
    }

    #[test]
    fn a_failing_command_carries_stderr() {
        let path = scratch_repository("failure");

        let error = Git::in_work_dir(&path)
            .run(&["cat-file", "-p", "0000000000000000000000000000000000000000"])
            .expect_err("this object cannot exist");

        match error {
            Error::Failed { stderr, .. } => assert!(!stderr.is_empty(), "stderr was empty"),
            other => panic!("expected a failed command, got {other:?}"),
        }

        std::fs::remove_dir_all(&path).expect("clean up");
    }
}
