use unicode_width::UnicodeWidthStr;

use crate::console::git_view::GitDo;
use crate::console::keymap::Keymap;
use crate::console::theme::Theme;
use crate::console::widgets::list::{ListState, paint_row};
use crate::console::widgets::wrap::wrap;
use crate::term::{
  Grid,
  grid::Rect,
  key::{Key, KeyCode, KeyMods},
  mouse::MouseEvent,
};

use super::modal::{Modal, ModalResult};

/// The widest the dialog gets.
const MAX_WIDTH: u16 = 72;
/// The most rows of text it shows: of a longer one, the end.
const MAX_TEXT: usize = 16;
/// Space inside the border.
const PAD: u16 = 2;

/// What the git view asks before it does something that can't be taken
/// back, or that can be done in more than one way: what it is about,
/// then the ways to go on, the first safe one selected.
pub struct GitChoice {
  title: String,
  text: String,
  options: Vec<Choice>,
  list: ListState,
}

pub struct Choice {
  pub label: String,
  /// What it does, in git's words.
  pub note: String,
  /// It loses something: drawn as an error.
  pub danger: bool,
  /// What the console does once it is picked; nothing for the way out.
  pub then: Option<GitDo>,
}

impl Choice {
  pub fn new(label: &str, note: &str, then: GitDo) -> Self {
    Choice {
      label: label.to_string(),
      note: note.to_string(),
      danger: false,
      then: Some(then),
    }
  }

  pub fn danger(mut self) -> Self {
    self.danger = true;
    self
  }

  /// The way out, under a name.
  pub fn close(label: &str) -> Self {
    Choice {
      label: label.to_string(),
      note: String::new(),
      danger: false,
      then: None,
    }
  }
}

impl GitChoice {
  /// `selected` is the option the dialog opens on.
  pub fn new(
    title: &str,
    text: impl Into<String>,
    options: Vec<Choice>,
    selected: usize,
  ) -> Self {
    let mut list = ListState::new(0);
    list.reset(options.len());
    list.select(selected);
    GitChoice {
      title: title.to_string(),
      text: text.into(),
      options,
      list,
    }
  }

  /// Something to read, with only the way out: what git said when a
  /// commit failed.
  pub fn message(title: &str, text: impl Into<String>) -> Self {
    GitChoice::new(title, text, vec![Choice::close("Close")], 0)
  }

  fn width(&self, frame: Rect) -> u16 {
    let options = self
      .options
      .iter()
      .map(|o| o.label.width() + o.note.width() + 6)
      .max()
      .unwrap_or(0);
    let text = self.text.lines().map(|l| l.width()).max().unwrap_or(0);
    let width = options.max(text).max(self.title.width() + 2) as u16;
    (width + 2 + PAD * 2).clamp(40, MAX_WIDTH).min(frame.width)
  }

  /// The text as it fits the dialog: of a long one, the end.
  fn lines(&self, width: u16) -> Vec<String> {
    let lines = wrap(&self.text, width.saturating_sub(2 + PAD * 2));
    let skip = lines.len().saturating_sub(MAX_TEXT);
    lines.into_iter().skip(skip).collect()
  }

  fn pick(&self, index: usize) -> ModalResult {
    match self.options.get(index).and_then(|o| o.then.clone()) {
      Some(then) => ModalResult::Git(then),
      None => ModalResult::Close,
    }
  }
}

impl Modal for GitChoice {
  fn handle_key(&mut self, key: &Key) -> ModalResult {
    match (key.code, key.mods) {
      (KeyCode::Enter, KeyMods::NONE) => {
        return self
          .list
          .selected()
          .map_or(ModalResult::Close, |index| self.pick(index));
      }
      (KeyCode::Esc | KeyCode::Char('q'), KeyMods::NONE) => {
        return ModalResult::Close;
      }
      _ => {
        self.list.handle_vi_key(key);
      }
    }
    ModalResult::Keep
  }

