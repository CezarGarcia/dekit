use tui_input::Input;
use unicode_width::UnicodeWidthStr;

use crate::console::git_view::GitDo;
use crate::console::keymap::Keymap;
use crate::console::theme::Theme;
use crate::console::widgets::border::draw_hints;
use crate::console::widgets::text_input::{
  paste_line, render_text_field, to_input_request,
};
use crate::git::diff::clean;
use crate::term::{
  CursorStyle, Grid,
  grid::Rect,
  key::{Key, KeyCode, KeyMods},
  mouse::{MouseButton, MouseEvent, MouseEventKind},
};

use super::modal::{Modal, ModalResult};

const WIDTH: u16 = 58;
/// Space inside the border.
const PAD: u16 = 2;
const LABEL: u16 = 6;

/// Asks for a new branch: its name, where it starts, and whether to
/// check it out.
pub struct GitNewBranch {
  name: Input,
  /// Where it can start: branches, local and of the remotes.
  bases: Vec<String>,
  base: usize,
  switch: bool,
  /// The local branches there are: a new one can't have their names.
  taken: Vec<String>,
  error: Option<String>,
  /// Where the base and the checkbox were last drawn.
  base_row: Rect,
  switch_row: Rect,
}

impl GitNewBranch {
  /// `bases` starts with where it starts unless another is picked.
  pub fn new(bases: Vec<String>, taken: Vec<String>) -> Self {
    GitNewBranch {
      name: Input::default(),
      bases,
      base: 0,
      switch: true,
      taken,
      error: None,
      base_row: Rect::default(),
      switch_row: Rect::default(),
    }
  }

  fn step_base(&mut self, by: isize) {
    let len = self.bases.len() as isize;
    if len > 0 {
      self.base = (self.base as isize + by).rem_euclid(len) as usize;
    }
  }

  fn create(&mut self) -> ModalResult {
    let name = self.name.value().trim().to_string();
    if name.is_empty() {
      self.error = Some("Type a name for the branch.".to_string());
      return ModalResult::Keep;
    }
    if self.taken.contains(&name) {
      self.error = Some(format!("There is a branch named {name} already."));
      return ModalResult::Keep;
    }
    let Some(from) = self.bases.get(self.base).cloned() else {
      return ModalResult::Close;
    };
    ModalResult::Git(GitDo::CreateBranch {
      name,
      from,
      switch: self.switch,
    })
  }
}

impl Modal for GitNewBranch {
  fn handle_key(&mut self, key: &Key) -> ModalResult {
    match (key.code, key.mods) {
      (KeyCode::Enter, KeyMods::NONE) => return self.create(),
      (KeyCode::Esc, KeyMods::NONE) => return ModalResult::Close,
      (KeyCode::Up, KeyMods::NONE) => self.step_base(-1),
      (KeyCode::Down, KeyMods::NONE) => self.step_base(1),
      (KeyCode::Tab, KeyMods::NONE) => self.switch = !self.switch,
      // A branch's name has no spaces: what is typed as one is a dash.
      (KeyCode::Char(' '), KeyMods::NONE) => {
        self.name.handle(tui_input::InputRequest::InsertChar('-'));
        self.error = None;
      }
      _ => {
        if let Some(request) = to_input_request(key) {
          self.name.handle(request);
          self.error = None;
        }
      }
    }
    ModalResult::Keep
  }

