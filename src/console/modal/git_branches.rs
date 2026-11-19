use unicode_width::UnicodeWidthStr;

use crate::console::git_view::GitDo;
use crate::console::keymap::Keymap;
use crate::console::theme::Theme;
use crate::console::widgets::border::draw_hints;
use crate::console::widgets::filter_list::{FilterInput, FilterList};
use crate::git::{Branch, Branches, diff::clean};
use crate::term::{
  CursorStyle, Grid,
  grid::Rect,
  key::{Key, KeyCode, KeyMods},
  mouse::{MouseButton, MouseEvent, MouseEventKind},
};

use super::modal::{Modal, ModalResult};

const WIDTH: u16 = 54;
/// The most rows of branches shown at once.
const MAX_ROWS: u16 = 18;

/// The repository's branches under a filter, hanging from the branch's
/// name: Enter checks the selected one out, and Right shows what else
/// can be done with it.
pub struct GitBranches {
  anchor: Rect,
  filter: FilterList,
  entries: Vec<Entry>,
}

enum Entry {
  New,
  Fetch,
  Local(Branch),
  /// A remote's branch with no local one, as `origin/x`.
  Remote(String),
}

impl Entry {
  fn text(&self) -> &str {
    match self {
      Entry::New => "+ New branch…",
      Entry::Fetch => "↻ Fetch",
      Entry::Local(branch) => &branch.name,
      Entry::Remote(name) => name,
    }
  }
}

/// How a branch stands with its upstream, in a few cells.
fn stands(branch: &Branch) -> String {
  match &branch.upstream {
    None => "local".to_string(),
    Some(upstream) if upstream.gone => "gone".to_string(),
    Some(upstream) => match (upstream.ahead, upstream.behind) {
      (0, 0) => "✓".to_string(),
      (ahead, 0) => format!("↑{ahead}"),
      (0, behind) => format!("↓{behind}"),
      (ahead, behind) => format!("↑{ahead} ↓{behind}"),
    },
  }
}

impl GitBranches {
  pub fn new(anchor: Rect, branches: Branches) -> Self {
    let mut entries = vec![Entry::New, Entry::Fetch];
    entries.extend(branches.local.into_iter().map(Entry::Local));
    entries.extend(branches.remote.into_iter().map(Entry::Remote));
    let mut modal = GitBranches {
      anchor,
      filter: FilterList::default(),
      entries,
    };
    modal.refilter();
    modal
  }

  /// With something typed, only the branches it is part of.
  fn refilter(&mut self) {
    let search = self.filter.value().to_lowercase();
    let shown = (0..self.entries.len())
      .filter(|&i| match &self.entries[i] {
        Entry::New | Entry::Fetch => search.is_empty(),
        Entry::Local(branch) => branch.name.to_lowercase().contains(&search),
        Entry::Remote(name) => name.to_lowercase().contains(&search),
      })
      .collect();
    self.filter.show(shown);
    // Opens on the first branch that is not the one checked out: the
    // one most likely wanted.
    if search.is_empty() {
      let other = self.filter.shown().iter().position(|&i| {
        matches!(&self.entries[i], Entry::Local(branch) if !branch.current)
      });
      if let Some(at) = other {
        self.filter.list.select(at);
      }
    }
  }

  fn pick(&self, entry: Option<usize>) -> ModalResult {
    match entry.and_then(|i| self.entries.get(i)) {
      Some(Entry::New) => ModalResult::Git(GitDo::NewBranch { from: None }),
      Some(Entry::Fetch) => ModalResult::Git(GitDo::Fetch),
      Some(Entry::Local(branch)) if branch.current => ModalResult::Close,
      Some(Entry::Local(branch)) => ModalResult::Git(GitDo::Checkout {
        name: branch.name.clone(),
        remote: false,
      }),
      Some(Entry::Remote(name)) => ModalResult::Git(GitDo::Checkout {
        name: name.clone(),
        remote: true,
      }),
      None => ModalResult::Close,
    }
  }

