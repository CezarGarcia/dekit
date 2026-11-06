use unicode_width::UnicodeWidthStr;

use crate::console::action::{Action, RunnerTab};
use crate::console::{
  keymap::{Keymap, KeymapGroup},
  state::State,
  theme::Theme,
};
use crate::term::{Grid, grid::Rect};

pub fn render_keymap(
  area: Rect,
  grid: &mut Grid,
  state: &State,
  keymap: &Keymap,
  theme: &Theme,
) {
  if area.width <= 3 || area.height < 3 {
    return;
  }

  let style = theme.block.panel(false);
  let mut line = style.draw(grid, area).title(" Help ", style.title).inner();
  let mut hint = |key: &str, desc: &str| {
    for (text, attrs) in [
      (" <", style.dim),
      (key, style.key),
      (": ", style.dim),
      (desc, style.text),
      ("> ", style.dim),
    ] {
      grid.draw_text(line.take_left(text.width() as u16), text, attrs);
    }
  };

  // The find bar takes the keys while it is open.
  if state.search.is_some() {
    for (key, desc) in [
      ("Enter", "Next match up"),
      ("S-Enter", "Next match down"),
      ("Up/Down", "Next match up/down"),
      ("Esc", "Close search"),
    ] {
      hint(key, desc);
    }
    return;
  }

  let group = state.keymap_group();
  let items: &[Action] = match group {
    KeymapGroup::Tasks => &[
      Action::ToggleFocus,
      Action::ShowQuit,
      Action::NextTask,
      Action::PrevTask,
      Action::StartTask,
      Action::StopTask,
      Action::RestartTask,
      Action::Search,
      Action::Zoom,
      Action::ShowCommandsMenu,
      Action::ShowRunnerSelect {
        tab: RunnerTab::Switch,
      },
      Action::ToggleKeymapWindow,
    ],
    KeymapGroup::Term => &[Action::ToggleFocus, Action::Search],
    KeymapGroup::Copy => &[
      Action::CopyModeEnd,
      Action::CopyModeCopy,
      Action::Search,
      Action::SearchNext,
      Action::CopyModeLeave,
    ],
  };

  for action in items {
    let Some(key) = keymap.key(group, action) else {
      continue;
    };
    hint(&key.to_string(), &action.desc());
  }
}
