//! Text of several lines, typed and edited in place: a commit message.
//! Lines longer than the field wrap between words; the cursor moves over
//! the rows as they are shown.

use std::ops::Range;

use crate::term::{
  Grid,
  grid::{Pos, Rect},
  key::{Key, KeyCode, KeyMods},
  style::Style,
};

#[derive(Default)]
pub struct TextArea {
  chars: Vec<char>,
  /// Before the character at this index; `chars.len()` at the end.
  cursor: usize,
}

impl TextArea {
  pub fn text(&self) -> String {
    self.chars.iter().collect()
  }

  pub fn is_blank(&self) -> bool {
    self.chars.iter().all(|c| c.is_whitespace())
  }

  /// Replaces the text; the cursor goes to its end.
  pub fn set(&mut self, text: &str) {
    self.chars.clear();
    self.cursor = 0;
    self.insert_str(text);
  }

  /// Types `text` at the cursor: its line breaks kept, and no other
  /// control character.
  pub fn insert_str(&mut self, text: &str) {
    for c in text.chars() {
      let c = match c {
        '\n' => '\n',
        '\t' => ' ',
        c if c.is_control() => continue,
        c => c,
      };
      self.chars.insert(self.cursor, c);
      self.cursor += 1;
    }
  }

  /// The rows the text is shown in at `width`, each a range of
  /// characters. A line too long for the width breaks after its last
  /// space that fits, or mid-word when it has none.
  pub fn rows(&self, width: u16) -> Vec<Range<usize>> {
    let width = usize::from(width.max(1));
    let mut rows = Vec::new();
    let mut start = 0;
    loop {
      let end = self.chars[start..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(self.chars.len(), |at| start + at);
      let mut at = start;
      while end - at > width {
        // A space right after a full row goes with it, not on to the
        // next row.
        let space = (at + 1..=at + width)
          .rev()
          .find(|i| self.chars[*i] == ' ')
          .map(|i| i + 1);
        let to = space.unwrap_or(at + width);
        rows.push(at..to);
        at = to;
      }
      rows.push(at..end);
      if end == self.chars.len() {
        return rows;
      }
      start = end + 1;
    }
  }

  /// The row the cursor is in, and how far into it.
  fn place(&self, rows: &[Range<usize>]) -> (usize, usize) {
    for (at, row) in rows.iter().enumerate() {
      // The end of a row that wrapped is the start of the next one.
      let wrapped = rows.get(at + 1).is_some_and(|next| next.start == row.end);
      if self.cursor < row.end || (self.cursor == row.end && !wrapped) {
        return (at, self.cursor.saturating_sub(row.start));
      }
    }
    (rows.len().saturating_sub(1), 0)
  }

  /// Moves the cursor `by` rows, keeping its column where the row is
  /// long enough.
  fn move_rows(&mut self, by: isize, width: u16) {
    let rows = self.rows(width);
    let (at, col) = self.place(&rows);
    let Some(to) = at.checked_add_signed(by).filter(|to| *to < rows.len())
    else {
      return;
    };
    let row = &rows[to];
    let wrapped = rows.get(to + 1).is_some_and(|next| next.start == row.end);
    let last = if wrapped {
      row.len().saturating_sub(1)
    } else {
      row.len()
    };
    self.cursor = row.start + col.min(last);
  }

  /// The start of the word before the cursor.
  fn word_start(&self) -> usize {
    let mut at = self.cursor;
    while at > 0 && self.chars[at - 1].is_whitespace() {
      at -= 1;
    }
    while at > 0 && !self.chars[at - 1].is_whitespace() {
      at -= 1;
    }
    at
  }

  /// Edits or moves by `key`, in a field `width` wide. False for a key
  /// that means nothing here.
  pub fn handle_key(&mut self, key: &Key, width: u16) -> bool {
    use KeyCode::*;
    let rows = self.rows(width);
    let (row, _) = self.place(&rows);
    match (key.code, key.mods) {
      (Char(c), KeyMods::NONE) | (Char(c), KeyMods::SHIFT) => {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
      }
      (Enter, KeyMods::NONE) => {
        self.chars.insert(self.cursor, '\n');
        self.cursor += 1;
      }
      (Backspace, KeyMods::NONE) | (Char('h'), KeyMods::CONTROL) => {
        if self.cursor > 0 {
          self.cursor -= 1;
          self.chars.remove(self.cursor);
        }
      }
      (Delete, KeyMods::NONE) => {
        if self.cursor < self.chars.len() {
          self.chars.remove(self.cursor);
        }
      }
      (Char('w'), KeyMods::CONTROL) | (Backspace, KeyMods::ALT) => {
        let start = self.word_start();
        self.chars.drain(start..self.cursor);
        self.cursor = start;
      }
      (Char('u'), KeyMods::CONTROL) => {
        let start = rows[row].start;
        self.chars.drain(start..self.cursor);
        self.cursor = start;
      }
      (Left, KeyMods::NONE) | (Char('b'), KeyMods::CONTROL) => {
        self.cursor = self.cursor.saturating_sub(1);
      }
      (Right, KeyMods::NONE) | (Char('f'), KeyMods::CONTROL) => {
        self.cursor = (self.cursor + 1).min(self.chars.len());
      }
      (Left, KeyMods::CONTROL) => self.cursor = self.word_start(),
      (Up, KeyMods::NONE) => self.move_rows(-1, width),
      (Down, KeyMods::NONE) => self.move_rows(1, width),
      (Home, KeyMods::NONE) | (Char('a'), KeyMods::CONTROL) => {
        self.cursor = rows[row].start;
      }
      (End, KeyMods::NONE) | (Char('e'), KeyMods::CONTROL) => {
        let end = rows[row].end;
        let wrapped = rows.get(row + 1).is_some_and(|next| next.start == end);
        self.cursor = if wrapped { end - 1 } else { end };
      }
      _ => return false,
    }
    true
  }

  /// Draws the rows that fit `area`, scrolled to keep the cursor's row
  /// in it, and returns where the cursor is.
  pub fn render(&self, area: Rect, grid: &mut Grid, style: Style) -> Pos {
    let rows = self.rows(area.width);
    let (at, col) = self.place(&rows);
    let height = usize::from(area.height.max(1));
    let top = (at + 1).saturating_sub(height);
    for (row, line) in rows.iter().skip(top).zip(area.rows()) {
      let text: String = self.chars[row.clone()].iter().collect();
      grid.draw_text(line, text.trim_end(), style);
    }
    Pos {
      col: area.x + (col as u16).min(area.width.saturating_sub(1)),
      row: area.y + (at - top) as u16,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::term::Size;

  fn key(code: KeyCode) -> Key {
    Key::new(code, KeyMods::NONE)
  }

  fn typed(text: &str) -> TextArea {
    let mut area = TextArea::default();
    area.set(text);
    area
  }

  fn shown(area: &TextArea, width: u16) -> Vec<String> {
    area
      .rows(width)
      .into_iter()
      .map(|row| area.chars[row].iter().collect())
      .collect()
  }

  #[test]
  fn wraps_between_words_and_keeps_line_breaks() {
    let area = typed("[DASHBOARD] Melhorias em componentes com IA\n\n#13193");
    assert_eq!(
      shown(&area, 20),
      vec![
        "[DASHBOARD] ",
        "Melhorias em ",
        "componentes com IA",
        "",
        "#13193"
      ]
    );
    // A word longer than the field is cut where the field ends.
    assert_eq!(shown(&typed("abcdefgh ij"), 4), vec!["abcd", "efgh ", "ij"]);
    assert_eq!(shown(&typed(""), 10), vec![""]);
    assert_eq!(shown(&typed("a\n"), 10), vec!["a", ""]);
  }

  #[test]
  fn types_and_deletes_at_the_cursor() {
    let mut area = typed("fix tooltip");
    for _ in 0..7 {
      area.handle_key(&key(KeyCode::Left), 40);
    }
    for c in "the ".chars() {
      area.handle_key(&key(KeyCode::Char(c)), 40);
    }
    assert_eq!(area.text(), "fix the tooltip");
    area.handle_key(&key(KeyCode::Backspace), 40);
    area.handle_key(&key(KeyCode::Delete), 40);
    assert_eq!(area.text(), "fix theooltip");
    area.handle_key(&Key::new(KeyCode::Char('w'), KeyMods::CONTROL), 40);
    assert_eq!(area.text(), "fix ooltip");
    area.handle_key(&key(KeyCode::Enter), 40);
    assert_eq!(area.text(), "fix \nooltip");
    assert!(!area.handle_key(&key(KeyCode::F(5)), 40));
    assert!(!area.is_blank());
    assert!(typed(" \n ").is_blank());
  }

  /// Up and down go by the rows as shown, wrapped ones too.
  #[test]
  fn moves_over_the_rows_shown() {
    let mut area = typed("one two three\nx");
    // "one two " / "three" / "x" at 8 wide; the cursor is after "x".
    assert_eq!(shown(&area, 8), vec!["one two ", "three", "x"]);
    area.handle_key(&key(KeyCode::Up), 8);
    assert_eq!(area.cursor, 9, "column 1 of three");
    area.handle_key(&key(KeyCode::End), 8);
    assert_eq!(area.cursor, 13);
    area.handle_key(&key(KeyCode::Up), 8);
    assert_eq!(area.cursor, 5, "the same column of the first row");
    area.handle_key(&key(KeyCode::End), 8);
    assert_eq!(area.cursor, 7, "before the space the row wraps at");
    area.handle_key(&key(KeyCode::Home), 8);
    assert_eq!(area.cursor, 0);
    area.handle_key(&key(KeyCode::Up), 8);
    assert_eq!(area.cursor, 0, "no row above");
    area.handle_key(&key(KeyCode::Down), 8);
    area.handle_key(&key(KeyCode::Down), 8);
    area.handle_key(&key(KeyCode::Down), 8);
    assert_eq!(area.cursor, 14, "the last row");
  }

  #[test]
  fn pasted_text_keeps_its_lines_and_drops_control_characters() {
    let mut area = typed("a");
    area.insert_str("b\r\nc\x1b[0m\td");
    assert_eq!(area.text(), "ab\nc[0m d");
  }

  /// A field shorter than the text shows the rows around the cursor.
  #[test]
  fn draws_the_rows_around_the_cursor() {
    let mut grid = Grid::new(
      Size {
        width: 12,
        height: 2,
      },
      0,
    );
    let mut area = typed("first\nsecond\nthird");
    let at = area.render(grid.area(), &mut grid, Style::default());
    assert_eq!(grid.row_text(0), "second");
    assert_eq!(grid.row_text(1), "third");
    assert_eq!(at, Pos { col: 5, row: 1 });
    area.handle_key(&key(KeyCode::Up), 12);
    area.handle_key(&key(KeyCode::Up), 12);
    let mut grid = Grid::new(
      Size {
        width: 12,
        height: 2,
      },
      0,
    );
    let at = area.render(grid.area(), &mut grid, Style::default());
    assert_eq!(grid.row_text(0), "first");
    assert_eq!(at, Pos { col: 5, row: 0 });
  }
}