  /// What else can be done with a branch.
  fn menu(&self, entry: Option<usize>) -> ModalResult {
    let (name, remote) = match entry.and_then(|i| self.entries.get(i)) {
      Some(Entry::Local(branch)) => (branch.name.clone(), false),
      Some(Entry::Remote(name)) => (name.clone(), true),
      Some(Entry::New | Entry::Fetch) | None => return ModalResult::Keep,
    };
    ModalResult::Git(GitDo::BranchMenu { name, remote })
  }

  fn took(&mut self, input: FilterInput) -> ModalResult {
    match input {
      FilterInput::Edited => self.refilter(),
      FilterInput::Picked(entry) => return self.pick(Some(entry)),
      FilterInput::Moved | FilterInput::Ignored => (),
    }
    ModalResult::Keep
  }
}

impl Modal for GitBranches {
  fn handle_key(&mut self, key: &Key) -> ModalResult {
    match (key.code, key.mods) {
      (KeyCode::Enter, KeyMods::NONE) => self.pick(self.filter.selected()),
      (KeyCode::Esc, KeyMods::NONE) => ModalResult::Close,
      (KeyCode::Right, KeyMods::NONE) => self.menu(self.filter.selected()),
      _ => {
        let input = self.filter.handle_key(key);
        self.took(input)
      }
    }
  }

  fn handle_mouse(&mut self, mouse: &MouseEvent) -> ModalResult {
    if mouse.kind == MouseEventKind::Down(MouseButton::Right) {
      let row = self.filter.list.row_at(mouse.x as u16, mouse.y as u16);
      let entry = row.and_then(|row| self.filter.shown().get(row.index));
      return self.menu(entry.copied());
    }
    let input = self.filter.handle_mouse(mouse);
    self.took(input)
  }

  fn handle_paste(&mut self, text: &str) -> ModalResult {
    let input = self.filter.handle_paste(text);
    self.took(input)
  }

  fn size(&self, frame: Rect) -> (u16, u16) {
    let rows = (self.filter.shown().len() as u16).clamp(1, MAX_ROWS);
    (WIDTH.min(frame.width), rows + 4)
  }

  fn unshaded(&self) -> Option<Rect> {
    Some(self.anchor)
  }

  /// Hangs under the branch's name, pulled back inside the frame.
  fn area(&self, frame: Rect) -> Rect {
    let (width, height) = self.size(frame);
    let x = self.anchor.x.min(frame.right().saturating_sub(width));
    let y = self.anchor.bottom().min(frame.bottom().saturating_sub(4));
    let height = height.min(frame.bottom().saturating_sub(y));
    Rect::new(x, y, width, height)
  }

