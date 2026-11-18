//! The git view's state: the repository as last read, the change
//! selected in it, and where the diff is scrolled to. Reading the
//! repository is the console's (`app_git`); drawing it is `ui_git`.

use crate::git::{Diff, FileChange, FileKey, Read, Snapshot};
use crate::term::grid::Rect;

/// Which panel the keys go to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GitFocus {
  #[default]
  Files,
  Diff,
}

impl GitFocus {
  pub fn toggle(self) -> Self {
    match self {
      GitFocus::Files => GitFocus::Diff,
      GitFocus::Diff => GitFocus::Files,
    }
  }
}

#[derive(Default)]
pub struct GitView {
  /// A read came back: until one does, nothing is known, not even
  /// whether there is a repository.
  pub loaded: bool,
  /// None where the project is not in a repository.
  pub snapshot: Option<Snapshot>,
  pub selected: Option<FileKey>,
  /// Where the selected change is in the list, to stay near it when it
  /// goes.
  selected_at: usize,
  /// The diff last read, and the change it is of: shown only while that
  /// one is selected.
  pub diff: Option<(FileKey, Diff)>,
  pub focus: GitFocus,
  /// The first line of the diff shown.
  pub scroll: usize,
  /// How many columns of the diff's text are off to the left.
  pub scroll_x: usize,
  /// The first row of the list of changes shown.
  pub list_top: usize,
  pub fetching: bool,
  /// Why the last fetch failed.
  pub fetch_error: Option<String>,
  pub hits: GitHits,
}

/// Where things were last drawn, for the mouse.
#[derive(Default)]
pub struct GitHits {
  /// Each change's row, with its place in the snapshot's files.
  pub rows: Vec<(Rect, usize)>,
  pub fetch: Option<Rect>,
  /// The panels on the left.
  pub side: Rect,
  /// The diff's lines.
  pub diff: Rect,
}

impl GitView {
  pub fn files(&self) -> &[FileChange] {
    match &self.snapshot {
      Some(snapshot) => &snapshot.files,
      None => &[],
    }
  }

  /// The diff of the selected change, once it is read.
  pub fn shown_diff(&self) -> Option<&Diff> {
    match (&self.diff, &self.selected) {
      (Some((key, diff)), Some(selected)) if key == selected => Some(diff),
      _ => None,
    }
  }

  /// Takes what a read found. True when the picture changed.
  pub fn take(&mut self, read: Read) -> bool {
    let mut changed = !self.loaded;
    self.loaded = true;
    if self.snapshot != read.snapshot {
      self.snapshot = read.snapshot;
      changed = true;
    }
    changed |= self.reselect();
    if let Some((key, diff)) = read.diff
      && self.selected.as_ref() == Some(&key)
    {
      let new = Some((key, diff));
      if self.diff != new {
        self.diff = new;
        changed = true;
      }
    }
    changed
  }

  /// Keeps the selection on a change that is there: the same one, the
  /// same file where it is now (staged since, or not any more), or what
  /// took its place in the list. True when it moved.
  fn reselect(&mut self) -> bool {
    let files = self.files();
    let at = match &self.selected {
      Some(key) => files
        .iter()
        .position(|file| file.key() == *key)
        .or_else(|| files.iter().position(|file| file.path == key.path))
        .or_else(|| {
          (!files.is_empty()).then(|| self.selected_at.min(files.len() - 1))
        }),
      None => (!files.is_empty()).then_some(0),
    };
    let selected = at.map(|at| files[at].key());
    if selected == self.selected {
      self.selected_at = at.unwrap_or(0);
      return false;
    }
    self.set_selected(at);
    true
  }

  fn set_selected(&mut self, at: Option<usize>) {
    self.selected = at.map(|at| self.files()[at].key());
    self.selected_at = at.unwrap_or(0);
    self.scroll = 0;
    self.scroll_x = 0;
  }

  /// Selects the change `at` in the files. True when it was not.
  pub fn select(&mut self, at: usize) -> bool {
    if at >= self.files().len() || self.selected == Some(self.files()[at].key())
    {
      return false;
    }
    self.set_selected(Some(at));
    true
  }

  /// Moves the selection by `by` changes, stopping at the ends. True
  /// when it moved.
  pub fn step(&mut self, by: isize) -> bool {
    let len = self.files().len();
    if len == 0 {
      return false;
    }
    let at = match &self.selected {
      Some(_) => self.selected_at.saturating_add_signed(by).min(len - 1),
      None => 0,
    };
    self.select(at)
  }

