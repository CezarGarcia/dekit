//! Draws the git view: the branch and the changes on the left, each
//! panel under its title bar, and the selected change's diff on the
//! right.

use std::time::{Duration, SystemTime};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::console::action::Action;
use crate::console::git_view::{GitFocus, GitHits, GitView};
use crate::console::keymap::{Keymap, KeymapGroup};
use crate::console::state::State;
use crate::console::theme::{GitTheme, Theme};
use crate::console::widgets::wrap::wrap;
use crate::git::diff::clean;
use crate::git::{Area, Diff, DiffLine, FileChange, Head, Kind, Snapshot};
use crate::term::{
  Grid, Rgb,
  grid::{Pos, Rect},
  style::Style,
};

/// Narrower than this, one panel is shown at a time: the one with the
/// focus.
const NARROW: u16 = 72;
/// Rows of the branch panel under its title bar.
const BRANCH_ROWS: u16 = 5;
/// How far a button's background moves toward the text color under the
/// mouse.
const HOVER: u8 = 28;

fn on(bg: Rgb, fg: Rgb) -> Style {
  Style::default().bg(bg.into()).fg(fg.into())
}

/// `text` cut to `width` cells, with `…` in place of what does not fit.
fn fit(text: &str, width: u16) -> String {
  let width = usize::from(width);
  if text.width() <= width {
    return text.to_string();
  }
  let mut out = String::new();
  let mut used = 0;
  for c in text.chars() {
    let w = c.width().unwrap_or(0);
    if used + w + 1 > width {
      break;
    }
    out.push(c);
    used += w;
  }
  if width > 0 {
    out.push('…');
  }
  out
}

/// A directory cut to `width` cells by leaving out its first parts:
/// `…/src/dashboard`. Nothing when not even its last part fits.
fn fit_dir(dir: &str, width: u16) -> String {
  if dir.width() <= usize::from(width) {
    return dir.to_string();
  }
  let mut rest = dir;
  while let Some((_, tail)) = rest.split_once('/') {
    if tail.width() + 2 <= usize::from(width) {
      return format!("…/{tail}");
    }
    rest = tail;
  }
  String::new()
}

/// A path's directory and its name; an untracked directory keeps its
/// `/`.
fn split_path(path: &str) -> (&str, &str) {
  let trimmed = path.strip_suffix('/').unwrap_or(path);
  match trimmed.rfind('/') {
    Some(at) => (&path[..at], &path[at + 1..]),
    None => ("", path),
  }
}

/// How long ago, in the largest unit that fits.
pub fn ago(elapsed: Duration) -> String {
  let secs = elapsed.as_secs();
  match secs {
    0..60 => "just now".to_string(),
    60..3600 => format!("{}m ago", secs / 60),
    3600..172_800 => format!("{}h ago", secs / 3600),
    _ => format!("{}d ago", secs / 86_400),
  }
}

/// The checked-out branch as the header and the branch panel name it.
pub fn head_name(head: &Head) -> String {
  match head {
    Head::Branch(name) | Head::Unborn(name) => clean(name),
    Head::Detached(commit) => format!("detached at {commit}"),
  }
}

/// How the branch stands with its upstream, in a few cells: `↑2 ↓3`,
/// `✓` when level, nothing when there is no upstream to compare with.
pub fn sync_mark(snapshot: &Snapshot) -> String {
  match &snapshot.upstream {
    Some(upstream) if upstream.gone => "gone".to_string(),
    Some(upstream) => match (upstream.ahead, upstream.behind) {
      (0, 0) => "✓".to_string(),
      (ahead, 0) => format!("↑{ahead}"),
      (0, behind) => format!("↓{behind}"),
      (ahead, behind) => format!("↑{ahead} ↓{behind}"),
    },
    None => String::new(),
  }
}

fn letter_color(kind: Kind, t: &GitTheme) -> Rgb {
  match kind {
    Kind::Modified => t.warn,
    Kind::Added => t.add,
    Kind::Deleted | Kind::Conflict => t.del,
    Kind::Renamed | Kind::Copied | Kind::TypeChanged | Kind::Untracked => {
      t.info
    }
  }
}

