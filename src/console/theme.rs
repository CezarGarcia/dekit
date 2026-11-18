use crate::config::tui::ThemeMode;
use crate::protocol::caps::Background;
use crate::term::{
  Grid, Palette, Rgb,
  draw::{Block, BorderType},
  grid::Rect,
  style::Style,
};

/// The console paints every color itself, so it looks the same on any
/// terminal; `tui.theme` picks the light or the dark palette.
pub struct Theme {
  pub dark: bool,
  /// For the task screens; its default colors are the blocks' `text` and
  /// `bg`.
  pub palette: Palette,
  pub block: BlockTheme,
  pub tasks: TaskListTheme,
  pub git: GitTheme,
}

/// The git view. Its panels are told apart by their backgrounds, each
/// under a title bar, where the task view draws boxes.
pub struct GitTheme {
  /// Behind the diff.
  pub bg: Rgb,
  /// Behind the panels on the left.
  pub surface: Rgb,
  /// A panel's title bar.
  pub bar: Rgb,
  /// The hint line at the bottom.
  pub raised: Rgb,
  /// The selected row.
  pub selected: Rgb,
  /// A row or a button under the mouse.
  pub hover: Rgb,
  /// A button.
  pub chip: Rgb,
  pub text: Rgb,
  /// Secondary text: labels, counts, a hunk's header.
  pub dim: Rgb,
  /// What recedes: a file's directory, line numbers.
  pub faint: Rgb,
  /// The line between the panels and the diff.
  pub border: Rgb,
  /// What has the focus, and the branch in the header.
  pub accent: Rgb,
  pub accent_bg: Rgb,
  pub accent_fg: Rgb,
  /// Lines and files added, and a branch level with its upstream.
  pub add: Rgb,
  pub add_bg: Rgb,
  /// Lines and files removed, conflicts, and what failed.
  pub del: Rgb,
  pub del_bg: Rgb,
  /// Files modified, and commits to pull.
  pub warn: Rgb,
  /// Files renamed, and files not in the repository yet.
  pub info: Rgb,
}

pub struct BlockTheme {
  pub bg: Rgb,
  pub modal_bg: Rgb,
  /// The header bar across the top: lighter than the blocks under it.
  pub header_bg: Rgb,
  /// The `dekit` badge at the bar's left end.
  pub logo_bg: Rgb,
  pub logo_fg: Rgb,
  /// The runner's name in the bar.
  pub header_project: Rgb,
  /// Everything under an open modal moves `shade_amount`/255 toward
  /// `shade`.
  pub shade: Rgb,
  pub shade_amount: u8,
  /// The selected row of a list.
  pub selected_bg: Rgb,
  pub text: Rgb,
  /// Secondary text, like the terminal's own title.
  pub dim: Rgb,
  /// What an empty text field shows in place of a value: fainter than
  /// `dim`, so it does not read as one.
  pub placeholder: Rgb,
  /// The line under a text field without the focus; with it, the line is
  /// `border_focused`.
  pub field_line: Rgb,
  /// What went wrong: a refused value, a failed command.
  pub error: Rgb,
  /// Key names in help and menus.
  pub key: Rgb,
  pub border: Rgb,
  pub border_focused: Rgb,
  pub title: Rgb,
  pub title_focused: Rgb,
  /// A title badge that needs attention: QUITTING.
  pub alert: Rgb,
  pub alert_bg: Rgb,
}

/// Group counts: `failed` if any failed, `up` if all are up, else `down`.
pub struct TaskListTheme {
  /// The `/` or `/…` after a group name.
  pub mark: Rgb,
  pub up: Rgb,
  /// On its way up or down: STARTING, STOPPING.
  pub pending: Rgb,
  pub down: Rgb,
  /// Ended with code 0: DONE, DOWN (0).
  pub done: Rgb,
  /// FAILED, DOWN (code), NOT READY.
  pub failed: Rgb,
}

impl Theme {
  /// The theme `tui.theme` asks for: `auto` follows the attached
  /// terminal's background, and is dark until a client says.
  pub fn resolve(mode: ThemeMode, background: Option<Background>) -> Self {
    match (mode, background) {
      (ThemeMode::Light, _) | (ThemeMode::Auto, Some(Background::Light)) => {
        Theme::light()
      }
      (ThemeMode::Dark, _) | (ThemeMode::Auto, _) => Theme::dark(),
    }
  }

