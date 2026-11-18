//! What the console shows of the project's git repository: the branch
//! and how it stands with its upstream, the files changed, and the diff
//! of one of them. All of it is read by running `git`, so the repository
//! is seen as the user's own git sees it, with their config and
//! credentials. Nothing here changes the working tree or the index.

pub mod diff;
pub mod status;
pub mod watch;

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

pub use diff::{Diff, DiffLine};
pub use status::{Area, FileChange, Head, Kind, Upstream};

/// How long a command that reads the repository may take.
const LOCAL_LIMIT: Duration = Duration::from_secs(30);
/// How long a fetch may take.
const FETCH_LIMIT: Duration = Duration::from_secs(120);
/// An untracked file larger than this is not read: its lines are not
/// counted, and it is not shown.
const MAX_NEW_FILE: u64 = 1024 * 1024;
/// A diff larger than this is not shown.
const MAX_DIFF: usize = 4 * 1024 * 1024;
/// How many untracked files have their lines counted.
const MAX_COUNTED: usize = 200;

/// Where a repository is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repo {
  /// The top of the working tree: what the paths of changes are from.
  pub root: PathBuf,
  /// This working tree's own `.git`: `HEAD`, the index.
  pub git_dir: PathBuf,
  /// The `.git` every working tree of the repository shares: the refs.
  pub common_dir: PathBuf,
}

/// The repository as last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
  pub repo: Repo,
  pub head: Head,
  pub upstream: Option<Upstream>,
  pub files: Vec<FileChange>,
  /// When the remotes were last fetched from.
  pub fetched: Option<SystemTime>,
  pub has_remote: bool,
  /// What is under way and not finished: a merge, a rebase.
  pub doing: Option<&'static str>,
}

/// One change of the snapshot.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FileKey {
  pub path: String,
  pub area: Area,
}

impl FileChange {
  pub fn key(&self) -> FileKey {
    FileKey {
      path: self.path.clone(),
      area: self.area,
    }
  }
}

/// What a read found: no snapshot where there is no repository, or no
/// git; the diff asked for whenever there is a snapshot.
#[derive(Debug, Default)]
pub struct Read {
  pub snapshot: Option<Snapshot>,
  pub diff: Option<(FileKey, Diff)>,
}

struct Output {
  ok: bool,
  stdout: Vec<u8>,
  stderr: String,
}

/// Runs `git` in `dir` and waits for it. It never asks for anything: the
/// runner has no terminal to ask on, so a command that would wait for a
/// password fails instead.
async fn git(dir: &Path, args: &[&str], limit: Duration) -> io::Result<Output> {
  // Looked for before it is run: a failed exec is the one case where
  // spawning waits for the child itself, which only the runner's reaper
  // may do.
  let program = which::which("git")
    .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;
  let mut cmd = std::process::Command::new(program);
  cmd
    .current_dir(dir)
    .args(args)
    // Reading must not write: a status that refreshed the index would
    // look like a change to whoever watches `.git`, this console
    // included.
    .env("GIT_OPTIONAL_LOCKS", "0")
    .env("GIT_TERMINAL_PROMPT", "0")
    .env("GCM_INTERACTIVE", "never")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  let late = || {
    io::Error::new(
      io::ErrorKind::TimedOut,
      format!("git {} took too long", args.first().unwrap_or(&"")),
    )
  };

  #[cfg(unix)]
  if crate::process::unix_processes_waiter::UnixProcessesWaiter::installed() {
    return reaped(cmd, limit).await.ok_or_else(late)?;
  }

  let mut cmd = tokio::process::Command::from(cmd);
  cmd.kill_on_drop(true);
  // The runner has no console window, and git must not open one.
  #[cfg(windows)]
  cmd.creation_flags(0x0800_0000);
  let out = tokio::time::timeout(limit, cmd.output())
    .await
    .map_err(|_| late())??;
  Ok(Output {
    ok: out.status.success(),
    stdout: out.stdout,
    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
  })
}

