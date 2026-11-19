//! The repository's branches and its stash: listing them, and what is
//! done with them from the console. A merge and a rebase are not here:
//! they run as tasks, like a pull, with a terminal of their own.

use std::path::Path;

use super::{LOCAL_LIMIT, Output, Upstream, diff, done, git};

/// How long fetching one branch may take.
const UPDATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Branch {
  pub name: String,
  /// The one checked out.
  pub current: bool,
  pub upstream: Option<Upstream>,
  /// When its last commit was made, in seconds since 1970.
  pub date: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Branches {
  /// The checked-out one first, then the ones committed to last.
  pub local: Vec<Branch>,
  /// Branches of the remotes that no local branch follows or is named
  /// after, as `origin/name`.
  pub remote: Vec<String>,
}

/// A set of changes put aside with `git stash`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stash {
  /// Its place in the stash: 0 is the latest.
  pub index: usize,
  /// What git says of it: `WIP on main: 3fa92c1 Fix`, or its message.
  pub message: String,
}

/// What `git switch` does with changes that are not committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Carry {
  /// They stay in the working tree; git refuses when the other branch
  /// has the same files otherwise.
  Keep,
  /// The changes to tracked files are thrown away.
  Discard,
}

const REFS_FORMAT: &str = "--format=%(refname)%00%(upstream:short)%00%(upstream:track)%00%(HEAD)%00%(committerdate:unix)";

/// The local branches and the remotes' own.
pub async fn branches(root: &Path) -> Result<Branches, String> {
  let args = ["for-each-ref", REFS_FORMAT, "refs/heads", "refs/remotes"];
  match git(root, &args, LOCAL_LIMIT).await {
    Ok(out) if out.ok => Ok(parse_refs(&String::from_utf8_lossy(&out.stdout))),
    Ok(out) => Err(said(&out)),
    Err(err) => Err(err.to_string()),
  }
}

/// Reads `git for-each-ref` in `REFS_FORMAT`.
fn parse_refs(text: &str) -> Branches {
  let mut local = Vec::new();
  let mut remote = Vec::new();
  for line in text.lines() {
    let mut fields = line.split('\0');
    let (Some(name), Some(upstream), Some(track), Some(head), Some(date)) = (
      fields.next(),
      fields.next(),
      fields.next(),
      fields.next(),
      fields.next(),
    ) else {
      continue;
    };
    if let Some(name) = name.strip_prefix("refs/heads/") {
      let upstream = (!upstream.is_empty()).then(|| {
        let count = |what: &str| -> u32 {
          track
            .split_once(what)
            .and_then(|(_, rest)| {
              let digits: String =
                rest.chars().take_while(char::is_ascii_digit).collect();
              digits.parse().ok()
            })
            .unwrap_or(0)
        };
        Upstream {
          name: upstream.to_string(),
          ahead: count("ahead "),
          behind: count("behind "),
          gone: track.contains("gone"),
        }
      });
      local.push(Branch {
        name: name.to_string(),
        current: head == "*",
        upstream,
        date: date.parse().unwrap_or(0),
      });
    } else if let Some(name) = name.strip_prefix("refs/remotes/") {
      // `origin/HEAD` only says which branch the remote opens on.
      if !name.ends_with("/HEAD") {
        remote.push(name.to_string());
      }
    }
  }
  local.sort_by(|a, b| {
    (b.current, b.date)
      .cmp(&(a.current, a.date))
      .then_with(|| a.name.cmp(&b.name))
  });
  remote.retain(|name| {
    let short = name
      .split_once('/')
      .map_or(name.as_str(), |(_, short)| short);
    !local.iter().any(|branch| {
      branch.name == short
        || branch.upstream.as_ref().is_some_and(|up| up.name == *name)
    })
  });
  remote.sort();
  Branches { local, remote }
}