/// A panel's title bar; the rest of the row is returned for what goes
/// at its right end.
fn title_bar(
  grid: &mut Grid,
  mut row: Rect,
  title: &str,
  focused: bool,
  t: &GitTheme,
) -> Rect {
  grid.fill_area(row, ' ', on(t.bar, t.dim));
  let mark = row.take_left(2);
  if focused {
    grid.draw_text(mark, "▎", on(t.bar, t.accent));
  }
  let color = if focused { t.accent } else { t.dim };
  let title_area = row.take_left(title.width() as u16);
  grid.draw_text(title_area, title, on(t.bar, color).set_bold(true));
  row
}

pub fn render_git(
  area: Rect,
  grid: &mut Grid,
  state: &mut State,
  keymap: &Keymap,
  theme: &Theme,
  now: SystemTime,
) {
  let t = &theme.git;
  let mut body = area;
  let hints = body.take_bottom(u16::from(!state.hide_keymap_window));
  grid.fill_area(body, ' ', on(t.bg, t.text));
  let hover = state.hover;
  let git = &mut state.git;

  let single = body.width < NARROW;
  let side_width = match (single, git.focus) {
    (true, GitFocus::Files) => body.width,
    (true, GitFocus::Diff) => 0,
    (false, _) => (u32::from(body.width) * 36 / 100).clamp(34, 50) as u16,
  };
  let mut main = body;
  let mut side = main.take_left(side_width);
  if !single {
    let rule = side.take_right(1);
    grid.fill_area(rule, '│', on(t.surface, t.border));
  }

  // The diff is scrolled within what it has: its lines under its bar.
  let page = usize::from(main.height.saturating_sub(1));
  let lines = git.shown_diff().map_or(0, |diff| diff.lines.len());
  git.scroll = git.scroll.min(lines.saturating_sub(page));

  let mut hits = GitHits::default();
  if side.width > 0 && side.height > 0 {
    hits.side = side;
    git.list_top = render_side(side, grid, git, &mut hits, hover, t, now);
  }
  if main.width > 0 && main.height > 0 {
    render_diff(main, grid, git, &mut hits, t);
  }
  git.hits = hits;

  render_hints(hints, grid, git.focus, keymap, t);

  if let Some(toast) = &state.toast {
    // At the bottom of the panel on the left, as many rows as it takes.
    let within = if side.width > 0 { side } else { main };
    let lines = wrap(&toast.text, within.width.saturating_sub(2));
    let height = (lines.len() as u16).min(within.height);
    let area = Rect {
      y: within.bottom().saturating_sub(height),
      height,
      ..within
    };
    let style = if toast.error {
      on(theme.block.alert_bg, theme.block.alert)
    } else {
      on(t.accent_bg, t.accent_fg)
    };
    grid.fill_area(area, ' ', style);
    for (row, line) in area.inner((0, 1)).rows().zip(&lines) {
      grid.draw_text(row, line, style);
    }
  }
}