/// Runs `cmd` in a runner, where the reaper alone waits for children:
/// the exit comes from it, and the output is read off the pipes. None
/// when it takes longer than `limit`; it is killed then, with what it
/// started, as it is when the future is dropped.
#[cfg(unix)]
async fn reaped(
  mut cmd: std::process::Command,
  limit: Duration,
) -> Option<io::Result<Output>> {
  use std::os::unix::process::CommandExt as _;

  use rustix::process::Pid;

  use crate::process::unix_processes_waiter::UnixProcessesWaiter as Reaper;

  fn drain<R: io::Read + Send + 'static>(
    pipe: Option<R>,
  ) -> tokio::task::JoinHandle<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
      let mut bytes = Vec::new();
      if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
      }
      bytes
    })
  }

  /// Kills the process group unless the reaper has collected it.
  struct Running(Pid);
  impl Drop for Running {
    fn drop(&mut self) {
      Reaper::kill(self.0, libc::SIGKILL, true);
    }
  }

  // A group of its own, so that what git starts (ssh, a credential
  // helper) can be killed with it.
  cmd.process_group(0);
  let mark = Reaper::mark();
  let mut child = match cmd.spawn() {
    Ok(child) => child,
    Err(err) => return Some(Err(err)),
  };
  let pid = Pid::from_raw(child.id() as i32)?;
  let (sender, mut exits) = tokio::sync::mpsc::unbounded_channel();
  Reaper::wait_for(
    pid,
    mark,
    Box::new(move |info| {
      let _ = sender.send(info);
    }),
  );
  let _running = Running(pid);
  let stdout = drain(child.stdout.take());
  let stderr = drain(child.stderr.take());
  let finished = async {
    let exit = exits.recv().await;
    let stdout = stdout.await.unwrap_or_default();
    let stderr = stderr.await.unwrap_or_default();
    Output {
      ok: exit.is_some_and(|exit| exit.code == Some(0)),
      stdout,
      stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
  };
  tokio::time::timeout(limit, finished).await.ok().map(Ok)
}

/// The repository `dir` is in, if it is in a working tree of one.
pub async fn discover(dir: &Path) -> io::Result<Option<Repo>> {
  let args = [
    "rev-parse",
    "--show-toplevel",
    "--absolute-git-dir",
    "--git-common-dir",
  ];
  let out = git(dir, &args, LOCAL_LIMIT).await?;
  if !out.ok {
    return Ok(None);
  }
  let text = String::from_utf8_lossy(&out.stdout);
  let mut lines = text.lines();
  let (Some(root), Some(git_dir), Some(common)) =
    (lines.next(), lines.next(), lines.next())
  else {
    return Ok(None);
  };
  // The common dir is printed as git found it: from `dir` when relative.
  let real = |path: PathBuf| dunce::canonicalize(&path).unwrap_or(path);
  Ok(Some(Repo {
    root: PathBuf::from(root),
    git_dir: real(PathBuf::from(git_dir)),
    common_dir: real(dir.join(common)),
  }))
}