  fn handle_mouse(&mut self, mouse: &MouseEvent) -> ModalResult {
    let (x, y) = (mouse.x as u16, mouse.y as u16);
    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
      if self.base_row.contains(x, y) {
        self.step_base(1);
      } else if self.switch_row.contains(x, y) {
        self.switch = !self.switch;
      }
    }
    ModalResult::Keep
  }

  fn handle_paste(&mut self, text: &str) -> ModalResult {
    let text = text.trim().replace(char::is_whitespace, "-");
    paste_line(&mut self.name, &text);
    self.error = None;
    ModalResult::Keep
  }

  fn size(&self, frame: Rect) -> (u16, u16) {
    (WIDTH.min(frame.width), 9)
  }

  fn render(&mut self, grid: &mut Grid, _keymap: &Keymap, theme: &Theme) {
    let area = self.area(grid.area());
    let style = theme.block.modal();
    let inner = style
      .draw(grid, area)
      .title(" New branch ", style.title)
      .inner();
    draw_hints(
      grid,
      area,
      &[
        ("<CR>", "create"),
        ("<Up/Down>", "from"),
        ("<Tab>", "check out"),
      ],
      &style,
    );
    let mut rows = inner.inner((1, PAD)).rows();
    let (Some(mut name), Some(mut from), Some(switch), Some(_), Some(said)) = (
      rows.next(),
      rows.next(),
      rows.next(),
      rows.next(),
      rows.next(),
    ) else {
      return;
    };

    grid.draw_text(name.take_left(LABEL), "Name", style.dim);
    let line = theme.block.border_focused;
    let cursor = render_text_field(
      &self.name,
      name,
      grid,
      "feat-#123",
      None,
      &style,
      line,
    );
    grid.cursor_pos = Some(cursor);
    grid.cursor_style = CursorStyle::BlinkingBar;

    self.base_row = from;
    grid.draw_text(from.take_left(LABEL), "From", style.dim);
    let base = self.bases.get(self.base).map_or("", String::as_str);
    let base = clean(base);
    let count = format!("{}/{}", self.base + 1, self.bases.len().max(1));
    let at = from.take_right(count.width() as u16);
    grid.draw_text(at, &count, style.dim);
    grid.draw_text(from, &base, style.text);

    self.switch_row = switch;
    let mut check = switch;
    check.take_left(LABEL);
    let mark = if self.switch { "[✓]" } else { "[ ]" };
    grid.draw_text(check.take_left(4), mark, style.key);
    grid.draw_text(check, "Check it out", style.text);

    if let Some(error) = &self.error {
      grid.draw_text(said, error, style.error);
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

  fn modal() -> GitNewBranch {
    GitNewBranch::new(
      vec![
        "feat-#13193".to_string(),
        "develop".to_string(),
        "origin/develop".to_string(),
      ],
      vec!["feat-#13193".to_string(), "develop".to_string()],
    )
  }

  fn typed(modal: &mut GitNewBranch, text: &str) {
    for c in text.chars() {
      modal.handle_key(&key(KeyCode::Char(c)));
    }
  }

  #[test]
  fn makes_the_branch_named_from_the_base_picked() {
    let mut modal = modal();
    // Nothing typed yet.
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Keep
    ));
    assert!(modal.error.is_some());
    typed(&mut modal, "feat #13201");
    assert_eq!(modal.error, None);
    modal.handle_key(&key(KeyCode::Down));
    modal.handle_key(&key(KeyCode::Down));
    modal.handle_key(&key(KeyCode::Tab));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Git(GitDo::CreateBranch { name, from, switch: false })
        if name == "feat-#13201" && from == "origin/develop"
    ));
    // The bases go around.
    modal.handle_key(&key(KeyCode::Down));
    assert_eq!(modal.base, 0);
    modal.handle_key(&key(KeyCode::Up));
    assert_eq!(modal.base, 2);
  }

  #[test]
  fn a_name_that_is_taken_is_said_so() {
    let mut modal = modal();
    typed(&mut modal, "develop");
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Keep
    ));
    let mut grid = Grid::new(
      Size {
        width: 70,
        height: 12,
      },
      0,
    );
    modal.render(&mut grid, &Keymap::new(), &Theme::dark());
    let all: Vec<String> = (0..12).map(|row| grid.row_text(row)).collect();
    let all = all.join("\n");
    assert!(all.contains("┌ New branch ─"), "{all}");
    assert!(all.contains("Name  develop"), "{all}");
    assert!(all.contains("From  feat-#13193"), "{all}");
    assert!(all.contains("[✓] Check it out"), "{all}");
    assert!(
      all.contains("There is a branch named develop already."),
      "{all}"
    );
  }
}
