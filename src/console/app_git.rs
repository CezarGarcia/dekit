//! The console's part in the git view: when the repository is read, and
//! what the view's keys and the mouse do. The repository is read only
//! while someone is attached: on a timer, sooner while the view is
//! shown, and right after `.git` changes. Each read runs `git` off the
//! console and posts back what it found.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::console::action::ScrollUnit;
use crate::console::git_view::GitFocus;
use crate::console::state::View;
use crate::console::ui_git::{head_name, sync_mark};
use crate::console::ui_header::HeaderGit;
use crate::git::{self, Repo, watch::GitWatch};
use crate::task::config_tasks::project_dir;
use crate::term::mouse::{MouseButton, MouseEvent, MouseEventKind};

use super::App;

/// How long after a read the next one is, while the git view is shown:
/// this is how soon an edit to a file shows up.
const POLL_SHOWN: Duration = Duration::from_secs(2);
/// The same while the task view is shown, where only the header tells
/// of the repository.
const POLL_HIDDEN: Duration = Duration::from_secs(10);
/// How long a change to `.git` settles before it is read: a commit is
/// several writes.
const SETTLE: Duration = Duration::from_millis(200);
/// The least time between a read and one a change to `.git` brings on,
/// should reading ever be what changes it.
const GAP: Duration = Duration::from_secs(1);
/// Lines a turn of the mouse wheel scrolls the diff by.
const WHEEL: isize = 3;
/// Columns a sideways scroll moves the diff by.
const SIDEWAYS: isize = 8;

/// When the repository is read, and what watches it.
#[derive(Default)]
pub(super) struct GitSync {
  /// What is shown may not be what is there: read as soon as can be.
  stale: bool,
  /// A read is under way.
  running: bool,
  /// When to read next; none while a read runs, and until someone
  /// attaches.
  pub(super) poll_at: Option<Instant>,
  /// When the last read came back.
  read_at: Option<Instant>,
  /// The repository as the last read found it, for the next one.
  repo: Option<Repo>,
  /// When a fetch was last started, whether or not it worked.
  fetch_tried: Option<Instant>,
  watch: Option<GitWatch>,
  /// The `.git` the watch is on, or is being set up on.
  watching: Option<PathBuf>,
  /// Bumped by the watch.
  pub(super) changes: Option<watch::Receiver<u64>>,
}

impl App {
  /// The time to read came.
  pub(super) fn git_poll(&mut self) {
    self.git.poll_at = None;
    self.git.stale = true;
  }

  /// `.git` changed: read once it settles, or right after the read
  /// under way, which may have been too early to see it.
  pub(super) fn git_changed(&mut self) {
    if self.git.running {
      self.git.stale = true;
      return;
    }
    let now = Instant::now();
    let mut at = now + SETTLE;
    if let Some(read_at) = self.git.read_at {
      at = at.max(read_at + GAP);
    }
    self.git.poll_at = Some(self.git.poll_at.map_or(at, |due| due.min(at)));
  }

  /// Starts a read when one is due and someone is attached to see it.
  pub(super) fn git_refresh(&mut self) {
    if self.git.running || !self.screen.has_observers() {
      return;
    }
    // Nothing asked and nothing scheduled: nobody was attached.
    if !self.git.stale && self.git.poll_at.is_some() {
      return;
    }
    self.git.stale = false;
    self.git.running = true;
    self.git.poll_at = None;
    let dir = project_dir(&self.project.config());
    let known = self.git.repo.clone();
    // The diff is only seen in the git view.
    let want = match self.state.view {
      View::Git => self.state.git.selected.clone(),
      View::Tasks => None,
    };
    self.spawn(async move {
      let read = git::read(&dir, known, want).await;
      move |app: &mut App| app.git_read(read)
    });
  }

  fn git_read(&mut self, read: io::Result<git::Read>) {
    let now = Instant::now();
    let shown = self.state.view == View::Git;
    self.git.running = false;
    self.git.read_at = Some(now);
    self.git.poll_at = Some(now + if shown { POLL_SHOWN } else { POLL_HIDDEN });
    let read = match read {
      Ok(read) => read,
      Err(err) => return log::debug!("Reading the git repository: {err}"),
    };
    self.git.repo = read.snapshot.as_ref().map(|s| s.repo.clone());
    let changed = self.state.git.take(read);
    // The view also says how long ago the last fetch was.
    self.dirty |= changed || shown;
    // The selection moved since the read was asked for: its diff is
    // still to read.
    let git = &self.state.git;
    if shown && git.selected.is_some() && git.shown_diff().is_none() {
      self.git.stale = true;
    }
    self.git_watch();
    self.git_auto_fetch();
  }

  /// Watches the repository's `.git`, once per repository.
  fn git_watch(&mut self) {
    let Some(repo) = self.git.repo.clone() else {
      self.git.watch = None;
      self.git.watching = None;
      self.git.changes = None;
      return;
    };
    if self.git.watching.as_ref() == Some(&repo.git_dir) {
      return;
    }
    self.git.watching = Some(repo.git_dir.clone());
    self.spawn(async move {
      // The system's watcher can take a while to start.
      let git_dir = repo.git_dir.clone();
      let made = tokio::task::spawn_blocking(move || {
        let (changes, changed) = watch::channel(0);
        GitWatch::new(&repo, changes).map(|watch| (watch, changed))
      })
      .await;
      move |app: &mut App| {
        if app.git.watching.as_ref() != Some(&git_dir) {
          return;
        }
        match made {
          Ok(Ok((watch, changed))) => {
            app.git.watch = Some(watch);
            app.git.changes = Some(changed);
          }
          // The timer still finds what changed.
          Ok(Err(err)) => {
            log::warn!("Can't watch {}: {err}", git_dir.display())
          }
          Err(_) => (),
        }
      }
    });
  }