  fn handle_mouse(&mut self, mouse: &MouseEvent) -> ModalResult {
    match self.list.handle_mouse(mouse) {
      Some(index) => self.pick(index),
      None => ModalResult::Keep,
    }
  }

  fn size(&self, frame: Rect) -> (u16, u16) {
    let width = self.width(frame);
    let rows = self.lines(width).len() + 1 + self.options.len() + 2;
    (width, (rows as u16).min(frame.height))
  }

  fn render(&mut self, grid: &mut Grid, _keymap: &Keymap, theme: &Theme) {
    let frame = grid.area();
    let area = self.area(frame);
    let style = theme.block.modal();
    let title = format!(" {} ", self.title);
    let mut inner = style.draw(grid, area).title(&title, style.title).inner();
    let lines = self.lines(area.width);
    let text = inner.take_top(lines.len() as u16).inner((0, PAD));
    for (at, (row, line)) in text.rows().zip(&lines).enumerate() {
      let attrs = if at == 0 { style.text } else { style.dim };
      grid.draw_text(row, line, attrs);
    }
    inner.take_top(1);
    for row in self.list.rows(inner) {
      let option = &self.options[row.index];
      let style = paint_row(grid, row.area, row.selected, style, &theme.block);
      let mut rest = row.area.inner((0, PAD));
      let note = rest.take_right(option.note.width() as u16);
      grid.draw_text(note, &option.note, style.dim);
      let label = if option.danger {
        style.error
      } else {
        style.text
      };
      grid.draw_text(rest, &option.label, label.set_bold(row.selected));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::console::git_view::PullHow;
  use crate::term::Size;

  fn key(code: KeyCode) -> Key {
    Key::new(code, KeyMods::NONE)
  }

  fn pull() -> GitChoice {
    GitChoice::new(
      "Pull",
      "Your branch and origin/main have each commits the other does not.",
      vec![
        Choice::new(
          "Rebase mine on top",
          "git pull --rebase",
          GitDo::Pull(PullHow::Rebase),
        ),
        Choice::new(
          "Merge",
          "git pull --no-rebase",
          GitDo::Pull(PullHow::Merge),
        ),
        Choice::close("Cancel"),
      ],
      0,
    )
  }

  #[test]
  fn enter_picks_the_selected_way_and_escape_none() {
    let mut modal = pull();
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Git(GitDo::Pull(PullHow::Rebase))
    ));
    modal.handle_key(&key(KeyCode::Char('j')));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Git(GitDo::Pull(PullHow::Merge))
    ));
    modal.handle_key(&key(KeyCode::Char('j')));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Close
    ));
    assert!(matches!(
      pull().handle_key(&key(KeyCode::Esc)),
      ModalResult::Close
    ));
  }

  #[test]
  fn shows_what_it_is_about_over_the_ways_to_go_on() {
    let mut grid = Grid::new(
      Size {
        width: 80,
        height: 12,
      },
      0,
    );
    let mut modal = pull();
    let keymap = Keymap::new();
    modal.render(&mut grid, &keymap, &Theme::dark());
    let rows: Vec<String> = (0..12).map(|row| grid.row_text(row)).collect();
    let all = rows.join("\n");
    assert!(all.contains("┌ Pull ─"), "{all}");
    assert!(all.contains("each commits the other does not."), "{all}");
    assert!(all.contains("Rebase mine on top"), "{all}");
    assert!(all.contains("git pull --no-rebase"), "{all}");
    assert!(all.contains("Cancel"), "{all}");
  }

  /// Of a long text, such as what a hook printed, the end is shown.
  #[test]
  fn a_long_text_shows_its_end() {
    let text: Vec<String> = (1..=40).map(|n| format!("line {n}")).collect();
    let modal = GitChoice::message("Commit failed", text.join("\n"));
    let lines = modal.lines(60);
    assert_eq!(lines.len(), MAX_TEXT);
    assert_eq!(lines.last().map(String::as_str), Some("line 40"));
  }
}