  /// Scrolls the diff by `lines`, down when positive. The view clamps it
  /// to the diff when it draws.
  pub fn scroll_by(&mut self, lines: isize) {
    let len = self.shown_diff().map_or(0, |diff| diff.lines.len());
    let last = len.saturating_sub(usize::from(self.hits.diff.height.max(1)));
    self.scroll = self.scroll.saturating_add_signed(lines).min(last);
  }

  pub fn scroll_x_by(&mut self, cols: isize) {
    self.scroll_x = self.scroll_x.saturating_add_signed(cols).min(2000);
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::*;
  use crate::git::{Area, DiffLine, Head, Kind, Repo};

  fn change(path: &str, area: Area) -> FileChange {
    FileChange {
      path: path.to_string(),
      orig: None,
      kind: Kind::Modified,
      area,
      added: Some(1),
      removed: None,
    }
  }

  fn snapshot(files: Vec<FileChange>) -> Snapshot {
    Snapshot {
      repo: Repo {
        root: PathBuf::from("/repo"),
        git_dir: PathBuf::from("/repo/.git"),
        common_dir: PathBuf::from("/repo/.git"),
      },
      head: Head::Branch("main".to_string()),
      upstream: None,
      files,
      fetched: None,
      has_remote: false,
      doing: None,
    }
  }

  fn read(files: Vec<FileChange>) -> Read {
    Read {
      snapshot: Some(snapshot(files)),
      diff: None,
    }
  }

  fn selected(view: &GitView) -> Option<(&str, Area)> {
    view
      .selected
      .as_ref()
      .map(|key| (key.path.as_str(), key.area))
  }

  #[test]
  fn the_first_change_is_selected_and_the_same_read_changes_nothing() {
    let mut view = GitView::default();
    let files = || vec![change("a", Area::Staged), change("b", Area::Unstaged)];
    assert!(view.take(read(files())));
    assert_eq!(selected(&view), Some(("a", Area::Staged)));
    assert!(!view.take(read(files())));
    assert!(view.step(1));
    assert_eq!(selected(&view), Some(("b", Area::Unstaged)));
    assert!(!view.step(1), "the last one");
  }

  /// A file staged elsewhere stays selected where it went; one that is
  /// gone leaves the selection on what took its place.
  #[test]
  fn the_selection_follows_the_file() {
    let mut view = GitView::default();
    view.take(read(vec![
      change("a", Area::Unstaged),
      change("b", Area::Unstaged),
      change("c", Area::Unstaged),
    ]));
    view.select(1);
    view.scroll = 7;
    assert!(view.take(read(vec![
      change("b", Area::Staged),
      change("a", Area::Unstaged),
      change("c", Area::Unstaged),
    ])));
    assert_eq!(selected(&view), Some(("b", Area::Staged)));
    assert_eq!(view.scroll, 0, "another diff");

    view.select(2);
    view.take(read(vec![change("a", Area::Unstaged)]));
    assert_eq!(selected(&view), Some(("a", Area::Unstaged)));
    view.take(read(Vec::new()));
    assert_eq!(view.selected, None);
    view.take(Read::default());
    assert!(view.snapshot.is_none());
  }

  /// A diff read for a change that is not selected any more is not
  /// shown, and one for the selected change keeps the scroll.
  #[test]
  fn only_the_selected_change_s_diff_is_shown() {
    let mut view = GitView::default();
    let files =
      || vec![change("a", Area::Unstaged), change("b", Area::Unstaged)];
    view.take(read(files()));
    let diff = |text: &str| Diff {
      lines: vec![DiffLine::Note(text.to_string())],
      note: None,
    };
    let key = |path: &str| FileKey {
      path: path.to_string(),
      area: Area::Unstaged,
    };
    assert!(view.shown_diff().is_none());
    assert!(view.take(Read {
      snapshot: Some(snapshot(files())),
      diff: Some((key("a"), diff("of a"))),
    }));
    assert_eq!(view.shown_diff(), Some(&diff("of a")));
    view.scroll = 3;
    assert!(view.take(Read {
      snapshot: Some(snapshot(files())),
      diff: Some((key("a"), diff("of a, edited"))),
    }));
    assert_eq!(view.scroll, 3);

    view.step(1);
    assert!(view.shown_diff().is_none());
    assert!(!view.take(Read {
      snapshot: Some(snapshot(files())),
      diff: Some((key("a"), diff("late"))),
    }));
    assert!(view.shown_diff().is_none());
  }
}