  /// Fetches when the last fetch, by anyone, is older than
  /// `tui.git.fetch_every`.
  fn git_auto_fetch(&mut self) {
    let Some(every) = self.project.config().tui.git.fetch_every else {
      return;
    };
    let Some(snapshot) = &self.state.git.snapshot else {
      return;
    };
    if !snapshot.has_remote || self.state.git.fetching {
      return;
    }
    let recent =
      |ago: Option<Duration>| ago.is_some_and(|elapsed| elapsed < every);
    let fetched = snapshot.fetched.and_then(|at| at.elapsed().ok());
    let tried = self.git.fetch_tried.map(|at| at.elapsed());
    if recent(fetched) || recent(tried) {
      return;
    }
    self.git_fetch(false);
  }

  /// Fetches from the remote. `by_hand` says how it went in a toast;
  /// a failure is on the branch panel either way.
  pub(super) fn git_fetch(&mut self, by_hand: bool) {
    if self.state.git.fetching {
      return;
    }
    let remote = self
      .state
      .git
      .snapshot
      .as_ref()
      .map(|snapshot| (snapshot.repo.root.clone(), snapshot.has_remote));
    let root = match remote {
      Some((root, true)) => root,
      Some((_, false)) if by_hand => {
        return self.toast("The repository has no remote to fetch from.", true);
      }
      None if by_hand => {
        return self.toast("This project is not in a git repository.", true);
      }
      Some((_, false)) | None => return,
    };
    self.state.git.fetching = true;
    self.git.fetch_tried = Some(Instant::now());
    self.dirty = true;
    self.spawn(async move {
      let result = git::fetch(&root).await;
      move |app: &mut App| {
        app.state.git.fetching = false;
        app.git.stale = true;
        app.dirty = true;
        match result {
          Ok(()) => {
            app.state.git.fetch_error = None;
            if by_hand {
              app.toast("Fetched from the remote.", false);
            }
          }
          Err(err) => {
            if by_hand {
              app.toast(format!("git fetch: {err}"), true);
            }
            app.state.git.fetch_error = Some(err);
          }
        }
      }
    });
  }

  /// What the header shows of the repository, if the project is in one.
  pub(super) fn header_git(&self) -> Option<HeaderGit> {
    let snapshot = self.state.git.snapshot.as_ref()?;
    Some(HeaderGit {
      shown: self.state.view == View::Git,
      changes: snapshot.files.len(),
      branch: head_name(&snapshot.head),
      sync: sync_mark(snapshot),
    })
  }

  pub(super) fn show_view(&mut self, view: View) {
    if self.state.view == view {
      return;
    }
    self.state.view = view;
    self.state.drag = None;
    if view == View::Git {
      // The view opens on the repository as it is now.
      self.git.stale = true;
    }
  }

  pub(super) fn git_focus(&mut self, focus: GitFocus) {
    self.state.git.focus = focus;
  }

  /// The next change or the one before, or a line of the diff while it
  /// has the focus.
  pub(super) fn git_step(&mut self, by: isize) {
    match self.state.git.focus {
      GitFocus::Diff => self.state.git.scroll_by(by),
      GitFocus::Files => {
        if self.state.git.step(by) {
          self.git.stale = true;
        }
      }
    }
  }

  pub(super) fn git_scroll(&mut self, up: bool, n: usize, unit: ScrollUnit) {
    let page = usize::from(self.state.git.hits.diff.height.max(1));
    let lines = match unit {
      ScrollUnit::Line => n,
      ScrollUnit::HalfScreen => n.saturating_mul((page / 2).max(1)),
      ScrollUnit::Screen => n.saturating_mul(page),
    };
    let lines = lines.min(isize::MAX as usize) as isize;
    self.state.git.scroll_by(if up { -lines } else { lines });
  }

  pub(super) fn git_scroll_sideways(&mut self, right: bool) {
    let cols = if right { SIDEWAYS } else { -SIDEWAYS };
    self.state.git.scroll_x_by(cols);
  }

  /// The mouse under the header while the git view is shown.
  pub(super) fn git_mouse(&mut self, mouse: &MouseEvent) {
    let (x, y) = (mouse.x as u16, mouse.y as u16);
    let hits = &self.state.git.hits;
    let on_diff = hits.diff.contains(x, y);
    let on_side = hits.side.contains(x, y);
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
        let row = hits
          .rows
          .iter()
          .find(|(area, _)| area.contains(x, y))
          .map(|(_, at)| *at);
        if hits.fetch.is_some_and(|hit| hit.contains(x, y)) {
          self.git_fetch(true);
        } else if let Some(at) = row {
          self.state.git.focus = GitFocus::Files;
          if self.state.git.select(at) {
            self.git.stale = true;
          }
        } else if on_diff {
          self.state.git.focus = GitFocus::Diff;
        } else if on_side {
          self.state.git.focus = GitFocus::Files;
        }
      }
      MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
        let down = mouse.kind == MouseEventKind::ScrollDown;
        if on_diff {
          self.state.git.scroll_by(if down { WHEEL } else { -WHEEL });
        } else if on_side && self.state.git.step(if down { 1 } else { -1 }) {
          self.git.stale = true;
        }
      }
      MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight if on_diff => {
        self.git_scroll_sideways(mouse.kind == MouseEventKind::ScrollRight);
      }
      MouseEventKind::Down(_)
      | MouseEventKind::Up(_)
      | MouseEventKind::Drag(_)
      | MouseEventKind::Moved
      | MouseEventKind::ScrollLeft
      | MouseEventKind::ScrollRight => (),
    }
  }
}