/// Checks `branch` out.
pub async fn switch(
  root: &Path,
  branch: &str,
  carry: Carry,
) -> Result<(), String> {
  let mut args = vec!["switch"];
  if carry == Carry::Discard {
    args.push("--discard-changes");
  }
  args.push(branch);
  heard(git(root, &args, LOCAL_LIMIT).await)
}

/// Makes a local branch that follows the remote's `remote` (`origin/x`)
/// under its own name, `x`, and checks it out.
pub async fn switch_to_remote(
  root: &Path,
  remote: &str,
  carry: Carry,
) -> Result<(), String> {
  let name = remote.split_once('/').map_or(remote, |(_, name)| name);
  let mut args = vec!["switch"];
  if carry == Carry::Discard {
    args.push("--discard-changes");
  }
  args.extend(["-c", name, "--track", remote]);
  heard(git(root, &args, LOCAL_LIMIT).await)
}

/// Makes the branch `name` at `from`, and checks it out with `switch`.
/// It follows nothing until it is published: made from `origin/develop`,
/// it must not push there.
pub async fn create(
  root: &Path,
  name: &str,
  from: &str,
  switch: bool,
) -> Result<(), String> {
  let args: &[&str] = if switch {
    &["switch", "--no-track", "-c", name, from]
  } else {
    &["branch", "--no-track", name, from]
  };
  heard(git(root, args, LOCAL_LIMIT).await)
}

/// Deletes the local branch `name`. Without `force` git refuses one with
/// commits that are nowhere else.
pub async fn delete(
  root: &Path,
  name: &str,
  force: bool,
) -> Result<(), String> {
  let flag = if force { "-D" } else { "-d" };
  heard(git(root, &["branch", flag, name], LOCAL_LIMIT).await)
}

/// Brings the local branch `name` level with its upstream `upstream`
/// (`origin/x`) without checking it out. Only a fast-forward: git refuses
/// when the branch has commits of its own.
pub async fn update(
  root: &Path,
  name: &str,
  upstream: &str,
) -> Result<(), String> {
  let Some((remote, theirs)) = upstream.split_once('/') else {
    return Err(format!("{upstream} is not a remote's branch"));
  };
  let refspec = format!("{theirs}:{name}");
  done(git(root, &["fetch", remote, &refspec], UPDATE_LIMIT).await)
}

/// What is in the stash, the latest first.
pub fn parse_stashes(text: &str) -> Vec<Stash> {
  text
    .lines()
    .filter(|line| !line.is_empty())
    .enumerate()
    .map(|(index, line)| Stash {
      index,
      message: diff::clean(line),
    })
    .collect()
}

/// Puts every change aside, files not in the repository included, and
/// leaves the working tree clean.
pub async fn stash_push(root: &Path, message: &str) -> Result<(), String> {
  let args = ["stash", "push", "--include-untracked", "-m", message];
  heard(git(root, &args, LOCAL_LIMIT).await)
}

/// Brings a stash's changes back and drops it; it stays when they do not
/// apply cleanly.
pub async fn stash_pop(root: &Path, index: usize) -> Result<(), String> {
  let name = format!("stash@{{{index}}}");
  heard(git(root, &["stash", "pop", &name], LOCAL_LIMIT).await)
}

/// Throws a stash away.
pub async fn stash_drop(root: &Path, index: usize) -> Result<(), String> {
  let name = format!("stash@{{{index}}}");
  heard(git(root, &["stash", "drop", &name], LOCAL_LIMIT).await)
}

/// Gives up the merge, rebase, cherry-pick, or revert under way: the
/// repository goes back to where it was before it.
pub async fn abort(root: &Path, doing: &str) -> Result<(), String> {
  let Some(command) = command_of(doing) else {
    return Err(format!("Nothing to abort while {doing}"));
  };
  heard(git(root, &[command, "--abort"], LOCAL_LIMIT).await)
}

