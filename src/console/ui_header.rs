use unicode_width::UnicodeWidthStr;

use crate::console::theme::Theme;
use crate::runner::{RunnerKind, RunnerSpec};
use crate::term::{Grid, Rgb, grid::Rect, style::Style};

/// The badge at the bar's left end. `≡` is in every monospace font and
/// reads as "menu".
const LOGO: &str = " ≡ dekit ";

/// How far a hovered cell's background moves toward the text color.
const HOVER: u8 = 24;

/// What the header bar shows: the runner the console lives in and the
/// dekit versions of the clients attached.
pub struct HeaderInfo<'a> {
  pub runner: Option<&'a RunnerSpec>,
  /// The runner's own dekit version.
  pub version: &'a str,
  pub client_versions: Vec<&'a str>,
  /// Where the mouse is, for the hover highlight.
  pub hover: Option<(u16, u16)>,
  /// A control drawn highlighted whatever the mouse does: the one whose
  /// menu is open.
  pub held: Option<Rect>,
  /// How many changes the config on disk has for the tasks.
  pub config_changes: usize,
  /// The config on disk does not load.
  pub config_error: bool,
  /// The repository the project is in, when it is in one: the views
  /// get their tabs, and the branch its place at the right.
  pub git: Option<HeaderGit>,
}

/// What the bar shows of the project's repository.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderGit {
  /// The git view is the one shown.
  pub shown: bool,
  /// How many changes there are to commit.
  pub changes: usize,
  pub branch: String,
  /// How the branch stands with its upstream, in a few cells.
  pub sync: String,
}

/// The longest branch name the bar shows whole.
const BRANCH_WIDTH: usize = 32;

/// The clickable cells of the bar as last drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeaderHits {
  /// The `dekit` badge; a click opens the main menu.
  pub logo: Option<Rect>,
  /// The runner name; a click opens the runner selector.
  pub runner: Option<Rect>,
  /// "config changed", or "config error"; a click opens the reload
  /// dialog, or says why the config does not load.
  pub reload: Option<Rect>,
  /// The tab of the task view.
  pub tasks: Option<Rect>,
  /// The tab of the git view.
  pub git: Option<Rect>,
  /// The branch; a click shows the git view.
  pub branch: Option<Rect>,
}

/// The name a runner goes by in the bar: its root's last component, or
/// `host`.
pub fn runner_name(runner: &RunnerSpec) -> String {
  match runner.kind {
    RunnerKind::Host => "host".to_string(),
    RunnerKind::Project => runner
      .root
      .file_name()
      .map(|name| name.to_string_lossy().into_owned())
      .unwrap_or_else(|| runner.root.display().to_string()),
  }
}

