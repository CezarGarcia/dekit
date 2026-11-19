//! The console's part in the git view: when the repository is read, and
//! what the view's keys and the mouse do. The repository is read only
//! while someone is attached: on a timer, sooner while the view is
//! shown, and right after `.git` changes. Each read runs `git` off the
//! console and posts back what it found.
//!
//! What changes the repository goes one at a time, in the order asked
//! for: staging, discarding, committing. A pull and a push run as tasks
//! of the runner, each with a terminal of its own, since they may ask
//! for a password; the view follows the task and reports how it ended.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::command::{Command, execute};
use crate::console::action::{Action, ScrollUnit};
use crate::console::git_view::{Changes, GitDo, GitFocus, PullHow, Syncing};
use crate::console::keymap::KeymapGroup;
use crate::console::modal::git_branches::GitBranches;
use crate::console::modal::git_choice::{Choice, GitChoice};
use crate::console::modal::git_new_branch::GitNewBranch;
use crate::console::modal::menu::{MenuItem, MenuModal};
use crate::console::state::{Scope, View};
use crate::console::ui_git::{head_name, sync_mark};
use crate::console::ui_header::HeaderGit;
use crate::git::{
  self, Area, Carry, FileChange, Head, Kind, Repo, branch, watch::GitWatch,
};
use crate::spec::TaskSpec;
use crate::target::Target;
use crate::task::config_tasks::project_dir;
use crate::term::grid::Rect;
use crate::term::key::{Key, KeyCode, KeyMods};
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
  /// What is still to do to the repository, in the order asked for.
  ops: VecDeque<Op>,
  /// One of them is under way.
  op_running: bool,
}

/// Something that changes the repository.
enum Op {
  Stage(Vec<FileChange>),
  Unstage(Vec<FileChange>),
  Discard(FileChange),
  Commit {
    message: String,
    amend: bool,
    /// Push once it is made.
    push: bool,
  },
  /// What is done with a branch or the stash, and what is said once it
  /// is done.
  Branch(BranchOp, String),
}