/// Reads the repository `dir` is in, and the diff of `want` if that
/// change is there. `known` is the repository as a read before found it,
/// which saves looking for it again.
pub async fn read(
  dir: &Path,
  known: Option<Repo>,
  want: Option<FileKey>,
) -> io::Result<Read> {
  let repo = match known {
    Some(repo) => repo,
    None => match discover(dir).await {
      Ok(Some(repo)) => repo,
      Ok(None) => return Ok(Read::default()),
      // No git, or no such directory.
      Err(err) if err.kind() == io::ErrorKind::NotFound => {
        return Ok(Read::default());
      }
      Err(err) => return Err(err),
    },
  };
  let root = &repo.root;
  let (listed, staged, unstaged, remotes) = tokio::join!(
    git(
      root,
      &["status", "--porcelain=v2", "--branch", "-z"],
      LOCAL_LIMIT
    ),
    git(root, &["diff", "--cached", "--numstat", "-z"], LOCAL_LIMIT),
    git(root, &["diff", "--numstat", "-z"], LOCAL_LIMIT),
    git(root, &["remote"], LOCAL_LIMIT),
  );
  let listed = match listed {
    Ok(listed) if listed.ok => listed,
    // Not a repository any more: the next read looks for one again.
    Ok(_) => return Ok(Read::default()),
    Err(err) if err.kind() == io::ErrorKind::NotFound => {
      return Ok(Read::default());
    }
    Err(err) => return Err(err),
  };
  let status::Status {
    head,
    upstream,
    mut files,
  } = status::parse(&listed.stdout);

  let counts = |out: io::Result<Output>| match out {
    Ok(out) if out.ok => status::parse_numstat(&out.stdout),
    Ok(_) | Err(_) => Default::default(),
  };
  let (staged, unstaged) = (counts(staged), counts(unstaged));
  for file in &mut files {
    let counted = match file.area {
      Area::Staged => staged.get(&file.path),
      Area::Unstaged | Area::Conflict => unstaged.get(&file.path),
    };
    if let Some((added, removed)) = counted {
      (file.added, file.removed) = (*added, *removed);
    }
  }
  let new_files: Vec<(usize, PathBuf)> = files
    .iter()
    .enumerate()
    .filter(|(_, file)| file.kind == Kind::Untracked)
    .take(MAX_COUNTED)
    .map(|(at, file)| (at, root.join(&file.path)))
    .collect();
  let counted = tokio::task::spawn_blocking(move || {
    new_files
      .into_iter()
      .map(|(at, path)| (at, count_lines(&path)))
      .collect::<Vec<_>>()
  })
  .await
  .unwrap_or_default();
  for (at, lines) in counted {
    files[at].added = lines;
  }

  let diff = match want {
    Some(key) => {
      let diff = match files.iter().find(|file| file.key() == key) {
        Some(file) => file_diff(root, file).await,
        None => Diff::default(),
      };
      Some((key, diff))
    }
    None => None,
  };
  let fetched = std::fs::metadata(repo.git_dir.join("FETCH_HEAD"))
    .and_then(|meta| meta.modified())
    .ok();
  let has_remote = matches!(
    &remotes,
    Ok(out) if out.ok && out.stdout.iter().any(|b| !b.is_ascii_whitespace())
  );
  let doing = doing(&repo);
  Ok(Read {
    snapshot: Some(Snapshot {
      repo,
      head,
      upstream,
      files,
      fetched,
      has_remote,
      doing,
    }),
    diff,
  })
}

/// Fetches from the default remote, as `git fetch` does; the error is
/// what git said.
pub async fn fetch(root: &Path) -> Result<(), String> {
  match git(root, &["fetch", "--quiet"], FETCH_LIMIT).await {
    Ok(out) if out.ok => Ok(()),
    Ok(out) => Err(reason(&out.stderr)),
    Err(err) => Err(err.to_string()),
  }
}

/// The line of git's complaint that says what is wrong.
fn reason(stderr: &str) -> String {
  let lines = || stderr.lines().map(str::trim).filter(|l| !l.is_empty());
  let line = lines()
    .find(|line| line.starts_with("fatal:") || line.starts_with("error:"))
    .or_else(|| lines().next())
    .unwrap_or("git failed");
  let line = line
    .strip_prefix("fatal: ")
    .or_else(|| line.strip_prefix("error: "))
    .unwrap_or(line);
  diff::clean(line)
}