  pub fn dark() -> Self {
    let bg = Rgb(0x16, 0x1b, 0x22);
    let text = Rgb(0xd1, 0xd7, 0xe0);
    Theme {
      dark: true,
      palette: Palette {
        fg: text,
        bg,
        ansi: [
          Rgb(0x48, 0x4f, 0x58),
          Rgb(0xff, 0x7b, 0x72),
          Rgb(0x3f, 0xb9, 0x50),
          Rgb(0xd2, 0x99, 0x22),
          Rgb(0x58, 0xa6, 0xff),
          Rgb(0xbc, 0x8c, 0xff),
          Rgb(0x39, 0xc5, 0xcf),
          Rgb(0xb1, 0xba, 0xc4),
          Rgb(0x6e, 0x76, 0x81),
          Rgb(0xff, 0xa1, 0x98),
          Rgb(0x56, 0xd3, 0x64),
          Rgb(0xe3, 0xb3, 0x41),
          Rgb(0x79, 0xc0, 0xff),
          Rgb(0xd2, 0xa8, 0xff),
          Rgb(0x56, 0xd4, 0xdd),
          Rgb(0xff, 0xff, 0xff),
        ],
      },
      block: BlockTheme {
        bg,
        modal_bg: Rgb(0x21, 0x28, 0x30),
        header_bg: Rgb(0x2b, 0x33, 0x3d),
        logo_bg: Rgb(0x27, 0x51, 0x93),
        logo_fg: Rgb(0xff, 0xff, 0xff),
        header_project: Rgb(0x56, 0xd4, 0xdd),
        shade: Rgb(0x00, 0x00, 0x00),
        shade_amount: 128,
        selected_bg: Rgb(0x42, 0x4a, 0x57),
        text,
        dim: Rgb(0x91, 0x98, 0xa1),
        placeholder: Rgb(0x5f, 0x66, 0x70),
        field_line: Rgb(0x45, 0x4d, 0x57),
        error: Rgb(0xff, 0xa1, 0x98),
        key: Rgb(0xe3, 0xb3, 0x41),
        border: Rgb(0x3d, 0x44, 0x4d),
        border_focused: Rgb(0x6c, 0xae, 0xff),
        title: Rgb(0x91, 0x98, 0xa1),
        title_focused: Rgb(0xf0, 0xf6, 0xfc),
        alert: Rgb(0xff, 0xff, 0xff),
        alert_bg: Rgb(0xda, 0x36, 0x33),
      },
      tasks: TaskListTheme {
        mark: Rgb(0x91, 0x98, 0xa1),
        up: Rgb(0x56, 0xd3, 0x64),
        pending: Rgb(0xe3, 0xb3, 0x41),
        down: Rgb(0xa3, 0xac, 0xb8),
        done: Rgb(0x79, 0xc0, 0xff),
        failed: Rgb(0xff, 0xa1, 0x98),
      },
      git: GitTheme {
        bg: Rgb(0x0f, 0x11, 0x16),
        surface: Rgb(0x17, 0x1a, 0x22),
        bar: Rgb(0x20, 0x24, 0x2f),
        raised: Rgb(0x28, 0x2d, 0x3a),
        selected: Rgb(0x2d, 0x35, 0x52),
        hover: Rgb(0x1e, 0x22, 0x2c),
        chip: Rgb(0x2c, 0x32, 0x40),
        text: Rgb(0xe4, 0xe7, 0xf0),
        dim: Rgb(0x9d, 0xa4, 0xb5),
        faint: Rgb(0x66, 0x6e, 0x80),
        border: Rgb(0x3a, 0x40, 0x50),
        accent: Rgb(0x8e, 0xa2, 0xff),
        accent_bg: Rgb(0x2a, 0x33, 0x66),
        accent_fg: Rgb(0xe3, 0xe7, 0xff),
        add: Rgb(0x74, 0xd6, 0x95),
        add_bg: Rgb(0x12, 0x26, 0x1a),
        del: Rgb(0xf3, 0x8d, 0x8d),
        del_bg: Rgb(0x2e, 0x17, 0x19),
        warn: Rgb(0xe6, 0xbb, 0x62),
        info: Rgb(0x62, 0xcb, 0xd9),
      },
    }
  }

