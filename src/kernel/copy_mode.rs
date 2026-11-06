use crate::kernel::search::{self, Match};
use crate::term::{Grid, Size};

/// A frozen copy of a screen's grid (cheap: rows and history are shared
/// until written) with a selection over it, and what is searched in it.
pub struct CopyState {
  snapshot: Grid,
  start: Pos,
  end: Option<Pos>,
  search: Option<Search>,
}

/// A search over the frozen grid.
struct Search {
  query: String,
  /// Top to bottom.
  matches: Vec<Match>,
  /// The size the matches were found at: a resize reflows the grid
  /// under them.
  size: Size,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pos {
  pub y: i32,
  pub x: i32,
}

#[derive(Clone, Copy, Debug)]
pub enum CopyMove {
  Up,
  Down,
  Left,
  Right,
}

/// Which way the next match is looked for: up is older output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchDir {
  Up,
  Down,
}

/// Where a search stands: the match the cursor is on, counted from the
/// bottom (1 is the newest), and how many there are.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchStatus {
  pub current: Option<usize>,
  pub total: usize,
}

impl CopyState {
  /// Freezes `grid` and starts copy mode with the cursor on the bottom row.
  pub fn new(grid: Grid) -> Self {
    let y = (grid.size().height as i32 - 1).max(0);
    CopyState {
      snapshot: grid,
      start: Pos { y, x: 0 },
      end: None,
      search: None,
    }
  }

  pub fn snapshot(&self) -> &Grid {
    &self.snapshot
  }

  /// Follows the observers' size. What a search found is looked for
  /// again, since the grid reflowed under it.
  pub fn set_size(&mut self, size: Size) {
    self.snapshot.set_size(size);
    if let Some(search) = &mut self.search
      && search.size != size
    {
      search.matches = search::find(&self.snapshot, &search.query);
      search.size = size;
    }
  }

  pub fn start(&self) -> Pos {
    self.start
  }

  pub fn end(&self) -> Option<Pos> {
    self.end
  }

  /// The position the cursor is currently driving: the selection cursor once a
  /// selection is started, otherwise the lone anchor.
  pub fn cursor(&self) -> Pos {
    self.end.unwrap_or(self.start)
  }

  fn cursor_mut(&mut self) -> &mut Pos {
    self.end.as_mut().unwrap_or(&mut self.start)
  }

  pub fn move_cursor(&mut self, dir: CopyMove) {
    let width = self.snapshot.size().width as i32;
    let height = self.snapshot.size().height as i32;
    let history_rows = self.snapshot.max_scrollback() as i32;
    let pos = self.cursor_mut();
    match dir {
      CopyMove::Up => {
        if pos.y > -history_rows {
          pos.y -= 1;
        }
      }
      CopyMove::Down => {
        if pos.y + 1 < height {
          pos.y += 1;
        }
      }
      CopyMove::Left => {
        if pos.x > 0 {
          pos.x -= 1;
        }
      }
      CopyMove::Right => {
        if pos.x + 1 < width {
          pos.x += 1;
        }
      }
    }
  }

  /// Anchors the selection at the current cursor and begins extending it.
  pub fn begin_selection(&mut self) {
    if self.end.is_none() {
      self.end = Some(self.start);
    }
  }

  /// Converts a present-screen cell (row, col) to a copy-mode position,
  /// accounting for the current scroll offset.
  pub fn pos_at(&self, row: u16, col: u16) -> Pos {
    Pos {
      y: row as i32 - self.snapshot.scrollback() as i32,
      x: col as i32,
    }
  }

  /// Places the selection anchor at `pos`, clearing any active selection.
  pub fn set_anchor(&mut self, pos: Pos) {
    self.start = pos;
    self.end = None;
  }

  /// Extends the selection to `pos`.
  pub fn set_extent(&mut self, pos: Pos) {
    self.end = Some(pos);
  }

  pub fn scroll_up(&mut self, n: usize) {
    let offset = self.snapshot.scrollback().saturating_add(n);
    self.snapshot.set_scrollback(offset);
  }

  pub fn scroll_down(&mut self, n: usize) {
    let offset = self.snapshot.scrollback().saturating_sub(n);
    self.snapshot.set_scrollback(offset);
  }

  /// Current scroll offset (rows above the live bottom).
  pub fn scrollback(&self) -> usize {
    self.snapshot.scrollback()
  }