async fn file_diff(root: &Path, file: &FileChange) -> Diff {
  if file.kind == Kind::Untracked {
    let path = root.join(&file.path);
    return tokio::task::spawn_blocking(move || new_file_diff(&path))
      .await
      .unwrap_or_default();
  }
  let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
  if file.area == Area::Staged {
    args.push("--cached");
  }
  args.push("--");
  // Both paths of a rename, or it is shown as a file added.
  if let Some(orig) = &file.orig {
    args.push(orig);
  }
  args.push(&file.path);
  match git(root, &args, LOCAL_LIMIT).await {
    Ok(out) if out.stdout.len() > MAX_DIFF => {
      Diff::note("The diff is too large to show")
    }
    Ok(out) if out.ok => diff::parse(&out.stdout),
    Ok(out) => Diff::note(reason(&out.stderr)),
    Err(err) => Diff::note(err.to_string()),
  }
}

fn new_file_diff(path: &Path) -> Diff {
  let meta = match std::fs::metadata(path) {
    Ok(meta) => meta,
    Err(err) => return Diff::note(err.to_string()),
  };
  if meta.is_dir() {
    return Diff::note("A directory that is not in the repository yet");
  }
  if meta.len() > MAX_NEW_FILE {
    return Diff::note("The file is too large to show");
  }
  match std::fs::read(path) {
    Ok(bytes) => diff::of_new_file(&bytes),
    Err(err) => Diff::note(err.to_string()),
  }
}

/// The lines of a text file small enough to read.
fn count_lines(path: &Path) -> Option<u32> {
  let meta = std::fs::metadata(path).ok()?;
  if !meta.is_file() || meta.len() > MAX_NEW_FILE {
    return None;
  }
  let bytes = std::fs::read(path).ok()?;
  if diff::is_binary(&bytes) {
    return None;
  }
  let newlines = bytes.iter().filter(|b| **b == b'\n').count();
  let last = usize::from(bytes.last().is_some_and(|b| *b != b'\n'));
  Some((newlines + last) as u32)
}