/// Goes on with a rebase, cherry-pick, or revert once its conflicts are
/// resolved and staged. The message git proposes is taken as it is: an
/// editor would have no terminal to open in.
pub async fn go_on(root: &Path, doing: &str) -> Result<(), String> {
  let Some(command) = command_of(doing) else {
    return Err(format!("Nothing to continue while {doing}"));
  };
  let args = ["-c", "core.editor=true", command, "--continue"];
  heard(git(root, &args, LOCAL_LIMIT).await)
}

/// The git command behind what `Snapshot::doing` says is under way.
fn command_of(doing: &str) -> Option<&'static str> {
  match doing {
    "merging" => Some("merge"),
    "rebasing" => Some("rebase"),
    "cherry-picking" => Some("cherry-pick"),
    "reverting" => Some("revert"),
    _ => None,
  }
}

/// Everything git said, for a failure whose first line alone would not
/// say which files are in the way.
fn said(out: &Output) -> String {
  let text =
    format!("{}\n{}", out.stderr, String::from_utf8_lossy(&out.stdout));
  let lines: Vec<String> = text
    .lines()
    .map(diff::clean)
    .filter(|line| !line.trim().is_empty())
    .collect();
  if lines.is_empty() {
    "git failed".to_string()
  } else {
    lines.join("\n")
  }
}

fn heard(out: std::io::Result<Output>) -> Result<(), String> {
  match out {
    Ok(out) if out.ok => Ok(()),
    Ok(out) => Err(said(&out)),
    Err(err) => Err(err.to_string()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_the_branches_and_how_each_stands() {
    let text = [
      "refs/heads/develop\0origin/develop\0[behind 12]\0 \01700000300",
      "refs/heads/feat-#13193\0origin/feat-#13193\0[ahead 2, behind 3]\0*\01700000100",
      "refs/heads/feat-#13174\0origin/feat-#13174\0\0 \01700000200",
      "refs/heads/bug-#13159\0origin/bug-#13159\0[gone]\0 \01700000050",
      "refs/heads/local-only\0\0\0 \01700000400",
      "refs/remotes/origin/HEAD\0\0\0 \01700000300",
      "refs/remotes/origin/develop\0\0\0 \01700000300",
      "refs/remotes/origin/feat-#13193\0\0\0 \01700000100",
      "refs/remotes/origin/feat-#13174\0\0\0 \01700000200",
      "refs/remotes/origin/feat-#13201\0\0\0 \01700000500",
      "refs/remotes/origin/local-only\0\0\0 \01700000500",
    ]
    .join("\n");
    let branches = parse_refs(&text);
    let names: Vec<&str> =
      branches.local.iter().map(|b| b.name.as_str()).collect();
    // The checked-out one, then the latest commits first.
    assert_eq!(
      names,
      vec![
        "feat-#13193",
        "local-only",
        "develop",
        "feat-#13174",
        "bug-#13159"
      ]
    );
    let stands = |name: &str| {
      let branch = branches.local.iter().find(|b| b.name == name).unwrap();
      branch
        .upstream
        .as_ref()
        .map(|up| (up.ahead, up.behind, up.gone))
    };
    assert_eq!(stands("feat-#13193"), Some((2, 3, false)));
    assert_eq!(stands("develop"), Some((0, 12, false)));
    assert_eq!(stands("feat-#13174"), Some((0, 0, false)));
    assert_eq!(stands("bug-#13159"), Some((0, 0, true)));
    assert_eq!(stands("local-only"), None);
    assert!(branches.local[0].current);
    // Only what has no local branch: followed, or of the same name.
    assert_eq!(branches.remote, vec!["origin/feat-#13201"]);
  }

  #[test]
  fn lists_the_stash_in_order() {
    let stashes =
      parse_stashes("On main: left by dekit\nWIP on main: 3fa92c1 Fix\n");
    assert_eq!(stashes.len(), 2);
    assert_eq!(stashes[0].index, 0);
    assert_eq!(stashes[1].message, "WIP on main: 3fa92c1 Fix");
    assert!(parse_stashes("").is_empty());
  }
}