enum BranchOp {
  Switch {
    name: String,
    remote: bool,
    carry: Carry,
  },
  Create {
    name: String,
    from: String,
    switch: bool,
  },
  Delete {
    name: String,
    force: bool,
  },
  Update {
    name: String,
    upstream: String,
  },
  StashPush(String),
  StashPop(usize),
  StashDrop(usize),
  /// Give up, or go on with, what `Snapshot::doing` names.
  Abort(&'static str),
  GoOn(&'static str),
}

impl BranchOp {
  /// What its failure is headed with.
  fn title(&self) -> &'static str {
    match self {
      BranchOp::Switch { .. } => "The checkout failed",
      BranchOp::Create { .. } => "The branch was not made",
      BranchOp::Delete { .. } => "The branch was not deleted",
      BranchOp::Update { .. } => "The branch was not updated",
      BranchOp::StashPush(_) => "The changes were not stashed",
      BranchOp::StashPop(_) => "The stash did not come back whole",
      BranchOp::StashDrop(_) => "The stash was not dropped",
      BranchOp::Abort(_) => "The abort failed",
      BranchOp::GoOn(_) => "It did not go on",
    }
  }

  async fn run(&self, root: &std::path::Path) -> Result<(), String> {
    match self {
      BranchOp::Switch {
        name,
        remote: false,
        carry,
      } => branch::switch(root, name, *carry).await,
      BranchOp::Switch {
        name,
        remote: true,
        carry,
      } => branch::switch_to_remote(root, name, *carry).await,
      BranchOp::Create { name, from, switch } => {
        branch::create(root, name, from, *switch).await
      }
      BranchOp::Delete { name, force } => {
        branch::delete(root, name, *force).await
      }
      BranchOp::Update { name, upstream } => {
        branch::update(root, name, upstream).await
      }
      BranchOp::StashPush(message) => branch::stash_push(root, message).await,
      BranchOp::StashPop(index) => branch::stash_pop(root, *index).await,
      BranchOp::StashDrop(index) => branch::stash_drop(root, *index).await,
      BranchOp::Abort(doing) => branch::abort(root, doing).await,
      BranchOp::GoOn(doing) => branch::go_on(root, doing).await,
    }
  }
}

/// How an `Op` went.
enum Done {
  Changed(Result<(), String>),
  Discarded(String, Result<(), String>),
  Committed {
    made: Result<String, String>,
    push: bool,
  },
  Ran {
    op: BranchOp,
    said: String,
    went: Result<(), String>,
  },
}

/// A branch's own name: a remote's, `origin/x`, without the remote.
fn short_name(name: &str, remote: bool) -> &str {
  match name.split_once('/') {
    Some((_, short)) if remote => short,
    Some(_) | None => name,
  }
}

fn commits(n: u32) -> String {
  if n == 1 {
    "1 commit".to_string()
  } else {
    format!("{n} commits")
  }
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
    // The message is typed only where there is something to commit to.
    if focus == GitFocus::Message && self.state.git.snapshot.is_none() {
      return;
    }
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
      GitFocus::Message => (),
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
    let on = |hit: Option<Rect>| hit.is_some_and(|hit| hit.contains(x, y));
    let on_diff = hits.diff.contains(x, y);
    let on_side = hits.side.contains(x, y);
    let at = |list: &[(Rect, usize)]| {
      list
        .iter()
        .find(|(area, _)| area.contains(x, y))
        .map(|(_, at)| *at)
    };
    let row = at(&hits.rows);
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
        let check = at(&hits.checks);
        let group = hits
          .groups
          .iter()
          .find(|(area, _)| area.contains(x, y))
          .map(|(_, area)| *area);
        let stash = at(&hits.stashes);
        if let Some(branch) = hits.branch.filter(|hit| hit.contains(x, y)) {
          self.git_show_branches(Some(branch));
        } else if on(hits.abort) {
          self.git_ask_abort();
        } else if on(hits.go_on) {
          self.git_continue();
        } else if let Some(index) = stash {
          self.git_pop_stash(index);
        } else if on(hits.fetch) {
          self.git_fetch(true);
        } else if on(hits.pull) {
          self.git_pull();
        } else if on(hits.push) {
          self.git_push(false);
        } else if on(hits.commit) {
          self.git_commit(false, false);
        } else if on(hits.commit_push) {
          self.git_commit(true, false);
        } else if on(hits.amend) {
          self.git_toggle_amend();
        } else if on(hits.message) {
          self.git_focus(GitFocus::Message);
        } else if let Some(area) = group {
          self.state.git.focus = GitFocus::Files;
          self.git_stage_group(area);
        } else if let Some(at) = check {
          self.state.git.focus = GitFocus::Files;
          self.git_stage_at(at);
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
      MouseEventKind::Down(MouseButton::Right) => {
        if let Some(index) = at(&hits.stash_rows) {
          self.git_stash_menu(index);
        } else if let Some(at) = row {
          self.state.git.focus = GitFocus::Files;
          if self.state.git.select(at) {
            self.git.stale = true;
          }
          self.git_menu(Some(Rect::new(x, y, 1, 1)));
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

  /// A key while the commit message has the focus: typed into it,
  /// unless it commits or leaves.
  pub(super) fn git_message_key(&mut self, key: &Key) {
    let commit = KeyMods::ALT | KeyMods::META | KeyMods::CONTROL;
    match (key.code, key.mods) {
      (KeyCode::Esc | KeyCode::Tab, KeyMods::NONE) => {
        self.state.git.focus = GitFocus::Files;
      }
      (KeyCode::Enter, mods) if mods.intersects(commit) => {
        self.git_commit(false, false);
      }
      _ => {
        let width = self.git_message_width();
        self.state.git.message.handle_key(key, width);
      }
    }
  }

  /// How wide the message's text is where it was last drawn.
  fn git_message_width(&self) -> u16 {
    let field = self.state.git.hits.message.unwrap_or_default();
    field.width.saturating_sub(2).max(1)
  }

  /// Text pasted while the commit message has the focus.
  pub(super) fn git_paste(&mut self, text: &str) {
    self.state.git.message.insert_str(text);
  }

  /// Queues something that changes the repository.
  fn git_op(&mut self, op: Op) {
    self.git.ops.push_back(op);
    self.git_pump();
  }

  /// Starts the next change to the repository once the one before is
  /// done.
  fn git_pump(&mut self) {
    if self.git.op_running {
      return;
    }
    let Some(snapshot) = &self.state.git.snapshot else {
      self.git.ops.clear();
      self.state.git.committing = false;
      return;
    };
    let Some(op) = self.git.ops.pop_front() else {
      return;
    };
    let root = snapshot.repo.root.clone();
    let unborn = matches!(snapshot.head, Head::Unborn(_));
    self.git.op_running = true;
    self.spawn(async move {
      let done = match op {
        Op::Stage(files) => Done::Changed(git::stage(&root, &files).await),
        Op::Unstage(files) => {
          Done::Changed(git::unstage(&root, &files, unborn).await)
        }
        Op::Discard(file) => {
          let went = git::discard(&root, &file).await;
          Done::Discarded(file.path, went)
        }
        Op::Commit {
          message,
          amend,
          push,
        } => Done::Committed {
          made: git::commit(&root, &message, amend).await,
          push,
        },
        Op::Branch(op, said) => {
          let went = op.run(&root).await;
          Done::Ran { op, said, went }
        }
      };
      move |app: &mut App| app.git_op_done(done)
    });
  }

  fn git_op_done(&mut self, done: Done) {
    self.git.op_running = false;
    self.git.stale = true;
    self.dirty = true;
    match done {
      Done::Changed(Ok(())) => (),
      Done::Changed(Err(err)) | Done::Discarded(_, Err(err)) => {
        self.toast(format!("git: {err}"), true);
      }
      Done::Discarded(path, Ok(())) => {
        self.toast(format!("Discarded the changes to {path}."), false);
      }
      Done::Committed { made, push } => {
        self.state.git.committing = false;
        match made {
          Ok(commit) => {
            let git = &mut self.state.git;
            git.message.set("");
            git.amend = false;
            git.focus = GitFocus::Files;
            self.toast(format!("Committed {commit}."), false);
            if push {
              self.git_push(true);
            }
          }
          // What git and its hooks said, whole: a toast would cut it.
          Err(said) => {
            self.open(Box::new(GitChoice::message("Commit failed", said)));
          }
        }
      }
      Done::Ran {
        said, went: Ok(()), ..
      } => {
        if !said.is_empty() {
          self.toast(said, false);
        }
      }
      Done::Ran {
        op, went: Err(err), ..
      } => {
        // What was to follow it does not: a checkout after a stash that
        // failed would carry the changes along.
        self.git.ops.clear();
        match op {
          BranchOp::Delete { name, force: false }
            if err.contains("not fully merged") =>
          {
            let text = format!(
              "Delete {name}?\nIt has commits that are on no other \
               branch: nothing would bring them back."
            );
            let force = GitDo::DeleteBranch { name, force: true };
            let options = vec![
              Choice::new("Delete anyway", "git branch -D", force).danger(),
              Choice::close("Cancel"),
            ];
            self.open(Box::new(GitChoice::new("Delete", text, options, 1)));
          }
          op => self.open(Box::new(GitChoice::message(op.title(), err))),
        }
      }
    }
    self.git_pump();
  }

  /// Stages the selected change, or takes it back out of the index;
  /// the selection goes on to the change that was next to it.
  pub(super) fn git_stage_selected(&mut self) {
    let Some(at) = self.state.git.selected_index() else {
      return;
    };
    self.state.git.select_next = self.state.git.neighbour(at);
    self.git_stage_at(at);
  }

  fn git_stage_at(&mut self, at: usize) {
    let Some(file) = self.state.git.files().get(at).cloned() else {
      return;
    };
    self.git_op(match file.area {
      Area::Staged => Op::Unstage(vec![file]),
      Area::Unstaged | Area::Conflict => Op::Stage(vec![file]),
    });
  }

  /// Stages everything; with nothing left to stage, unstages it all.
  pub(super) fn git_stage_all(&mut self) {
    let unstaged = self
      .state
      .git
      .files()
      .iter()
      .any(|file| file.area != Area::Staged);
    self.git_stage_group(if unstaged {
      Area::Unstaged
    } else {
      Area::Staged
    });
  }

  /// Unstages every staged change, or stages every other one.
  fn git_stage_group(&mut self, area: Area) {
    let staged = area == Area::Staged;
    let files: Vec<FileChange> = self
      .state
      .git
      .files()
      .iter()
      .filter(|file| (file.area == Area::Staged) == staged)
      .cloned()
      .collect();
    if files.is_empty() {
      return;
    }
    self.git_op(if staged {
      Op::Unstage(files)
    } else {
      Op::Stage(files)
    });
  }

  /// Asks before throwing the selected change away.
  pub(super) fn git_ask_discard(&mut self) {
    let Some(file) = self.state.git.selected_file() else {
      return;
    };
    if file.area != Area::Unstaged {
      return self.toast(
        "Only a change that is not staged can be discarded: unstage it first.",
        true,
      );
    }
    let (text, label, note) = if file.kind == Kind::Untracked {
      (
        format!(
          "Delete {}?\nIt is not in the repository: nothing can bring it back.",
          file.path
        ),
        "Delete",
        "git clean",
      )
    } else {
      (
        format!(
          "Discard the changes to {}?\nNothing can bring them back.",
          file.path
        ),
        "Discard",
        "git checkout",
      )
    };
    let options = vec![
      Choice::new(label, note, GitDo::Discard(file.key())).danger(),
      Choice::close("Cancel"),
    ];
    // Opens on the way out.
    self.open(Box::new(GitChoice::new("Discard", text, options, 1)));
  }

  /// The selected change's menu, hanging from `anchor` or from its row.
  pub(super) fn git_menu(&mut self, anchor: Option<Rect>) {
    let git = &self.state.git;
    let (Some(file), Some(at)) = (git.selected_file(), git.selected_index())
    else {
      return;
    };
    let row = git
      .hits
      .rows
      .iter()
      .find(|(_, index)| *index == at)
      .map(|(area, _)| *area);
    let keymap = &self.keymap;
    let item = |label: &str, action: Action| {
      let key = keymap.key(KeymapGroup::Git, &action).cloned();
      MenuItem {
        key,
        ..MenuItem::new(label, action, keymap)
      }
    };
    let staged = file.area == Area::Staged;
    let all_staged = git.files().iter().all(|f| f.area == Area::Staged);
    let items = vec![
      item(if staged { "Unstage" } else { "Stage" }, Action::GitStage),
      item(
        if file.kind == Kind::Untracked {
          "Delete…"
        } else {
          "Discard…"
        },
        Action::GitDiscard,
      )
      .enabled(file.area == Area::Unstaged),
      item(
        if all_staged {
          "Unstage all"
        } else {
          "Stage all"
        },
        Action::GitStageAll,
      )
      .separated(),
    ];
    let title = file.path.rsplit('/').find(|part| !part.is_empty());
    let title = git::diff::clean(title.unwrap_or(&file.path));
    let anchor = anchor.or(row).unwrap_or(git.hits.side);
    self.open(Box::new(
      MenuModal::new(anchor, items).title(title).keep(row),
    ));
  }

  /// What a dialog of the git view was answered with.
  pub(super) fn git_do(&mut self, then: GitDo) {
    match then {
      GitDo::Discard(key) => {
        let file = self.state.git.files().iter().find(|f| f.key() == key);
        if let Some(file) = file.cloned() {
          self.git_op(Op::Discard(file));
        }
      }
      GitDo::Amend { push } => self.git_commit(push, true),
      GitDo::Pull(how) => self.git_pull_how(how),
      GitDo::Checkout { name, remote } => self.git_checkout(name, remote),
      GitDo::Switch {
        name,
        remote,
        changes,
      } => self.git_switch(name, remote, changes),
      GitDo::BranchMenu { name, remote } => self.git_branch_menu(name, remote),
      GitDo::NewBranch { from } => self.git_new_branch(from),
      GitDo::CreateBranch { name, from, switch } => {
        let said = if switch {
          format!("Made {name} from {from}, and checked it out.")
        } else {
          format!("Made {name} from {from}.")
        };
        self.git_op(Op::Branch(BranchOp::Create { name, from, switch }, said));
      }
      GitDo::Merge(name) => {
        let done = format!("Merged {name} into {}.", self.git_head());
        let args = ["merge", "--no-edit", name.as_str()];
        self.git_sync("merge", "merging", &args, done);
      }
      GitDo::Rebase(name) => {
        let done = format!("Rebased {} onto {name}.", self.git_head());
        let args = ["rebase", "--autostash", name.as_str()];
        self.git_sync("rebase", "rebasing", &args, done);
      }
      GitDo::Update { name, upstream } => {
        let said = format!("Updated {name} from {upstream}.");
        self.git_op(Op::Branch(BranchOp::Update { name, upstream }, said));
      }
      GitDo::DeleteBranch { name, force } => {
        let said = format!("Deleted {name}.");
        self.git_op(Op::Branch(BranchOp::Delete { name, force }, said));
      }
      GitDo::Fetch => self.git_fetch(true),
      GitDo::PopStash(index) => self.git_pop_stash(index),
      GitDo::DropStash(index) => {
        let said = "Dropped the stash.".to_string();
        self.git_op(Op::Branch(BranchOp::StashDrop(index), said));
      }
      GitDo::Abort => {
        if let Some(doing) = self.git_doing() {
          let said = format!("Gave up {doing}.");
          self.git_op(Op::Branch(BranchOp::Abort(doing), said));
        }
      }
    }
  }

  /// The checked-out branch, as it is named to the user.
  fn git_head(&self) -> String {
    match &self.state.git.snapshot {
      Some(snapshot) => head_name(&snapshot.head),
      None => String::new(),
    }
  }

  /// The merge or rebase that stopped, if one did.
  fn git_doing(&self) -> Option<&'static str> {
    self.state.git.snapshot.as_ref()?.doing
  }

  /// Lists the branches under `anchor`, or under the branch's name
  /// where the view shows it, else under the header's.
  pub(super) fn git_show_branches(&mut self, anchor: Option<Rect>) {
    let Some(snapshot) = &self.state.git.snapshot else {
      return self.toast("This project is not in a git repository.", true);
    };
    let in_view = match self.state.view {
      View::Git => self.state.git.hits.branch,
      View::Tasks => None,
    };
    let anchor = anchor
      .or(in_view)
      .or(self.state.header.branch)
      .unwrap_or_default();
    let root = snapshot.repo.root.clone();
    let seq = self.modal_seq;
    self.spawn(async move {
      let listed = git::branch::branches(&root).await;
      move |app: &mut App| match listed {
        Ok(branches) => {
          let modal = GitBranches::new(anchor, branches);
          app.open_if_current(seq, Box::new(modal));
        }
        Err(err) => app.toast(format!("git: {err}"), true),
      }
    });
  }

  /// Asks for a new branch, starting at `from` or at the one checked
  /// out unless another is picked.
  pub(super) fn git_new_branch(&mut self, from: Option<String>) {
    let Some(snapshot) = &self.state.git.snapshot else {
      return self.toast("This project is not in a git repository.", true);
    };
    let root = snapshot.repo.root.clone();
    let seq = self.modal_seq;
    self.spawn(async move {
      let listed = git::branch::branches(&root).await;
      move |app: &mut App| {
        let branches = match listed {
          Ok(branches) => branches,
          Err(err) => return app.toast(format!("git: {err}"), true),
        };
        // Where it can start: what was asked for, the branch checked
        // out, then every branch with what it follows on the remote.
        let mut bases: Vec<String> = from.into_iter().collect();
        for branch in &branches.local {
          bases.push(branch.name.clone());
          let upstream = branch.upstream.as_ref().filter(|up| !up.gone);
          bases.extend(upstream.map(|up| up.name.clone()));
        }
        bases.extend(branches.remote.iter().cloned());
        let mut seen = std::collections::HashSet::new();
        bases.retain(|base| seen.insert(base.clone()));
        if bases.is_empty() {
          return app.toast("There is no commit to start a branch from.", true);
        }
        let taken = branches.local.into_iter().map(|b| b.name).collect();
        app.open_if_current(seq, Box::new(GitNewBranch::new(bases, taken)));
      }
    });
  }

  /// Checks a branch out; with changes that are not committed, asks
  /// what becomes of them first.
  fn git_checkout(&mut self, name: String, remote: bool) {
    let changes = self.state.git.files().len();
    if changes == 0 {
      return self.git_switch(name, remote, Changes::Keep);
    }
    let text = if changes == 1 {
      "You have 1 change that is not committed.\nWhat becomes of it?"
        .to_string()
    } else {
      format!(
        "You have {changes} changes that are not committed.\nWhat becomes \
         of them?"
      )
    };
    let to = |changes| GitDo::Switch {
      name: name.clone(),
      remote,
      changes,
    };
    let options = vec![
      Choice::new("Take them along", "git switch", to(Changes::Keep)),
      Choice::new("Leave them in the stash", "git stash", to(Changes::Stash)),
      Choice::new(
        "Discard them",
        "git switch --discard-changes",
        to(Changes::Discard),
      )
      .danger(),
      Choice::close("Cancel"),
    ];
    let title = format!("Check out {}", short_name(&name, remote));
    self.open(Box::new(GitChoice::new(&title, text, options, 0)));
  }

  fn git_switch(&mut self, name: String, remote: bool, changes: Changes) {
    let short = short_name(&name, remote).to_string();
    let (carry, said) = match changes {
      Changes::Keep => (Carry::Keep, format!("Checked out {short}.")),
      Changes::Stash => {
        // First: the checkout only follows if this works.
        let message = format!("left behind for {short}");
        let stash = BranchOp::StashPush(message);
        self.git.ops.push_back(Op::Branch(stash, String::new()));
        let said =
          format!("Checked out {short}. Your changes are in the stash.");
        (Carry::Keep, said)
      }
      Changes::Discard => (
        Carry::Discard,
        format!("Checked out {short}. Your changes were discarded."),
      ),
    };
    let op = BranchOp::Switch {
      name,
      remote,
      carry,
    };
    self.git_op(Op::Branch(op, said));
  }

  /// What can be done with a branch, as it stands by now.
  fn git_branch_menu(&mut self, name: String, remote: bool) {
    let Some(snapshot) = &self.state.git.snapshot else {
      return;
    };
    let root = snapshot.repo.root.clone();
    let head = head_name(&snapshot.head);
    let seq = self.modal_seq;
    self.spawn(async move {
      let listed = git::branch::branches(&root).await;
      move |app: &mut App| {
        let branches = listed.unwrap_or_default();
        let branch = branches.local.iter().find(|b| b.name == name && !remote);
        let current = branch.is_some_and(|branch| branch.current);
        let upstream = branch.and_then(|branch| branch.upstream.as_ref());
        let mut options = Vec::new();
        if !current {
          let checkout = GitDo::Checkout {
            name: name.clone(),
            remote,
          };
          let label = if remote {
            "Check out as a local branch"
          } else {
            "Check out"
          };
          options.push(Choice::new(label, "git switch", checkout));
          options.push(Choice::new(
            &format!("Merge into {head}"),
            "git merge",
            GitDo::Merge(name.clone()),
          ));
          options.push(Choice::new(
            &format!("Rebase {head} onto it"),
            "git rebase",
            GitDo::Rebase(name.clone()),
          ));
          let behind = upstream.filter(|up| !up.gone && up.behind > 0);
          if let Some(upstream) = behind {
            let update = GitDo::Update {
              name: name.clone(),
              upstream: upstream.name.clone(),
            };
            let label = "Update without checking out";
            options.push(Choice::new(label, "git fetch", update));
          }
        }
        let from = GitDo::NewBranch {
          from: Some(name.clone()),
        };
        options.push(Choice::new(
          "New branch from here…",
          "git switch -c",
          from,
        ));
        if branch.is_some() && !current {
          let delete = GitDo::DeleteBranch {
            name: name.clone(),
            force: false,
          };
          options.push(Choice::new("Delete", "git branch -d", delete).danger());
        }
        options.push(Choice::close("Cancel"));
        let text = match (branch, upstream) {
          (None, _) => "A branch of the remote, with no local one.".to_string(),
          (Some(_), None) => "A local branch, not published.".to_string(),
          (Some(_), Some(up)) if up.gone => {
            format!("{} is gone from the remote.", up.name)
          }
          (Some(_), Some(up)) if up.ahead == 0 && up.behind == 0 => {
            format!("Level with {}.", up.name)
          }
          (Some(_), Some(up)) => format!(
            "{} to push to {}, {} to pull.",
            commits(up.ahead),
            up.name,
            commits(up.behind)
          ),
        };
        let title = if current {
          format!("{name}, checked out")
        } else {
          name.clone()
        };
        let modal = GitChoice::new(&title, text, options, 0);
        app.open_if_current(seq, Box::new(modal));
      }
    });
  }

  /// Puts every change aside in the stash.
  pub(super) fn git_stash(&mut self) {
    let changes = self.state.git.files().len();
    if changes == 0 {
      return self.toast("There is nothing to stash.", false);
    }
    let said = if changes == 1 {
      "Stashed 1 change.".to_string()
    } else {
      format!("Stashed {changes} changes.")
    };
    let message = "put aside from the console".to_string();
    self.git_op(Op::Branch(BranchOp::StashPush(message), said));
  }

  /// Brings a stash back.
  pub(super) fn git_pop_stash(&mut self, index: usize) {
    let stashed = match &self.state.git.snapshot {
      Some(snapshot) => snapshot.stashes.len(),
      None => 0,
    };
    if index >= stashed {
      return self.toast("The stash is empty.", false);
    }
    let said = "Brought the stash back.".to_string();
    self.git_op(Op::Branch(BranchOp::StashPop(index), said));
  }

  /// A stash's own choices: bring it back, or throw it away.
  fn git_stash_menu(&mut self, index: usize) {
    let message = self
      .state
      .git
      .snapshot
      .as_ref()
      .and_then(|snapshot| snapshot.stashes.get(index))
      .map(|stash| stash.message.clone());
    let Some(message) = message else {
      return;
    };
    let options = vec![
      Choice::new("Bring it back", "git stash pop", GitDo::PopStash(index)),
      Choice::new("Drop it", "git stash drop", GitDo::DropStash(index))
        .danger(),
      Choice::close("Cancel"),
    ];
    self.open(Box::new(GitChoice::new("Stash", message, options, 0)));
  }

  /// Asks before giving up the merge or rebase that stopped.
  pub(super) fn git_ask_abort(&mut self) {
    let Some(doing) = self.git_doing() else {
      return self.toast("No merge or rebase is under way.", false);
    };
    let text = format!(
      "Give up {doing}?\nThe repository goes back to where it was before, \
       and what was resolved so far is lost."
    );
    let options = vec![
      Choice::new("Give it up", "--abort", GitDo::Abort).danger(),
      Choice::close("Cancel"),
    ];
    self.open(Box::new(GitChoice::new("Abort", text, options, 1)));
  }

  /// Goes on with a rebase once nothing is left in conflict. A merge is
  /// finished by committing.
  pub(super) fn git_continue(&mut self) {
    let Some(doing) = self.git_doing() else {
      return self.toast("No merge or rebase is under way.", false);
    };
    let conflicts = self
      .state
      .git
      .files()
      .iter()
      .any(|file| file.area == Area::Conflict);
    if conflicts {
      return self
        .toast("Resolve the conflicts and stage those files first.", true);
    }
    if doing == "merging" {
      self.state.git.focus = GitFocus::Message;
      return self.toast("A merge is finished by committing it.", false);
    }
    let said = format!("Went on {doing}.");
    self.git_op(Op::Branch(BranchOp::GoOn(doing), said));
  }

  /// Makes the next commit replace the last one, or not. Its message
  /// is what the field starts with, to change it.
  pub(super) fn git_toggle_amend(&mut self) {
    let git = &mut self.state.git;
    git.amend = !git.amend;
    let Some(snapshot) = &git.snapshot else {
      return;
    };
    if !git.amend || !git.message.is_blank() {
      return;
    }
    let root = snapshot.repo.root.clone();
    self.spawn(async move {
      let message = git::last_message(&root).await;
      move |app: &mut App| {
        let git = &mut app.state.git;
        if let Some(message) = message
          && git.amend
          && git.message.is_blank()
        {
          git.message.set(&message);
          app.dirty = true;
        }
      }
    });
  }

  /// Commits what is staged with the message typed, then pushes with
  /// `push`. `sure` is the answer to what it asks before amending a
  /// commit the remote has.
  pub(super) fn git_commit(&mut self, push: bool, sure: bool) {
    let git = &self.state.git;
    let Some(snapshot) = &git.snapshot else {
      return;
    };
    if git.committing {
      return;
    }
    if git.syncing.is_some() {
      return self.toast("A pull or a push is still running.", true);
    }
    let staged = snapshot.files.iter().any(|f| f.area == Area::Staged);
    if !staged && !git.amend {
      return self
        .toast("Nothing is staged: select a change and press space.", true);
    }
    if git.message.is_blank() {
      self.state.git.focus = GitFocus::Message;
      return self.toast("Type a commit message first.", true);
    }
    // The remote has the commit an amend would replace.
    let published = snapshot
      .upstream
      .as_ref()
      .filter(|up| !up.gone && up.ahead == 0);
    if let (true, false, Some(upstream)) = (git.amend, sure, published) {
      let text = format!(
        "The last commit is on {} already.\nAmending it sets your branch \
         and the remote apart: a push will be refused until it is forced.",
        upstream.name
      );
      let options = vec![
        Choice::new(
          "Amend anyway",
          "git commit --amend",
          GitDo::Amend { push },
        )
        .danger(),
        Choice::close("Cancel"),
      ];
      return self.open(Box::new(GitChoice::new("Amend", text, options, 1)));
    }
    let op = Op::Commit {
      message: git.message.text(),
      amend: git.amend,
      push,
    };
    self.state.git.committing = true;
    self.git_op(op);
  }

  /// Whether a pull or a push can start; where not, a toast says why.
  fn git_can_sync(&mut self) -> bool {
    if self.state.git.snapshot.is_none() {
      self.toast("This project is not in a git repository.", true);
      return false;
    }
    if self.state.git.syncing.is_some() {
      self.toast("A pull or a push is still running.", true);
      return false;
    }
    true
  }

  /// Pulls from the upstream. Where the two branches have each commits
  /// the other does not and the user's config does not say how to bring
  /// them together, it asks.
  pub(super) fn git_pull(&mut self) {
    if !self.git_can_sync() {
      return;
    }
    let Some(snapshot) = &self.state.git.snapshot else {
      return;
    };
    let upstream = match (&snapshot.head, &snapshot.upstream) {
      (Head::Branch(_), Some(upstream)) if !upstream.gone => upstream,
      _ => {
        return self.toast(
          "There is no upstream to pull from: push to publish the branch.",
          true,
        );
      }
    };
    if upstream.ahead == 0 || upstream.behind == 0 {
      return self.git_pull_how(PullHow::AsConfigured);
    }
    let text = format!(
      "Your branch has {} that {} does not, and it has {} that yours \
       does not.\nYour git config does not say how to bring them together.",
      commits(upstream.ahead),
      upstream.name,
      commits(upstream.behind),
    );
    let root = snapshot.repo.root.clone();
    let seq = self.modal_seq;
    self.spawn(async move {
      let configured = git::pull_is_configured(&root).await;
      move |app: &mut App| {
        if configured {
          return app.git_pull_how(PullHow::AsConfigured);
        }
        let options = vec![
          Choice::new(
            "Rebase mine on top",
            "git pull --rebase",
            GitDo::Pull(PullHow::Rebase),
          ),
          Choice::new(
            "Merge the two",
            "git pull --no-rebase",
            GitDo::Pull(PullHow::Merge),
          ),
          Choice::close("Cancel"),
        ];
        let modal = GitChoice::new("Pull", text, options, 0);
        app.open_if_current(seq, Box::new(modal));
      }
    });
  }

  fn git_pull_how(&mut self, how: PullHow) {
    if !self.git_can_sync() {
      return;
    }
    let Some(snapshot) = &self.state.git.snapshot else {
      return;
    };
    let from = match &snapshot.upstream {
      Some(upstream) => upstream.name.clone(),
      None => return,
    };
    // A merge commit takes the message git proposes: an editor would
    // open in the task's terminal and wait there.
    let args: &[&str] = match how {
      PullHow::AsConfigured => &["pull", "--no-edit"],
      PullHow::Rebase => &["pull", "--rebase", "--autostash"],
      PullHow::Merge => &["pull", "--no-rebase", "--no-edit"],
    };
    self.git_sync("pull", "pulling", args, format!("Pulled from {from}."));
  }

  /// Pushes to the upstream, or publishes a branch that has none.
  /// `committed` is a push right after a commit, which the snapshot
  /// does not count yet.
  pub(super) fn git_push(&mut self, committed: bool) {
    if !self.git_can_sync() {
      return;
    }
    let Some(snapshot) = &self.state.git.snapshot else {
      return;
    };
    let Head::Branch(branch) = &snapshot.head else {
      return self.toast("Check out a branch to push.", true);
    };
    let Some(remote) = &snapshot.remote else {
      return self.toast("The repository has no remote to push to.", true);
    };
    match &snapshot.upstream {
      Some(upstream) if !upstream.gone => {
        if upstream.behind > 0 {
          let why = format!(
            "{} has {} you do not have: pull first.",
            upstream.name,
            commits(upstream.behind)
          );
          return self.toast(why, true);
        }
        if upstream.ahead == 0 && !committed {
          return self.toast("Nothing to push.", false);
        }
        let done = format!("Pushed to {}.", upstream.name);
        self.git_sync("push", "pushing", &["push"], done);
      }
      Some(_) | None => {
        let done = format!("Published {branch} to {remote}.");
        let (remote, branch) = (remote.clone(), branch.clone());
        let args = ["push", "-u", remote.as_str(), branch.as_str()];
        self.git_sync("push", "pushing", &args, done);
      }
    }
  }

  /// Runs `git` with `args` as a task of the runner, at `git-pull` or
  /// `git-push`, and follows it: once it ends well it is removed and
  /// `done` is said; one that fails stays, and is shown.
  fn git_sync(
    &mut self,
    what: &'static str,
    doing: &'static str,
    args: &[&str],
    done: String,
  ) {
    if !self.git_can_sync() {
      return;
    }
    let Some(snapshot) = &self.state.git.snapshot else {
      return;
    };
    let root = snapshot.repo.root.to_string_lossy().into_owned();
    let path = format!("git-{what}");
    let mut argv = vec!["git".to_string()];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    let mut spec = TaskSpec::with_cmd(argv);
    spec.set_label(Some(&format!("git {what}")));
    spec.set("type", Some("job".into()));
    spec.set("cwd", Some(root.into()));
    self.state.git.syncing = Some(Syncing {
      doing,
      path: path.clone(),
      since: Instant::now(),
    });
    self.dirty = true;
    let pc = self.pc.clone();
    let config = self.project.config();
    self.spawn(async move {
      let run = async {
        let target = Target::glob(&path);
        let selector = target
          .selector()
          .map_err(|_| format!("No task can be at {path}"))?;
        // The one before, kept for what it said, makes room.
        let remove = Command::Remove {
          target: target.clone(),
        };
        let _ = execute(&pc, &config, &remove).await;
        let mut active = pc.watch_active(selector);
        let add = Command::Add {
          target,
          spec,
          place: Default::default(),
          start: true,
          defaults: true,
          origin: None,
        };
        execute(&pc, &config, &add)
          .await
          .map_err(|err| err.to_string())?;
        while active.recv().await == Some(true) {}
        Ok::<_, String>(())
      };
      let ran = run.await;
      move |app: &mut App| app.git_synced(what, &path, ran, done)
    });
  }

  fn git_synced(
    &mut self,
    what: &'static str,
    path: &str,
    ran: Result<(), String>,
    done: String,
  ) {
    self.state.git.syncing = None;
    self.git.stale = true;
    self.dirty = true;
    if let Err(message) = ran {
      return self.toast(format!("git {what}: {message}"), true);
    }
    let ended = self
      .state
      .tasks
      .iter()
      .find(|task| task.path.as_str() == path)
      .map(|task| task.exit_code());
    match ended {
      Some(Some(0)) => {
        self.issue(Command::Remove {
          target: Target::glob(path),
        });
        self.toast(done, false);
      }
      // What it said is on its screen: the task view shows it.
      Some(_) => {
        self.show_view(View::Tasks);
        self.state.tasks.select_path(path);
        self.state.scope = Scope::Tasks;
        self.toast(
          format!(
            "git {what} failed: this is what it said. 2 is the git view."
          ),
          true,
        );
      }
      // Removed meanwhile, by hand.
      None => (),
    }
  }

  /// Shows the terminal of the pull or push under way, with the focus:
  /// to see what it waits for, and to type a password.
  pub(super) fn git_show_sync(&mut self) {
    let Some(syncing) = &self.state.git.syncing else {
      return self.toast("No pull or push is running.", false);
    };
    let path = syncing.path.clone();
    self.show_view(View::Tasks);
    self.state.tasks.select_path(&path);
    self.state.scope = Scope::Term;
  }
}
