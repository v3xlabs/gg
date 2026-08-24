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

    fn scratch_repository(name: &str) -> PathBuf {
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
