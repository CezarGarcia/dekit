//! Says when a repository changed: a commit, a checkout, something
//! staged, a fetch. Only `.git` is watched, where each of those writes;
//! the working tree is not, as watching every directory of a project
//! takes more than some systems allow. Edits to files are found by
//! reading the status again.

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::watch;

use super::Repo;

pub struct GitWatch {
  /// Watches for as long as it is kept.
  _watcher: notify::RecommendedWatcher,
}

impl GitWatch {
  /// Bumps `changes` each time something in `repo`'s `.git` changes.
  pub fn new(repo: &Repo, changes: watch::Sender<u64>) -> notify::Result<Self> {
    let mut watcher =
      notify::recommended_watcher(move |event: notify::Result<Event>| {
        match event {
          Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
            changes.send_modify(|n| *n = n.wrapping_add(1));
          }
          Ok(_) => (),
          Err(err) => log::debug!("Git watch: {err}"),
        }
      })?;
    // `HEAD`, the index, `FETCH_HEAD`, and what a merge or a rebase
    // leaves while it is under way.
    watcher.watch(&repo.git_dir, RecursiveMode::NonRecursive)?;
    if repo.common_dir != repo.git_dir {
      // `packed-refs`, in the `.git` the working trees share.
      watcher.watch(&repo.common_dir, RecursiveMode::NonRecursive)?;
    }
    let refs = repo.common_dir.join("refs");
    if refs.is_dir() {
      watcher.watch(&refs, RecursiveMode::Recursive)?;
    }
    Ok(GitWatch { _watcher: watcher })
  }
}