/// What the repository is in the middle of, by what git leaves in `.git`
/// meanwhile.
fn doing(repo: &Repo) -> Option<&'static str> {
  let has = |name: &str| repo.git_dir.join(name).exists();
  if has("rebase-merge") || has("rebase-apply") {
    Some("rebasing")
  } else if has("MERGE_HEAD") {
    Some("merging")
  } else if has("CHERRY_PICK_HEAD") {
    Some("cherry-picking")
  } else if has("REVERT_HEAD") {
    Some("reverting")
  } else if has("BISECT_LOG") {
    Some("bisecting")
  } else {
    None
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::test_util::TempDir;

  fn run(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
      .current_dir(dir)
      .args([
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "init.defaultBranch=main",
      ])
      .args(args)
      .output()
      .unwrap();
    assert!(
      out.status.success(),
      "git {args:?}: {}",
      String::from_utf8_lossy(&out.stderr)
    );
  }

  /// A repository with one commit, then: a file changed and staged, the
  /// same file changed again, one deleted, and one new.
  fn repo(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    let root = &dir.0;
    run(root, &["init", "--quiet"]);
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(root.join("old.txt"), "bye\n").unwrap();
    run(root, &["add", "."]);
    run(root, &["commit", "--quiet", "-m", "first"]);
    std::fs::write(root.join("a.txt"), "one\n2\nthree\n").unwrap();
    run(root, &["add", "a.txt"]);
    std::fs::write(root.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
    std::fs::remove_file(root.join("old.txt")).unwrap();
    std::fs::write(root.join("new.txt"), "x\ny").unwrap();
    dir
  }

  #[tokio::test]
  async fn reads_a_repository_and_the_diff_asked_for() {
    let dir = repo("git-read");
    let want = FileKey {
      path: "a.txt".to_string(),
      area: Area::Staged,
    };
    let read = read(&dir.0, None, Some(want.clone())).await.unwrap();
    let snapshot = read.snapshot.unwrap();
    assert_eq!(snapshot.head, Head::Branch("main".to_string()));
    assert_eq!(snapshot.upstream, None);
    assert!(!snapshot.has_remote);
    assert_eq!(snapshot.doing, None);
    let listed: Vec<_> = snapshot
      .files
      .iter()
      .map(|f| (f.path.as_str(), f.area, f.kind.letter(), f.added, f.removed))
      .collect();
    assert_eq!(
      listed,
      vec![
        ("a.txt", Area::Staged, 'M', Some(1), Some(1)),
        ("a.txt", Area::Unstaged, 'M', Some(1), Some(0)),
        ("old.txt", Area::Unstaged, 'D', Some(0), Some(1)),
        ("new.txt", Area::Unstaged, '?', Some(2), None),
      ]
    );

    let (key, diff) = read.diff.unwrap();
    assert_eq!(key, want);
    assert!(diff.lines.contains(&DiffLine::Removed {
      old: 2,
      text: "two".to_string()
    }));
    assert!(diff.lines.contains(&DiffLine::Added {
      new: 2,
      text: "2".to_string()
    }));
    // What is staged, not what the file has by now.
    assert!(!diff.lines.iter().any(
      |line| matches!(line, DiffLine::Added { text, .. } if text == "four")
    ));

    // A file that is not in the repository yet is shown whole, and the
    // repository found before is taken as given.
    let want = FileKey {
      path: "new.txt".to_string(),
      area: Area::Unstaged,
    };
    let again = super::read(&dir.0, Some(snapshot.repo.clone()), Some(want))
      .await
      .unwrap();
    assert_eq!(again.snapshot.as_ref(), Some(&snapshot));
    assert_eq!(again.diff.unwrap().1, diff::of_new_file(b"x\ny"));
  }

  /// Reading leaves `.git` as it was: a read that wrote would be seen as
  /// a change, and read again without end.
  #[tokio::test]
  async fn reading_does_not_touch_the_index() {
    let dir = repo("git-read-only");
    let index = dir.0.join(".git/index");
    let before = std::fs::metadata(&index).unwrap().modified().unwrap();
    // Changed since the index was written, so a status would refresh it.
    std::fs::write(dir.0.join("a.txt"), "other\n").unwrap();
    read(&dir.0, None, None).await.unwrap();
    let after = std::fs::metadata(&index).unwrap().modified().unwrap();
    assert_eq!(before, after);
    assert!(!dir.0.join(".git/index.lock").exists());
  }

  #[tokio::test]
  async fn a_directory_outside_any_repository_has_no_snapshot() {
    let dir = TempDir::new("git-none");
    let found = tokio::process::Command::new("git")
      .current_dir(&dir.0)
      .args(["rev-parse", "--show-toplevel"])
      .output()
      .await
      .unwrap();
    if found.status.success() {
      // The temp dir is inside a repository: nothing to tell here.
      return;
    }
    let read = read(&dir.0, None, None).await.unwrap();
    assert!(read.snapshot.is_none());
    assert!(read.diff.is_none());
  }

  #[tokio::test]
  async fn a_fetch_without_a_remote_says_what_git_said() {
    let dir = repo("git-fetch");
    // Nothing to fetch from is not a failure.
    assert_eq!(fetch(&dir.0).await, Ok(()));
    run(&dir.0, &["remote", "add", "origin", "./nowhere"]);
    let err = fetch(&dir.0).await.unwrap_err();
    assert!(!err.is_empty() && !err.starts_with("fatal:"), "{err}");
    let read = read(&dir.0, None, None).await.unwrap();
    assert!(read.snapshot.unwrap().has_remote);
  }

  #[test]
  fn picks_the_line_that_says_what_is_wrong() {
    let stderr = "warning: slow\nfatal: unable to access 'https://x/': \
                  Could not resolve host: x\n";
    assert_eq!(
      reason(stderr),
      "unable to access 'https://x/': Could not resolve host: x"
    );
    assert_eq!(reason("  \n"), "git failed");
  }
}