/// Draws the bar and returns where its clickable parts went.
pub fn render_header(
  area: Rect,
  grid: &mut Grid,
  info: &HeaderInfo,
  theme: &Theme,
) -> HeaderHits {
  let mut hits = HeaderHits::default();
  if area.height == 0 {
    return hits;
  }
  let block = &theme.block;
  let hovered = |hit: Rect| {
    info.hover.is_some_and(|(x, y)| hit.contains(x, y))
      || info.held.is_some_and(|held| hit.contains(held.x, held.y))
  };
  let bg = block.header_bg.into();
  let on_bg = |fg: Rgb| Style::default().bg(bg).fg(fg.into());
  let text = on_bg(block.text);
  let dim = on_bg(block.dim);
  grid.fill_area(area, ' ', text);

  // Right side first, so the left gets whatever is left.
  let mut line = area;
  let mut right: Vec<(String, Style)> = Vec::new();
  let mut others: Vec<&str> = info
    .client_versions
    .iter()
    .copied()
    .filter(|v| *v != info.version)
    .collect();
  others.sort_unstable();
  others.dedup();
  if info.client_versions.len() > 1 {
    right.push((format!("{} attached", info.client_versions.len()), dim));
    right.push(("  ".to_string(), dim));
  }
  if !others.is_empty() {
    right.push((
      format!("client v{} ≠ ", others.join(", v")),
      on_bg(block.key),
    ));
  }
  right.push((format!("v{} ", info.version), dim));
  let right_w: usize = right.iter().map(|(s, _)| s.width()).sum();
  let mut right_area = line.take_right(right_w.min(line.width as usize) as u16);
  for (s, attrs) in &right {
    grid.draw_text(right_area.take_left(s.width() as u16), s, *attrs);
  }

  // The branch, left of the versions, while it leaves room for the
  // rest of the bar.
  if let Some(git) = &info.git {
    let mut branch = git.branch.clone();
    if branch.width() > BRANCH_WIDTH {
      branch = branch.chars().take(BRANCH_WIDTH - 1).collect();
      branch.push('…');
    }
    let caption = if git.sync.is_empty() {
      format!(" {branch} ")
    } else {
      format!(" {branch}  {} ", git.sync)
    };
    if caption.width() + 40 <= line.width as usize {
      let hit = line.take_right(caption.width() as u16);
      let mut chip_bg = theme.git.accent_bg;
      if hovered(hit) {
        chip_bg = chip_bg.blend(theme.git.accent_fg, HOVER);
      }
      let chip = Style::default()
        .bg(chip_bg.into())
        .fg(theme.git.accent_fg.into())
        .set_bold(true);
      grid.draw_text(hit, &caption, chip);
      hits.branch = Some(hit);
    }
  }

  let logo = line.take_left(LOGO.width() as u16);
  let mut logo_bg = block.logo_bg;
  if hovered(logo) {
    logo_bg = logo_bg.blend(block.logo_fg, HOVER);
  }
  grid.draw_text(
    logo,
    LOGO,
    Style::default()
      .bg(logo_bg.into())
      .fg(block.logo_fg.into())
      .set_bold(true),
  );
  hits.logo = Some(logo);

  if let Some(runner) = info.runner {
    let name = runner_name(runner);
    let caption = format!(" {name} ");
    if caption.width() > line.width as usize {
      return hits;
    }
    let hit = line.take_left(caption.width() as u16);
    let mut project_bg = block.header_bg;
    if hovered(hit) {
      project_bg = project_bg.blend(block.text, HOVER);
    }
    let project = Style::default()
      .bg(project_bg.into())
      .fg(block.header_project.into())
      .set_bold(true);
    grid.fill_area(hit, ' ', project);
    let mut cells = hit;
    cells.take_left(1);
    grid.draw_text(cells.take_left(name.width() as u16), &name, project);
    hits.runner = Some(hit);
  }

  // The views, each a tab: the one shown is cut out of the bar.
  if let Some(git) = &info.git {
    let count = match git.changes {
      0 => String::new(),
      n => format!("{n} "),
    };
    let tabs = [
      (" Tasks ", "", !git.shown, &mut hits.tasks),
      (" Git ", count.as_str(), git.shown, &mut hits.git),
    ];
    line.take_left(1);
    for (label, count, shown, hit_slot) in tabs {
      let width = (label.width() + count.width()) as u16;
      if width + 1 > line.width {
        break;
      }
      let mut hit = line.take_left(width);
      line.take_left(1);
      *hit_slot = Some(hit);
      let tab_bg = if shown {
        block.bg
      } else if hovered(hit) {
        block.header_bg.blend(block.text, HOVER)
      } else {
        block.header_bg
      };
      let fg = if shown {
        block.title_focused
      } else {
        block.dim
      };
      let tab = Style::default().bg(tab_bg.into()).fg(fg.into());
      let label_area = hit.take_left(label.width() as u16);
      grid.draw_text(label_area, label, tab.set_bold(shown));
      grid.draw_text(hit, count, tab.fg(theme.git.warn.into()));
    }
  }

  if info.runner.is_none() {
    return hits;
  }
  // What the config and the tasks disagree on, each a button.
  let mut badge = |text: String, fg: Rgb| -> Option<Rect> {
    let caption = format!(" {text} ");
    if caption.width() + 1 > line.width as usize {
      return None;
    }
    line.take_left(1);
    let hit = line.take_left(caption.width() as u16);
    let mut bg = block.header_bg.blend(fg, 40);
    if hovered(hit) {
      bg = bg.blend(block.text, HOVER);
    }
    grid.draw_text(hit, &caption, Style::default().bg(bg.into()).fg(fg.into()));
    Some(hit)
  };
  if info.config_error {
    hits.reload =
      badge("\u{2715} config error".to_string(), theme.tasks.failed);
  } else if info.config_changes > 0 {
    hits.reload = badge(
      format!("● config changed: {}", info.config_changes),
      block.key,
    );
  }
  hits
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::*;
  use crate::term::{Size, grid::Pos};

  fn row_text(grid: &Grid) -> String {
    let mut out = grid.row_text(0);
    while out.chars().count() < grid.size().width as usize {
      out.push(' ');
    }
    out
  }

  fn bg_at(grid: &Grid, col: u16) -> crate::term::Color {
    grid.style_at(Pos { col, row: 0 }).unwrap().bg
  }

  fn spec(kind: RunnerKind, root: &str) -> RunnerSpec {
    RunnerSpec {
      kind,
      root: PathBuf::from(root),
    }
  }

  fn grid(width: u16) -> Grid {
    Grid::new(Size { width, height: 1 }, 0)
  }

  fn app_info<'a>(
    runner: Option<&'a RunnerSpec>,
    hover: Option<(u16, u16)>,
  ) -> HeaderInfo<'a> {
    HeaderInfo {
      runner,
      version: "0.10.1",
      client_versions: vec!["0.10.1"],
      hover,
      held: None,
      config_changes: 0,
      config_error: false,
      git: None,
    }
  }

  #[test]
  fn names_the_project_and_offers_it_for_clicking() {
    let mut grid = grid(40);
    let runner = spec(RunnerKind::Project, "/home/me/app");
    let hits = render_header(
      grid.area(),
      &mut grid,
      &app_info(Some(&runner), None),
      &Theme::dark(),
    );
    assert_eq!(row_text(&grid), " ≡ dekit  app                   v0.10.1 ");
    assert_eq!(hits.logo, Some(Rect::new(0, 0, 9, 1)));
    assert_eq!(hits.runner, Some(Rect::new(9, 0, 5, 1)));
  }

  /// A config that does not load says so, in place of its changes, which
  /// are unknown then.
  #[test]
  fn a_config_that_does_not_load_says_so() {
    let mut grid = grid(60);
    let runner = spec(RunnerKind::Project, "/home/me/app");
    let info = HeaderInfo {
      config_changes: 2,
      config_error: true,
      ..app_info(Some(&runner), None)
    };
    let hits = render_header(grid.area(), &mut grid, &info, &Theme::dark());
    let row = row_text(&grid);
    assert!(row.contains(" \u{2715} config error "), "{row}");
    assert!(!row.contains("changed"), "{row}");
    assert_eq!(hits.reload.map(|hit| hit.x), Some(15));
  }

  #[test]
  fn the_mouse_lightens_what_it_is_over() {
    let theme = Theme::dark();
    let runner = spec(RunnerKind::Project, "/home/me/app");
    let mut plain = grid(40);
    render_header(
      plain.area(),
      &mut plain,
      &app_info(Some(&runner), None),
      &theme,
    );
    let mut over_logo = grid(40);
    render_header(
      over_logo.area(),
      &mut over_logo,
      &app_info(Some(&runner), Some((3, 0))),
      &theme,
    );
    let mut over_name = grid(40);
    render_header(
      over_name.area(),
      &mut over_name,
      &app_info(Some(&runner), Some((10, 0))),
      &theme,
    );

    assert_eq!(bg_at(&plain, 3), theme.block.logo_bg.into());
    assert_ne!(bg_at(&over_logo, 3), bg_at(&plain, 3));
    assert_eq!(bg_at(&over_logo, 10), bg_at(&plain, 10), "name untouched");
    assert_ne!(bg_at(&over_name, 10), bg_at(&plain, 10));
    assert_eq!(bg_at(&over_name, 3), bg_at(&plain, 3), "logo untouched");
    assert_eq!(bg_at(&over_name, 20), bg_at(&plain, 20), "bar untouched");
    assert_eq!(row_text(&over_name), row_text(&plain));

    // Held, the badge stays lit with the mouse elsewhere.
    let mut held = grid(40);
    render_header(
      held.area(),
      &mut held,
      &HeaderInfo {
        held: Some(Rect::new(0, 0, 9, 1)),
        config_changes: 0,
        config_error: false,
        ..app_info(Some(&runner), Some((30, 0)))
      },
      &theme,
    );
    assert_eq!(bg_at(&held, 3), bg_at(&over_logo, 3));
  }

  #[test]
  fn flags_clients_on_another_version() {
    let mut grid = grid(70);
    let runner = spec(RunnerKind::Host, "/home/me/.config/dekit/host");
    render_header(
      grid.area(),
      &mut grid,
      &HeaderInfo {
        runner: Some(&runner),
        version: "0.10.1",
        client_versions: vec!["0.9.6", "0.10.1", "0.9.6"],
        hover: None,
        held: None,
        config_changes: 0,
        config_error: false,
        git: None,
      },
      &Theme::dark(),
    );
    let row = row_text(&grid);
    assert!(row.starts_with(" ≡ dekit  host   "), "{row:?}");
    assert!(
      row.ends_with("  3 attached  client v0.9.6 ≠ v0.10.1 "),
      "{row:?}"
    );
  }

  /// In a repository, the views get tabs after the project's name and
  /// the branch goes left of the versions; each is a button.
  #[test]
  fn shows_the_views_and_the_branch_of_a_repository() {
    let theme = Theme::dark();
    let runner = spec(RunnerKind::Project, "/home/me/app");
    let git = |shown: bool, changes: usize| HeaderGit {
      shown,
      changes,
      branch: "feat-#13193".to_string(),
      sync: "↑2 ↓3".to_string(),
    };
    let mut wide = grid(80);
    let info = HeaderInfo {
      git: Some(git(false, 5)),
      ..app_info(Some(&runner), None)
    };
    let hits = render_header(wide.area(), &mut wide, &info, &theme);
    assert_eq!(
      row_text(&wide),
      format!(
        " ≡ dekit  app   Tasks   Git 5 {} feat-#13193  ↑2 ↓3 v0.10.1 ",
        " ".repeat(22)
      )
    );
    assert_eq!(hits.tasks, Some(Rect::new(15, 0, 7, 1)));
    assert_eq!(hits.git, Some(Rect::new(23, 0, 7, 1)));
    assert_eq!(hits.branch, Some(Rect::new(52, 0, 20, 1)));
    // The view shown is the tab cut out of the bar.
    assert_eq!(bg_at(&wide, 16), theme.block.bg.into());
    assert_eq!(bg_at(&wide, 24), theme.block.header_bg.into());

    let mut shown = grid(80);
    let info = HeaderInfo {
      git: Some(git(true, 0)),
      ..app_info(Some(&runner), None)
    };
    let hits = render_header(shown.area(), &mut shown, &info, &theme);
    assert!(row_text(&shown).contains(" Tasks   Git   "), "no count");
    assert_eq!(hits.git, Some(Rect::new(23, 0, 5, 1)));
    assert_eq!(bg_at(&shown, 24), theme.block.bg.into());
    assert_eq!(bg_at(&shown, 16), theme.block.header_bg.into());

    // Too narrow for the branch: the tabs stay.
    let mut narrow = grid(50);
    let info = HeaderInfo {
      git: Some(git(false, 5)),
      ..app_info(Some(&runner), None)
    };
    let hits = render_header(narrow.area(), &mut narrow, &info, &theme);
    assert_eq!(hits.branch, None);
    assert!(hits.git.is_some());
  }

  #[test]
  fn without_a_runner_only_the_logo_is_clickable() {
    let mut grid = grid(20);
    let hits = render_header(
      grid.area(),
      &mut grid,
      &app_info(None, None),
      &Theme::dark(),
    );
    assert_eq!(row_text(&grid), " ≡ dekit    v0.10.1 ");
    assert_eq!(hits.logo, Some(Rect::new(0, 0, 9, 1)));
    assert_eq!(hits.runner, None);
  }
}
