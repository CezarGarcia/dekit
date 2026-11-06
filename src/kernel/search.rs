//! Finding text in a frozen screen: its history and its live rows, as
//! copy mode shows them.
//!
//! Text is matched per logical line, so a match can run across the rows
//! a long line wraps onto. A query with no capital letters ignores case
//! (smart case, as `rg -S` and vim's `smartcase` have it).

use std::ops::Range;

use crate::kernel::copy_mode::Pos;
use crate::term::{
  Grid,
  cell::GraphemeTable,
  history::{Line, wrap},
  row::Row,
  style::StyleTable,
};

/// One match: the cells from `start` to `end`, both included, in reading
/// order, in copy-mode coordinates (row 0 is the live screen's top row,
/// history rows are above it).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Match {
  pub start: Pos,
  pub end: Pos,
}

/// Every match of `query` in `grid`, top to bottom. Empty for an empty
/// query.
pub fn find(grid: &Grid, query: &str) -> Vec<Match> {
  let Some(needle) = Needle::new(query) else {
    return Vec::new();
  };
  let width = grid.size().width;
  let history = grid.history();
  let tables = grid.tables();
  let mut out = Vec::new();
  let mut line = LineBuf::default();

  // History rows sit above the live ones: the oldest at the top.
  let mut y = -(grid.max_scrollback() as i32);
  let last = history.len().checked_sub(1);
  for i in 0..history.len() {
    let Some(entry) = history.get(i) else {
      continue;
    };
    // The open line goes on in the screen's first row: it is matched
    // with the rows that continue it.
    let open = history.is_open() && Some(i) == last;
    if open {
      line.push_line(&entry, y, width);
    } else if needle.is_in(&entry.text) {
      line.push_line(&entry, y, width);
      line.flush(&needle, &mut out);
    }
    y += entry.rows_at(width) as i32;
  }

  for (r, row) in grid.rows().iter().enumerate() {
    line.push_row(row, r as i32, &tables.styles, &tables.graphemes);
    if !row.wrapped() {
      line.flush(&needle, &mut out);
    }
  }
  line.flush(&needle, &mut out);
  out
}

/// What is looked for, and how.
struct Needle {
  query: String,
  /// Case is ignored: the query has no capital letters.
  fold: bool,
}

impl Needle {
  fn new(query: &str) -> Option<Self> {
    if query.is_empty() {
      return None;
    }
    Some(Needle {
      query: query.to_string(),
      fold: !query.chars().any(char::is_uppercase),
    })
  }

  fn is_in(&self, hay: &str) -> bool {
    if self.fold {
      self.next_in(hay, 0).is_some()
    } else {
      hay.contains(&self.query)
    }
  }

  /// The byte ranges of `hay` it matches, left to right, not
  /// overlapping.
  fn find_all(&self, hay: &str) -> Vec<Range<usize>> {
    if !self.fold {
      return hay
        .match_indices(&self.query)
        .map(|(at, found)| at..at + found.len())
        .collect();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(range) = self.next_in(hay, from) {
      from = range.end;
      out.push(range);
    }
    out
  }

  /// The first match at or after byte `from` (a char boundary), case
  /// ignored.
  fn next_in(&self, hay: &str, from: usize) -> Option<Range<usize>> {
    let rest = hay.get(from..)?;
    'starts: for (at, _) in rest.char_indices() {
      let start = from + at;
      let mut end = start;
      let mut hay_chars = hay[start..].chars();
      for want in self.query.chars() {
        match hay_chars.next() {
          Some(got) if same_folded(got, want) => end += got.len_utf8(),
          _ => continue 'starts,
        }
      }
      return Some(start..end);
    }
    None
  }
}