/// The branch panel over the list of changes. Returns the list's first
/// row shown, moved to keep the selected change on screen.
fn render_side(
  mut area: Rect,
  grid: &mut Grid,
  git: &GitView,
  hits: &mut GitHits,
  hover: Option<(u16, u16)>,
  t: &GitTheme,
  now: SystemTime,
) -> usize {
  let base = on(t.surface, t.text);
  let dim = on(t.surface, t.dim);
  grid.fill_area(area, ' ', base);
  title_bar(grid, area.take_top(1), "BRANCH", false, t);
  let Some(snapshot) = &git.snapshot else {
    let text = if git.loaded {
      "This project is not in a git repository."
    } else {
      "Reading the repository…"
    };
    let lines = wrap(text, area.width.saturating_sub(4));
    for (row, line) in area.inner((1, 2)).rows().zip(&lines) {
      grid.draw_text(row, line, dim);
    }
    return 0;
  };

  let mut branch = area.take_top(BRANCH_ROWS).inner((1, 1, 1, 2));
  render_branch(&mut branch, grid, git, snapshot, hits, hover, t, now);

  let files = &snapshot.files;
  let mut bar = title_bar(
    grid,
    area.take_top(1),
    "CHANGES",
    git.focus == GitFocus::Files,
    t,
  );
  if !files.is_empty() {
    let count = format!("{} ", files.len());
    grid.draw_text(
      bar.take_right(count.width() as u16),
      &count,
      on(t.bar, t.dim),
    );
  }
  if files.is_empty() {
    let mut text = area.inner((1, 2));
    grid.draw_text(
      text.take_top(1),
      "✓ Nothing to commit",
      on(t.surface, t.add),
    );
    return 0;
  }

  // The list: each group under its name, a blank row between them.
  enum Row {
    Gap,
    Group(&'static str, usize, Rgb),
    File(usize),
  }
  let mut rows = vec![Row::Gap];
  let mut selected_row = None;
  for (label, area_of, color) in [
    ("Conflicts", Area::Conflict, t.del),
    ("Staged", Area::Staged, t.dim),
    ("Unstaged", Area::Unstaged, t.dim),
  ] {
    let of_area = files.iter().enumerate().filter(|(_, f)| f.area == area_of);
    let count = of_area.clone().count();
    if count == 0 {
      continue;
    }
    rows.push(Row::Group(label, count, color));
    for (at, file) in of_area {
      if git.selected.as_ref() == Some(&file.key()) {
        selected_row = Some(rows.len());
      }
      rows.push(Row::File(at));
    }
    rows.push(Row::Gap);
  }
  let height = usize::from(area.height);
  let mut top = git.list_top.min(rows.len().saturating_sub(height));
  if let Some(row) = selected_row {
    // With the rows above it: its group's name, when it is the first.
    if row <= top {
      top = row.saturating_sub(2);
    } else if row >= top + height {
      top = row + 1 - height;
    }
  }

  let width_of = |count: Option<u32>| match count {
    Some(n) if n > 0 => n.to_string().len() as u16 + 1,
    Some(_) | None => 0,
  };
  let added_width = files.iter().map(|f| width_of(f.added)).max().unwrap_or(0);
  let removed_width =
    files.iter().map(|f| width_of(f.removed)).max().unwrap_or(0);
  let focused = git.focus == GitFocus::Files;
  for (row, line) in rows.iter().skip(top).zip(area.rows()) {
    match row {
      Row::Gap => (),
      Row::Group(label, count, color) => {
        let mut line = line.inner((0, 1, 0, 2));
        let label_area = line.take_left(label.width() as u16 + 1);
        grid.draw_text(label_area, label, on(t.surface, *color).set_bold(true));
        grid.draw_text(line, &count.to_string(), on(t.surface, t.faint));
      }
      Row::File(at) => {
        let file = &files[*at];
        let selected = git.selected.as_ref() == Some(&file.key());
        let hovered = hover.is_some_and(|(x, y)| line.contains(x, y));
        let bg = if selected {
          t.selected
        } else if hovered {
          t.hover
        } else {
          t.surface
        };
        hits.rows.push((line, *at));
        render_file(
          line,
          grid,
          file,
          bg,
          selected.then_some(focused),
          (added_width, removed_width),
          t,
        );
      }
    }
  }
  top
}

/// The checked-out branch, how it stands with its upstream, and the
/// fetch.
#[allow(clippy::too_many_arguments)]
fn render_branch(
  area: &mut Rect,
  grid: &mut Grid,
  git: &GitView,
  snapshot: &Snapshot,
  hits: &mut GitHits,
  hover: Option<(u16, u16)>,
  t: &GitTheme,
  now: SystemTime,
) {
  let paint = |fg: Rgb| on(t.surface, fg);

  // When it was last fetched, at the right; the name gets the rest.
  let mut line = area.take_top(1);
  let (fetched, color) = if git.fetching {
    ("fetching…".to_string(), t.accent)
  } else if git.fetch_error.is_some() {
    ("fetch failed".to_string(), t.del)
  } else {
    match snapshot.fetched {
      Some(at) if snapshot.has_remote => {
        let elapsed = now.duration_since(at).unwrap_or_default();
        (format!("fetched {}", ago(elapsed)), t.faint)
      }
      Some(_) | None => (String::new(), t.faint),
    }
  };
  let name = head_name(&snapshot.head);
  if name.width() + fetched.width() + 2 <= usize::from(line.width) {
    let at = line.take_right(fetched.width() as u16);
    grid.draw_text(at, &fetched, paint(color));
    line.take_right(2);
  }
  let name = fit(&name, line.width);
  let name_area = line.take_left(name.width() as u16);
  grid.draw_text(name_area, &name, paint(t.text).set_bold(true));
  if let Some(doing) = snapshot.doing {
    line.take_left(2);
    grid.draw_text(line, doing, paint(t.warn));
  }

  let mut line = area.take_top(1);
  let mut say = |text: &str, color: Rgb| {
    let text = fit(text, line.width);
    grid.draw_text(line.take_left(text.width() as u16), &text, paint(color));
    line.take_left(3);
  };
  match (&snapshot.head, &snapshot.upstream) {
    (Head::Unborn(_), _) => say("No commits yet", t.dim),
    (Head::Detached(_), _) => say("Not on a branch", t.dim),
    (Head::Branch(_), None) if snapshot.has_remote => {
      say("Not published yet", t.warn)
    }
    (Head::Branch(_), None) => say("No remote to compare with", t.dim),
    (Head::Branch(_), Some(upstream)) => {
      let name = clean(&upstream.name);
      if upstream.gone {
        say(&format!("{name} is gone from the remote"), t.del);
      } else if upstream.ahead == 0 && upstream.behind == 0 {
        say(&format!("✓ Up to date with {name}"), t.add);
      } else {
        if upstream.ahead > 0 {
          say(&format!("↑{} to push", upstream.ahead), t.text);
        }
        if upstream.behind > 0 {
          say(&format!("↓{} to pull", upstream.behind), t.warn);
        }
      }
    }
  }

  if !snapshot.has_remote {
    return;
  }
  let mut line = area.take_top(1);
  let label = " ↻ Fetch ";
  let hit = line.take_left(label.width() as u16);
  let mut bg = t.chip;
  if hover.is_some_and(|(x, y)| hit.contains(x, y)) && !git.fetching {
    bg = bg.blend(t.text, HOVER);
  }
  let fg = if git.fetching { t.faint } else { t.text };
  grid.draw_text(hit, label, on(bg, fg));
  hits.fetch = Some(hit);
  if let Some(error) = &git.fetch_error {
    line.take_left(2);
    grid.draw_text(line, &fit(error, line.width), paint(t.del));
  }
}

/// One change: its letter, the file's name with its directory receding
/// beside it, and the lines added and removed, each in its column.
/// `selected` says whether the list has the focus, for the selected row.
fn render_file(
  row: Rect,
  grid: &mut Grid,
  file: &FileChange,
  bg: Rgb,
  selected: Option<bool>,
  (added_width, removed_width): (u16, u16),
  t: &GitTheme,
) {
  grid.fill_area(row, ' ', on(bg, t.text));
  let mut line = row;
  let mark = line.take_left(2);
  if let Some(focused) = selected {
    let color = if focused { t.accent } else { t.faint };
    grid.draw_text(mark, "▎", on(bg, color));
  }
  line.take_right(1);
  let removed = line.take_right(removed_width);
  if removed_width > 0 {
    line.take_right(1);
  }
  let added = line.take_right(added_width);
  let mut count = |mut at: Rect, sign: char, count: Option<u32>, fg: Rgb| {
    if let Some(n) = count.filter(|n| *n > 0) {
      let text = format!("{sign}{n}");
      let at = at.take_right(text.width() as u16);
      grid.draw_text(at, &text, on(bg, fg));
    }
  };
  count(removed, '-', file.removed, t.del);
  count(added, '+', file.added, t.add);
  line.take_right(1);

  let letter = file.kind.letter().to_string();
  let color = letter_color(file.kind, t);
  grid.draw_text(line.take_left(2), &letter, on(bg, color).set_bold(true));
  let path = clean(&file.path);
  let (dir, name) = split_path(&path);
  let name = fit(name, line.width);
  let name_color = if file.kind == Kind::Deleted {
    t.dim
  } else {
    t.text
  };
  grid.draw_text(
    line.take_left(name.width() as u16),
    &name,
    on(bg, name_color),
  );
  line.take_left(2);
  let dir = fit_dir(dir, line.width);
  grid.draw_text(line, &dir, on(bg, t.faint));
}

/// The selected change's diff under a bar that names the file.
fn render_diff(
  mut area: Rect,
  grid: &mut Grid,
  git: &GitView,
  hits: &mut GitHits,
  t: &GitTheme,
) {
  let focused = git.focus == GitFocus::Diff;
  let mut bar = title_bar(grid, area.take_top(1), "DIFF", focused, t);
  hits.diff = area;
  let file = git
    .selected
    .as_ref()
    .and_then(|key| git.files().iter().find(|file| file.key() == *key));
  let message = |grid: &mut Grid, text: &str, color: Rgb| {
    let within = area.inner((1, 3));
    for (row, line) in within.rows().zip(wrap(text, within.width)) {
      grid.draw_text(row, &line, on(t.bg, color));
    }
  };
  let Some(file) = file else {
    if git.loaded && git.snapshot.is_some() {
      message(grid, "Nothing to commit. The working tree is clean.", t.dim);
    }
    return;
  };

  // Which side of the file, then the file itself at the right end.
  let what = match file.area {
    Area::Staged => "staged",
    Area::Unstaged if file.kind == Kind::Untracked => "new file",
    Area::Unstaged => "not staged",
    Area::Conflict => "conflict",
  };
  bar.take_left(2);
  grid.draw_text(bar.take_left(what.width() as u16), what, on(t.bar, t.dim));
  bar.take_left(2);
  bar.take_right(1);
  let path = clean(&file.path);
  let (dir, name) = split_path(&path);
  let moved = file.orig.as_ref().map(|orig| {
    let orig = clean(orig);
    format!("{} → ", split_path(&orig).1)
  });
  let counts = [
    (file.removed.filter(|n| *n > 0), '-', t.del),
    (file.added.filter(|n| *n > 0), '+', t.add),
  ];
  for (count, sign, color) in counts {
    if let Some(n) = count {
      let text = format!(" {sign}{n}");
      if text.width() + 8 < usize::from(bar.width) {
        let at = bar.take_right(text.width() as u16);
        grid.draw_text(at, &text, on(t.bar, color));
      }
    }
  }
  let name = fit(name, bar.width);
  let name_area = bar.take_right(name.width() as u16);
  grid.draw_text(name_area, &name, on(t.bar, t.text));
  let before = match moved {
    Some(moved) if moved.width() <= usize::from(bar.width) => moved,
    Some(_) => String::new(),
    None if dir.is_empty() => String::new(),
    None => {
      let dir = fit_dir(dir, bar.width.saturating_sub(1));
      if dir.is_empty() {
        dir
      } else {
        format!("{dir}/")
      }
    }
  };
  let before_area = bar.take_right(before.width() as u16);
  grid.draw_text(before_area, &before, on(t.bar, t.faint));

  let Some(diff) = git.shown_diff() else {
    return;
  };
  if diff.lines.is_empty() {
    let note = diff.note.as_deref().unwrap_or("No lines changed");
    message(grid, note, t.dim);
    return;
  }
  render_lines(area, grid, diff, git.scroll, git.scroll_x, t);
}

fn render_lines(
  area: Rect,
  grid: &mut Grid,
  diff: &Diff,
  scroll: usize,
  scroll_x: usize,
  t: &GitTheme,
) {
  let height = usize::from(area.height);
  let digits = (diff.last_line().max(1).ilog10() as u16 + 1).max(3);
  let scrolls = diff.lines.len() > height;
  for (line, mut row) in diff.lines.iter().skip(scroll).zip(area.rows()) {
    let (bg, sign, sign_color, old, new, text) = match line {
      DiffLine::Hunk(header) => {
        grid.fill_area(row, ' ', on(t.surface, t.dim));
        grid.draw_text(row.inner((0, 2)), header, on(t.surface, t.dim));
        continue;
      }
      DiffLine::Note(note) => {
        let at = row.inner((0, 0, 0, digits * 2 + 6));
        grid.draw_text(at, note, on(t.bg, t.faint).set_italic(true));
        continue;
      }
      DiffLine::Context { old, new, text } => {
        (t.bg, ' ', t.dim, Some(*old), Some(*new), text)
      }
      DiffLine::Added { new, text } => {
        (t.add_bg, '+', t.add, None, Some(*new), text)
      }
      DiffLine::Removed { old, text } => {
        (t.del_bg, '-', t.del, Some(*old), None, text)
      }
    };
    grid.fill_area(row, ' ', on(bg, t.text));
    if scrolls {
      row.take_right(1);
    }
    row.take_left(1);
    for number in [old, new] {
      let at = row.take_left(digits);
      if let Some(n) = number {
        let n = format!("{n:>width$}", width = usize::from(digits));
        grid.draw_text(at, &n, on(bg, t.faint));
      }
      row.take_left(1);
    }
    row.take_left(1);
    let sign_area = row.take_left(2);
    if sign != ' ' {
      grid.put_char(
        Pos {
          col: sign_area.x,
          row: sign_area.y,
        },
        sign,
        on(bg, sign_color).set_bold(true),
      );
    }
    let shown: String;
    let text = if scroll_x == 0 {
      text.as_str()
    } else {
      shown = text.chars().skip(scroll_x).collect();
      shown.as_str()
    };
    grid.draw_text(row, text, on(bg, t.text));
  }

  if scrolls && area.width > 0 && height > 0 {
    // A thumb as tall as the part of the diff that is shown.
    let len = diff.lines.len();
    let thumb = (height * height / len).max(1);
    let most = len - height;
    let top = (height - thumb) * scroll.min(most) / most;
    let track = Rect {
      x: area.right() - 1,
      width: 1,
      ..area
    };
    for row in track.rows().skip(top).take(thumb) {
      grid.map_styles(row, |style| style.fg(t.faint.into()));
      let pos = Pos {
        col: row.x,
        row: row.y,
      };
      let style = grid.style_at(pos).unwrap_or_default();
      grid.put_char(pos, '▐', style);
    }
  }
}

/// The keys of the view on one line: each key in bold before what it
/// does.
fn render_hints(
  area: Rect,
  grid: &mut Grid,
  focus: GitFocus,
  keymap: &Keymap,
  t: &GitTheme,
) {
  if area.height == 0 {
    return;
  }
  grid.fill_area(area, ' ', on(t.raised, t.dim));
  let key = |action: &Action| {
    keymap
      .key(KeymapGroup::Git, action)
      .map(|key| key.to_string())
  };
  let mut hints: Vec<(String, &str)> = Vec::new();
  if let (Some(next), Some(prev)) =
    (key(&Action::GitNext), key(&Action::GitPrev))
  {
    let what = match focus {
      GitFocus::Files => "select",
      GitFocus::Diff => "scroll",
    };
    hints.push((format!("{next}/{prev}"), what));
  }
  for (action, what) in [
    (Action::ToggleFocus, "focus"),
    (Action::GitFetch, "fetch"),
    (Action::ShowTasks, "tasks"),
    (Action::ShowCommandsMenu, "commands"),
    (Action::ToggleKeymapWindow, "hide keys"),
    (Action::ShowQuit, "quit"),
  ] {
    if let Some(key) = key(&action) {
      hints.push((key, what));
    }
  }
  let mut line = area.inner((0, 2));
  for (key, what) in hints {
    if key.width() + what.width() + 1 > usize::from(line.width) {
      break;
    }
    let bold = on(t.raised, t.text).set_bold(true);
    grid.draw_text(line.take_left(key.width() as u16), &key, bold);
    line.take_left(1);
    let what_area = line.take_left(what.width() as u16);
    grid.draw_text(what_area, what, on(t.raised, t.dim));
    line.take_left(3);
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::*;
  use crate::config::keymap::KeymapConfig;
  use crate::console::state::View;
  use crate::git::{FileKey, Read, Repo, Upstream};
  use crate::term::Size;

  fn change(
    path: &str,
    kind: Kind,
    area: Area,
    added: u32,
    removed: u32,
  ) -> FileChange {
    FileChange {
      path: path.to_string(),
      orig: None,
      kind,
      area,
      added: Some(added),
      removed: Some(removed),
    }
  }

  fn snapshot(files: Vec<FileChange>) -> Snapshot {
    Snapshot {
      repo: Repo {
        root: PathBuf::from("/repo"),
        git_dir: PathBuf::from("/repo/.git"),
        common_dir: PathBuf::from("/repo/.git"),
      },
      head: Head::Branch("feat-#13193".to_string()),
      upstream: Some(Upstream {
        name: "origin/feat-#13193".to_string(),
        ahead: 2,
        behind: 3,
        gone: false,
      }),
      files,
      fetched: Some(SystemTime::UNIX_EPOCH),
      has_remote: true,
      doing: None,
    }
  }

  fn files() -> Vec<FileChange> {
    vec![
      change(
        "web/src/dashboard/Grid.tsx",
        Kind::Modified,
        Area::Staged,
        13,
        3,
      ),
      change("server/Service.java", Kind::Modified, Area::Unstaged, 5, 2),
      change("web/Legacy.tsx", Kind::Deleted, Area::Unstaged, 0, 14),
      change("docs/", Kind::Untracked, Area::Unstaged, 0, 0),
    ]
  }

  fn diff() -> Diff {
    crate::git::diff::parse(
      b"@@ -12,3 +12,3 @@ fn load() {\n   let a = 1;\n-  let b = 2;\n+  let b = 3;\n   done();\n",
    )
  }

  /// A console's state showing the git view with `read` taken.
  fn state(read: Read) -> State {
    let tasks =
      crate::console::task_tree::TaskTree::with_groups(Default::default());
    let mut state = State::new(tasks, false);
    state.view = View::Git;
    state.git.take(read);
    state
  }

  fn drawn(state: &mut State, width: u16, height: u16) -> (Grid, Vec<String>) {
    let mut grid = Grid::new(Size { width, height }, 0);
    let keymap = KeymapConfig::default().build();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(4 * 60);
    render_git(grid.area(), &mut grid, state, &keymap, &Theme::dark(), now);
    let rows = (0..height).map(|row| grid.row_text(row)).collect();
    (grid, rows)
  }

  #[test]
  fn draws_the_branch_the_changes_and_the_diff() {
    let key = FileKey {
      path: "web/src/dashboard/Grid.tsx".to_string(),
      area: Area::Staged,
    };
    let mut state = state(Read {
      snapshot: Some(snapshot(files())),
      diff: Some((key, diff())),
    });
    let (grid, rows) = drawn(&mut state, 100, 16);
    let side: Vec<&str> = rows
      .iter()
      .map(|row| row.split('│').next().unwrap().trim_end())
      .collect();
    assert_eq!(
      side,
      vec![
        "  BRANCH",
        "",
        "  feat-#13193       fetched 4m ago",
        "  ↑2 to push   ↓3 to pull",
        "   ↻ Fetch",
        "",
        "▎ CHANGES                        4",
        "",
        "  Staged 1",
        "▎ M Grid.tsx  …/dashboard  +13  -3",
        "",
        "  Unstaged 3",
        "  M Service.java  server    +5  -2",
        "  D Legacy.tsx  web            -14",
        "  ? docs/",
        "  j/k select   C-a focus   f fetch   1 tasks   p commands   ? hide keys   q quit",
      ]
    );
    let main: Vec<&str> = rows
      .iter()
      .take(6)
      .map(|row| row.split_once('│').unwrap().1)
      .collect();
    assert_eq!(
      main,
      vec![
        "  DIFF  staged                web/src/dashboard/Grid.tsx +13 -3",
        "  @@ -12,3 +12,3 @@ fn load() {",
        "  12  12      let a = 1;",
        "  13      -   let b = 2;",
        "      13  +   let b = 3;",
        "  14  14      done();",
      ]
    );
    // Added and removed lines are told apart by their backgrounds too.
    let theme = Theme::dark();
    let bg = |row| grid.style_at(Pos { col: 60, row }).unwrap().bg;
    assert_eq!(bg(3), theme.git.del_bg.into());
    assert_eq!(bg(4), theme.git.add_bg.into());
    assert_eq!(bg(2), theme.git.bg.into());
    // The rows of the changes, for the mouse.
    let at: Vec<usize> = state.git.hits.rows.iter().map(|r| r.1).collect();
    assert_eq!(at, vec![0, 1, 2, 3]);
    assert_eq!(state.git.hits.rows[0].0, Rect::new(0, 9, 35, 1));
    assert_eq!(state.git.hits.fetch, Some(Rect::new(2, 4, 9, 1)));
  }

  #[test]
  fn says_what_there_is_when_there_is_no_change_or_no_repository() {
    let mut clean = state(Read {
      snapshot: Some(Snapshot {
        upstream: Some(Upstream {
          name: "origin/main".to_string(),
          ahead: 0,
          behind: 0,
          gone: false,
        }),
        ..snapshot(Vec::new())
      }),
      diff: None,
    });
    let (_, rows) = drawn(&mut clean, 100, 12);
    let all = rows.join("\n");
    assert!(all.contains("✓ Up to date with origin/main"), "{all}");
    assert!(all.contains("✓ Nothing to commit"), "{all}");
    assert!(all.contains("The working tree is clean."), "{all}");

    let mut none = state(Read::default());
    let (_, rows) = drawn(&mut none, 100, 12);
    let all = rows.join("\n");
    assert!(all.contains("This project is not in a git"), "{all}");
    assert!(!all.contains("Fetch"), "{all}");
  }

  /// A narrow terminal shows the panel with the focus alone.
  #[test]
  fn a_narrow_terminal_shows_one_panel() {
    let key = FileKey {
      path: "web/src/dashboard/Grid.tsx".to_string(),
      area: Area::Staged,
    };
    let mut state = state(Read {
      snapshot: Some(snapshot(files())),
      diff: Some((key, diff())),
    });
    let (_, rows) = drawn(&mut state, 60, 12);
    assert!(rows[0].starts_with("  BRANCH"), "{rows:?}");
    assert!(!rows.join("\n").contains("DIFF"), "{rows:?}");
    state.git.focus = GitFocus::Diff;
    let (_, rows) = drawn(&mut state, 60, 12);
    assert!(rows[0].starts_with("▎ DIFF  staged"), "{rows:?}");
    assert!(!rows.join("\n").contains("BRANCH"), "{rows:?}");
  }

  /// The list keeps the selected change on screen, and the diff stops
  /// at its last line.
  #[test]
  fn scrolls_to_what_is_selected() {
    let many: Vec<FileChange> = (0..30)
      .map(|n| {
        change(&format!("f{n:02}.rs"), Kind::Modified, Area::Unstaged, 1, 0)
      })
      .collect();
    let mut state = state(Read {
      snapshot: Some(snapshot(many)),
      diff: None,
    });
    for _ in 0..29 {
      state.git.step(1);
    }
    state.git.scroll = 500;
    let (_, rows) = drawn(&mut state, 100, 14);
    assert!(rows[12].starts_with("▎ M f29.rs"), "{rows:?}");
    assert_eq!(state.git.scroll, 0);
    state.git.select(0);
    let (_, rows) = drawn(&mut state, 100, 14);
    assert!(rows[8].starts_with("  Unstaged 30"), "{rows:?}");
    assert!(rows[9].starts_with("▎ M f00.rs"), "{rows:?}");
    assert!(rows[12].starts_with("  M f03.rs"), "{rows:?}");
  }

  #[test]
  fn names_times_and_paths() {
    assert_eq!(ago(Duration::from_secs(20)), "just now");
    assert_eq!(ago(Duration::from_secs(250)), "4m ago");
    assert_eq!(ago(Duration::from_secs(3 * 3600)), "3h ago");
    assert_eq!(ago(Duration::from_secs(3 * 86_400)), "3d ago");
    assert_eq!(split_path("a/b/c.rs"), ("a/b", "c.rs"));
    assert_eq!(split_path("c.rs"), ("", "c.rs"));
    assert_eq!(split_path("notes/drafts/"), ("notes", "drafts/"));
    assert_eq!(fit("DashboardGrid.tsx", 10), "Dashboard…");
    assert_eq!(fit_dir("web/src/dashboard", 13), "…/dashboard");
    assert_eq!(fit_dir("web/src/dashboard", 5), "");
    let mut s = snapshot(Vec::new());
    assert_eq!(sync_mark(&s), "↑2 ↓3");
    s.upstream = None;
    assert_eq!(sync_mark(&s), "");
  }
}