  pub fn light() -> Self {
    let bg = Rgb(0xf6, 0xf8, 0xfa);
    let text = Rgb(0x1f, 0x23, 0x28);
    Theme {
      dark: false,
      palette: Palette {
        fg: text,
        bg,
        ansi: [
          Rgb(0x24, 0x29, 0x2f),
          Rgb(0xcf, 0x22, 0x2e),
          Rgb(0x11, 0x63, 0x29),
          Rgb(0x9a, 0x67, 0x00),
          Rgb(0x09, 0x69, 0xda),
          Rgb(0x82, 0x50, 0xdf),
          Rgb(0x1b, 0x7c, 0x83),
          Rgb(0x6e, 0x77, 0x81),
          Rgb(0x57, 0x60, 0x6a),
          Rgb(0xa4, 0x0e, 0x26),
          Rgb(0x1a, 0x7f, 0x37),
          Rgb(0xbf, 0x87, 0x00),
          Rgb(0x21, 0x8b, 0xff),
          Rgb(0xa4, 0x75, 0xf9),
          Rgb(0x31, 0x92, 0xaa),
          Rgb(0x8c, 0x95, 0x9f),
        ],
      },
      block: BlockTheme {
        bg,
        modal_bg: Rgb(0xff, 0xff, 0xff),
        header_bg: Rgb(0xe1, 0xe7, 0xee),
        logo_bg: Rgb(0x4a, 0x80, 0xc8),
        logo_fg: Rgb(0xff, 0xff, 0xff),
        header_project: Rgb(0x1b, 0x7c, 0x83),
        shade: Rgb(0x1f, 0x23, 0x28),
        shade_amount: 77,
        selected_bg: Rgb(0xdd, 0xe4, 0xec),
        text,
        dim: Rgb(0x59, 0x63, 0x6e),
        placeholder: Rgb(0x9a, 0xa3, 0xad),
        field_line: Rgb(0xc4, 0xcc, 0xd5),
        error: Rgb(0xd1, 0x24, 0x2f),
        key: Rgb(0x9a, 0x67, 0x00),
        border: Rgb(0xb8, 0xc2, 0xcc),
        border_focused: Rgb(0x05, 0x50, 0xae),
        title: Rgb(0x59, 0x63, 0x6e),
        title_focused: Rgb(0x1f, 0x23, 0x28),
        alert: Rgb(0xff, 0xff, 0xff),
        alert_bg: Rgb(0xcf, 0x22, 0x2e),
      },
      tasks: TaskListTheme {
        mark: Rgb(0x59, 0x63, 0x6e),
        up: Rgb(0x1a, 0x7f, 0x37),
        pending: Rgb(0x9a, 0x67, 0x00),
        down: Rgb(0x59, 0x63, 0x6e),
        done: Rgb(0x09, 0x69, 0xda),
        failed: Rgb(0xd1, 0x24, 0x2f),
      },
      git: GitTheme {
        bg: Rgb(0xff, 0xff, 0xff),
        surface: Rgb(0xf3, 0xf4, 0xf8),
        bar: Rgb(0xe8, 0xea, 0xf1),
        raised: Rgb(0xdd, 0xe0, 0xe9),
        selected: Rgb(0xd3, 0xd9, 0xf2),
        hover: Rgb(0xe9, 0xeb, 0xf2),
        chip: Rgb(0xdd, 0xe0, 0xe9),
        text: Rgb(0x18, 0x1b, 0x24),
        dim: Rgb(0x55, 0x5c, 0x6d),
        faint: Rgb(0x92, 0x99, 0xa8),
        border: Rgb(0xc5, 0xca, 0xd6),
        accent: Rgb(0x3f, 0x50, 0xc8),
        accent_bg: Rgb(0xdd, 0xe2, 0xfb),
        accent_fg: Rgb(0x27, 0x34, 0x9a),
        add: Rgb(0x17, 0x78, 0x3a),
        add_bg: Rgb(0xe6, 0xf6, 0xea),
        del: Rgb(0xbd, 0x32, 0x38),
        del_bg: Rgb(0xfd, 0xea, 0xea),
        warn: Rgb(0x8f, 0x5f, 0x07),
        info: Rgb(0x0d, 0x74, 0x83),
      },
    }
  }
}

/// The attrs of one drawn block, resolved from the theme.
#[derive(Clone, Copy)]
pub struct BlockStyle {
  pub border: Style,
  pub title: Style,
  pub text: Style,
  pub dim: Style,
  /// Italic, in the theme's `placeholder`.
  pub placeholder: Style,
  pub error: Style,
  pub key: Style,
  pub alert: Style,
}

impl BlockTheme {
  pub fn panel(&self, focused: bool) -> BlockStyle {
    self.style(self.bg, focused)
  }

  /// A modal has the focus while it is open.
  pub fn modal(&self) -> BlockStyle {
    self.style(self.modal_bg, true)
  }

  /// A modal's list row: `style` on the selected background when it is
  /// selected.
  pub fn modal_row(&self, style: BlockStyle, selected: bool) -> BlockStyle {
    if selected {
      style.on_bg(self.selected_bg)
    } else {
      style
    }
  }

  fn style(&self, bg: Rgb, focused: bool) -> BlockStyle {
    let on_bg = |fg: Rgb| Style::default().bg(bg.into()).fg(fg.into());
    let (border, title) = if focused {
      (self.border_focused, self.title_focused)
    } else {
      (self.border, self.title)
    };
    BlockStyle {
      border: on_bg(border),
      title: on_bg(title).set_bold(focused),
      text: on_bg(self.text),
      dim: on_bg(self.dim),
      placeholder: on_bg(self.placeholder).set_italic(true),
      error: on_bg(self.error),
      key: on_bg(self.key),
      alert: Style::default()
        .bg(self.alert_bg.into())
        .fg(self.alert.into())
        .set_bold(true),
    }
  }
}

impl BlockStyle {
  /// The same attrs on another background: a list row that is selected.
  pub fn on_bg(self, bg: Rgb) -> BlockStyle {
    let on_bg = |attrs: Style| Style {
      bg: bg.into(),
      ..attrs
    };
    BlockStyle {
      border: on_bg(self.border),
      title: on_bg(self.title),
      text: on_bg(self.text),
      dim: on_bg(self.dim),
      placeholder: on_bg(self.placeholder),
      error: on_bg(self.error),
      key: on_bg(self.key),
      alert: self.alert,
    }
  }

  /// Draws the border and clears the inside.
  pub fn draw<'a>(&self, grid: &'a mut Grid, area: Rect) -> Block<'a> {
    grid.fill_area(area.inner(1), ' ', self.text);
    grid.block(area, BorderType::Plain, self.border)
  }
}