fn same_folded(a: char, b: char) -> bool {
  a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// A logical line being gathered: its text, and where each cell of it
/// shows.
#[derive(Default)]
struct LineBuf {
  text: String,
  cells: Vec<CellAt>,
}

/// A cell of a logical line: where its text starts in the line's text,
/// and the row, column and width it shows at.
struct CellAt {
  byte: usize,
  y: i32,
  x: i32,
  width: i32,
}

impl LineBuf {
  /// A history line whose first row is at `y`, wrapped at `width`.
  fn push_line(&mut self, line: &Line, y: i32, width: u16) {
    let widths = line.cells().map(|cell| cell.width());
    for (cell, (row, col)) in line.cells().zip(wrap(widths, width)) {
      // The right half of a wide cell has no text of its own.
      if cell.width() == 0 {
        continue;
      }
      self.cells.push(CellAt {
        byte: self.text.len(),
        y: y + row as i32,
        x: i32::from(col),
        width: i32::from(cell.width()),
      });
      self.text.push_str(cell.text);
    }
  }

  /// Live row `y`: all of it when it wraps onto the next, since its
  /// blanks are part of the line, else up to its last visible cell.
  fn push_row(
    &mut self,
    row: &Row,
    y: i32,
    styles: &StyleTable,
    graphemes: &GraphemeTable,
  ) {
    let len = if row.wrapped() {
      row.cols()
    } else {
      row.used_len(|id| styles.get(id).is_visible_blank())
    };
    let mut buf = [0u8; 4];
    for col in 0..len {
      let Some(cell) = row.get(col) else {
        break;
      };
      if cell.is_continuation() || cell.text.is_filler() {
        continue;
      }
      let text = if cell.text.is_image() {
        " "
      } else {
        cell.text.as_str(&mut buf, graphemes)
      };
      self.cells.push(CellAt {
        byte: self.text.len(),
        y,
        x: i32::from(col),
        width: i32::from(cell.width().max(1)),
      });
      self.text.push_str(if text.is_empty() { " " } else { text });
    }
  }

  /// Matches the gathered line and starts the next one.
  fn flush(&mut self, needle: &Needle, out: &mut Vec<Match>) {
    if !self.text.is_empty() {
      for range in needle.find_all(&self.text) {
        // The cells holding the first and the last byte of the match.
        let first = self.cells.partition_point(|c| c.byte <= range.start);
        let last = self.cells.partition_point(|c| c.byte < range.end);
        let (Some(first), Some(last)) =
          (first.checked_sub(1), last.checked_sub(1))
        else {
          continue;
        };
        let (first, last) = (&self.cells[first], &self.cells[last]);
        out.push(Match {
          start: Pos {
            y: first.y,
            x: first.x,
          },
          end: Pos {
            y: last.y,
            x: last.x + last.width - 1,
          },
        });
      }
    }
    self.text.clear();
    self.cells.clear();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::term::{Screen, Size};

  fn screen(width: u16, height: u16, scrollback: usize) -> Screen {
    Screen::new(Size { width, height }, scrollback)
  }

  fn at(y: i32, x: i32) -> Pos {
    Pos { y, x }
  }

  fn found(screen: &Screen, query: &str) -> Vec<(Pos, Pos)> {
    find(screen.grid(), query)
      .into_iter()
      .map(|m| (m.start, m.end))
      .collect()
  }

  #[test]
  fn finds_on_the_screen_and_in_history() {
    let mut s = screen(20, 3, 100);
    s.process(b"error one\r\nok\r\nerror two\r\nok\r\nlast");
    // `error one` and `ok` scrolled into history, two rows above row 0.
    assert_eq!(
      found(&s, "error"),
      vec![(at(-2, 0), at(-2, 4)), (at(0, 0), at(0, 4))]
    );
    assert_eq!(found(&s, "two"), vec![(at(0, 6), at(0, 8))]);
    assert!(found(&s, "missing").is_empty());
    assert!(found(&s, "").is_empty());
  }

  #[test]
  fn smart_case() {
    let mut s = screen(30, 3, 0);
    s.process(b"Error error ERROR");
    assert_eq!(found(&s, "error").len(), 3);
    assert_eq!(found(&s, "Error"), vec![(at(0, 0), at(0, 4))]);
    assert_eq!(found(&s, "ERROR"), vec![(at(0, 12), at(0, 16))]);
  }

  #[test]
  fn matches_do_not_overlap() {
    let mut s = screen(20, 2, 0);
    s.process(b"aaaa");
    assert_eq!(
      found(&s, "aa"),
      vec![(at(0, 0), at(0, 1)), (at(0, 2), at(0, 3))]
    );
  }

  #[test]
  fn a_match_runs_across_a_wrapped_line() {
    let mut s = screen(10, 4, 100);
    // Wraps at 10: `0123456789` then `abcdef`.
    s.process(b"0123456789abcdef");
    assert_eq!(found(&s, "89ab"), vec![(at(0, 8), at(1, 1))]);
    // Pushed into history, the line is still found whole.
    s.process(b"\r\n1\r\n2\r\n3\r\n4");
    assert_eq!(found(&s, "89ab"), vec![(at(-2, 8), at(-1, 1))]);
  }

  #[test]
  fn wide_characters_take_their_two_columns() {
    let mut s = screen(20, 2, 100);
    s.process("ab日本cd".as_bytes());
    assert_eq!(found(&s, "日本"), vec![(at(0, 2), at(0, 5))]);
    assert_eq!(found(&s, "cd"), vec![(at(0, 6), at(0, 7))]);
    s.process(b"\r\n\r\n");
    assert_eq!(found(&s, "本c"), vec![(at(-1, 4), at(-1, 6))]);
  }

  #[test]
  fn trailing_blanks_are_not_text() {
    let mut s = screen(10, 2, 0);
    s.process(b"a");
    assert!(found(&s, "a ").is_empty());
  }

  #[test]
  fn non_ascii_case_folding() {
    let mut s = screen(20, 2, 0);
    s.process("ÉCHEC échec".as_bytes());
    assert_eq!(found(&s, "échec").len(), 2);
    assert_eq!(found(&s, "ÉCHEC"), vec![(at(0, 0), at(0, 4))]);
  }
}