  /// How far back the view can go.
  pub fn max_scrollback(&self) -> usize {
    self.snapshot.max_scrollback()
  }

  /// Looks for `query` and moves the cursor to the nearest match at or
  /// above it, or else to the bottom one: as the query is typed, the
  /// cursor stays on a match while it still matches. A fresh search
  /// counts the cursor's whole row as above it. An empty query ends the
  /// search.
  pub fn search(&mut self, query: &str) {
    if query.is_empty() {
      self.search = None;
      return;
    }
    let cursor = self.cursor();
    let origin = match self.search {
      Some(_) => cursor,
      None => Pos {
        y: cursor.y,
        x: i32::MAX,
      },
    };
    let matches = search::find(&self.snapshot, query);
    let above = matches.partition_point(|m| !origin.is_before(m.start));
    let target = above
      .checked_sub(1)
      .map_or(matches.last(), |i| matches.get(i));
    let target = target.copied();
    self.search = Some(Search {
      query: query.to_string(),
      matches,
      size: self.snapshot.size(),
    });
    if let Some(found) = target {
      self.jump(found);
    }
  }

  /// Moves the cursor to the next match up or down from it, wrapping
  /// around at the top and the bottom.
  pub fn search_move(&mut self, dir: SearchDir) {
    let Some(search) = &self.search else {
      return;
    };
    let cursor = self.cursor();
    let matches = &search.matches;
    let target = match dir {
      SearchDir::Up => {
        let above = matches.partition_point(|m| m.start.is_before(cursor));
        above
          .checked_sub(1)
          .map_or(matches.last(), |i| matches.get(i))
      }
      SearchDir::Down => {
        let below = matches.partition_point(|m| !cursor.is_before(m.start));
        matches.get(below).or(matches.first())
      }
    };
    if let Some(found) = target.copied() {
      self.jump(found);
    }
  }

  /// What the search found, top to bottom; nothing without one.
  pub fn matches(&self) -> &[Match] {
    self.search.as_ref().map_or(&[], |search| &search.matches)
  }

  /// The match the cursor is on, if any.
  pub fn current_match(&self) -> Option<Match> {
    self.current_index().map(|i| self.matches()[i])
  }

  fn current_index(&self) -> Option<usize> {
    let cursor = self.cursor();
    let matches = self.matches();
    let i = matches.partition_point(|m| m.start.is_before(cursor));
    matches
      .get(i)
      .is_some_and(|m| m.start == cursor)
      .then_some(i)
  }

  /// `None` while nothing is searched for.
  pub fn search_status(&self) -> Option<SearchStatus> {
    let total = self.search.as_ref()?.matches.len();
    let current = self.current_index().map(|i| total - i);
    Some(SearchStatus { current, total })
  }

  /// Puts the cursor on a match and scrolls it into view.
  fn jump(&mut self, found: Match) {
    *self.cursor_mut() = found.start;
    self.reveal(found.start.y, found.end.y);
  }

  /// Scrolls rows `top..=bottom` into view, centered, unless they are in
  /// view already. The top row does not count: the badge is over it.
  fn reveal(&mut self, top: i32, bottom: i32) {
    let height = i32::from(self.snapshot.size().height);
    let view_top = -(self.snapshot.scrollback() as i32);
    let first = if height > 2 { view_top + 1 } else { view_top };
    if top >= first && bottom < view_top + height {
      return;
    }
    let rows = bottom - top + 1;
    let shown_top = top - (height - rows).max(0) / 2;
    self.snapshot.set_scrollback((-shown_top).max(0) as usize);
  }

  /// Extracts the selected text, or `None` if no selection has been started.
  pub fn selected_text(&self) -> Option<String> {
    let end = self.end?;
    let (low, high) = Pos::to_low_high(self.start, end);
    // Selection coordinates are relative to the live screen's top row;
    // the snapshot reads in view coordinates.
    let offset = self.snapshot.scrollback() as i32;
    Some(self.snapshot.get_selected_text(
      low.x,
      low.y + offset,
      high.x,
      high.y + offset,
    ))
  }
}

impl Pos {
  /// Whether `self` comes before `other` in reading order.
  pub fn is_before(self, other: Pos) -> bool {
    (self.y, self.x) < (other.y, other.x)
  }

  pub fn to_low_high(a: Pos, b: Pos) -> (Pos, Pos) {
    if a.y < b.y || (a.y == b.y && a.x < b.x) {
      (a, b)
    } else {
      (b, a)
    }
  }
}
