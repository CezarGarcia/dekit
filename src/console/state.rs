use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use tui_input::Input;

use crate::console::{
  keymap::KeymapGroup, task_tree::TaskTree, task_view::TaskView,
  ui_header::HeaderHits,
};
use crate::kernel::{task::TaskId, task_path::TaskPath};
use crate::term::grid::Rect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
  Tasks,
  Term,
  TermZoom,
}

impl Scope {
  pub fn toggle(&self) -> Self {
    match self {
      Scope::Tasks => Scope::Term,
      Scope::Term => Scope::Tasks,
      Scope::TermZoom => Scope::Tasks,
    }
  }

  pub fn is_zoomed(&self) -> bool {
    match self {
      Scope::Tasks => false,
      Scope::Term => false,
      Scope::TermZoom => true,
    }
  }

  pub fn is_term(&self) -> bool {
    match self {
      Scope::Tasks => false,
      Scope::Term => true,
      Scope::TermZoom => true,
    }
  }
}

pub struct State {
  pub scope: Scope,
  pub tasks: TaskTree,
  pub hide_keymap_window: bool,
  pub quitting: bool,
  /// The clickable parts of the header as last drawn.
  pub header: HeaderHits,
  /// The unsaved changes on the task list's border as last drawn; a
  /// click opens the write-back dialog.
  pub unsaved_hit: Option<Rect>,
  /// Where the mouse was last seen, for hover highlights.
  pub hover: Option<(u16, u16)>,
  /// A line at the bottom of the task list for a few seconds.
  pub toast: Option<Toast>,
  /// What the config on disk would change, and what changed here only.
  pub config: ConfigStatus,
  /// A task or group being dragged in the list.
  pub drag: Option<Drag>,
  /// The find bar, while it is open.
  pub search: Option<SearchBar>,
  /// What the find bar had when it last closed: it opens with it again.
  pub last_search: String,
}

/// The find bar on a task's terminal: what is typed is looked for in the
/// task's output, which copy mode holds still meanwhile.
pub struct SearchBar {
  pub task: TaskId,
  pub input: Input,
  /// The value is shown selected, as the bar opens with the last query:
  /// typing replaces it, moving the cursor keeps it.
  pub selected: bool,
  /// The bar put the task in copy mode: closing it empty leaves it.
  pub entered_copy: bool,
}

pub struct Toast {
  pub text: String,
  pub error: bool,
  pub until: Instant,
}

#[derive(Default)]
pub struct ConfigStatus {
  /// The config does not load; its changes are unknown then.
  pub broken: bool,
  /// Changes a reload has to offer.
  pub changes: usize,
  /// Config tasks and folders changed here: marked in the list.
  pub edited: HashSet<TaskId>,
  /// Config tasks and folders removed here, still in the config.
  pub removed: usize,
  /// The runner's own tasks, not in the config.
  pub own: HashSet<TaskId>,
}

impl ConfigStatus {
  /// Changes here a write-back has to offer.
  pub fn unsaved(&self) -> usize {
    self.edited.len() + self.removed
  }
}

/// A press on a row that may become a drag.
pub struct Drag {
  pub observer: crate::kernel::task_screen::ObserverId,
  /// The path pressed; its row is looked up as the list is now, since
  /// the list can change while the button is held.
  pub from: Arc<TaskPath>,
  /// Moved off the row: a drag, not a click.
  pub active: bool,
  pub drop: Option<Drop>,
}

/// Where a drag would put what it carries: under `parent`, right after
/// `after` or first, shown as a line under row `row` at `parent`'s depth.
#[derive(Clone, Debug, PartialEq)]
pub struct Drop {
  pub parent: TaskPath,
  pub after: Option<TaskPath>,
  /// The row the line goes under; none for the top of the list.
  pub row: Option<usize>,
}

impl State {
  pub fn current_task(&self) -> Option<&TaskView> {
    self.tasks.current()
  }

  pub fn keymap_group(&self) -> KeymapGroup {
    match self.scope {
      Scope::Tasks => KeymapGroup::Tasks,
      Scope::Term | Scope::TermZoom => match self.current_task() {
        Some(task) if task.present.is_some() => KeymapGroup::Copy,
        _ => KeymapGroup::Term,
      },
    }
  }
}