  fn render(&mut self, grid: &mut Grid, _keymap: &Keymap, theme: &Theme) {
    let area = self.area(grid.area());
    let style = theme.block.modal();
    let mut block = style.draw(grid, area);
    block.title(" Branches ", style.title);
    let mut inner = block.inner();
    let input_row = inner.take_top(1);
    let sep_row = inner.take_top(1);
    block.separator(sep_row.y);
    draw_hints(
      grid,
      area,
      &[("<CR>", "check out"), ("<Right>", "more")],
      &style,
    );

    grid.cursor_pos =
      Some(self.filter.render_input(grid, input_row, "/ ", &style));
    grid.cursor_style = CursorStyle::BlinkingBar;

    if self.filter.shown().is_empty() {
      grid.draw_text(inner.inner((0, 2)), "No branch like that", style.dim);
      return;
    }
    for (entry, area, style) in
      self.filter.rows(grid, inner, style, &theme.block)
    {
      let entry = &self.entries[entry];
      let mut rest = area.inner((0, 1, 0, 2));
      match entry {
        Entry::New | Entry::Fetch => {
          grid.draw_text(rest, entry.text(), style.key);
        }
        Entry::Local(branch) => {
          let mark = stands(branch);
          let behind = branch.upstream.as_ref().is_some_and(|up| up.behind > 0);
          let attrs = match mark.as_str() {
            "gone" => style.error,
            _ if behind => style.key,
            _ => style.dim,
          };
          let at = rest.take_right(mark.width() as u16);
          grid.draw_text(at, &mark, attrs);
          rest.take_right(1);
          let dot = rest.take_left(2);
          if branch.current {
            grid.draw_text(dot, "●", style.key);
          }
          let name = clean(&branch.name);
          grid.draw_text(rest, &name, style.text.set_bold(branch.current));
        }
        Entry::Remote(name) => {
          let at = rest.take_right("remote".width() as u16);
          grid.draw_text(at, "remote", style.dim);
          rest.take_right(1);
          rest.take_left(2);
          grid.draw_text(rest, &clean(name), style.dim);
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::git::Upstream;
  use crate::term::Size;

  fn branch(name: &str, current: bool, track: Option<(u32, u32)>) -> Branch {
    Branch {
      name: name.to_string(),
      current,
      upstream: track.map(|(ahead, behind)| Upstream {
        name: format!("origin/{name}"),
        ahead,
        behind,
        gone: false,
      }),
      date: 0,
    }
  }

  fn modal() -> GitBranches {
    GitBranches::new(
      Rect::new(2, 3, 14, 1),
      Branches {
        local: vec![
          branch("feat-#13193", true, Some((2, 3))),
          branch("develop", false, Some((0, 12))),
          branch("feat-#13174", false, Some((0, 0))),
          branch("scratch", false, None),
        ],
        remote: vec!["origin/feat-#13201".to_string()],
      },
    )
  }

  fn key(code: KeyCode) -> Key {
    Key::new(code, KeyMods::NONE)
  }

  fn rows(modal: &mut GitBranches) -> Vec<String> {
    let mut grid = Grid::new(
      Size {
        width: 70,
        height: 20,
      },
      0,
    );
    modal.render(&mut grid, &Keymap::new(), &Theme::dark());
    (0..20).map(|row| grid.row_text(row)).collect()
  }

  /// It hangs under the name, and opens on the first branch that is not
  /// the one checked out.
  #[test]
  fn lists_the_branches_under_the_name() {
    let mut modal = modal();
    let rows = rows(&mut modal);
    assert!(rows[4].starts_with("  ┌ Branches ─"), "{rows:?}");
    assert!(rows[5].contains("/ "), "{rows:?}");
    let listed: Vec<&str> = rows[7..14]
      .iter()
      .map(|row| row.trim_matches(|c| c == '│' || c == ' ' || c == '▎'))
      .collect();
    assert_eq!(listed[0], "+ New branch…");
    assert_eq!(listed[1], "↻ Fetch");
    assert!(
      listed[2].starts_with("● feat-#13193") && listed[2].ends_with("↑2 ↓3")
    );
    assert!(listed[3].starts_with("develop") && listed[3].ends_with("↓12"));
    assert!(listed[4].ends_with("✓"), "{listed:?}");
    assert!(listed[5].starts_with("scratch") && listed[5].ends_with("local"));
    assert!(listed[6].starts_with("origin/feat-#13201"), "{listed:?}");
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Git(GitDo::Checkout { name, remote: false }) if name == "develop"
    ));
  }

  #[test]
  fn typing_narrows_and_right_shows_what_else_can_be_done() {
    let mut modal = modal();
    for c in "1320".chars() {
      modal.handle_key(&key(KeyCode::Char(c)));
    }
    assert_eq!(modal.filter.shown().len(), 1);
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Right)),
      ModalResult::Git(GitDo::BranchMenu { name, remote: true })
        if name == "origin/feat-#13201"
    ));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Git(GitDo::Checkout { name, remote: true })
        if name == "origin/feat-#13201"
    ));
    for _ in 0..4 {
      modal.handle_key(&key(KeyCode::Backspace));
    }
    assert_eq!(modal.filter.shown().len(), 7);
    // The checked-out branch has nothing to check out.
    modal.handle_key(&key(KeyCode::Up));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Close
    ));
    modal.handle_key(&key(KeyCode::Up));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Enter)),
      ModalResult::Git(GitDo::Fetch)
    ));
    assert!(matches!(
      modal.handle_key(&key(KeyCode::Right)),
      ModalResult::Keep
    ));
  }
}
