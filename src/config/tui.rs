use std::time::Duration;

use anyhow::{Result, bail};

use crate::cfg::{CfgCx, CfgNode, CfgObj, FromCfg};

const DEFAULT_SIDEBAR_WIDTH: usize = 30;
const DEFAULT_SIDEBAR_TITLE: &str = "Tasks";
const DEFAULT_FETCH_EVERY: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub struct TuiConfig {
  pub sidebar: SidebarConfig,
  pub tips: TipsConfig,
  pub zoom_tip: bool,
  pub theme: ThemeMode,
  pub git: GitConfig,
}

#[derive(Clone)]
pub struct GitConfig {
  /// How often the console fetches from the repository's remote while
  /// someone is attached; never when none.
  pub fetch_every: Option<Duration>,
}

/// `tui.git.fetch_every`: a duration, or `off`.
struct FetchEvery(Option<Duration>);

impl FromCfg for FetchEvery {
  fn from_cfg(node: &CfgNode<'_>, cx: &CfgCx) -> Result<Self> {
    match node.raw() {
      serde_yaml::Value::String(s) if s == "off" => Ok(FetchEvery(None)),
      serde_yaml::Value::Bool(false) => Ok(FetchEvery(None)),
      _ => match Duration::from_cfg(node, cx) {
        Ok(every) => Ok(FetchEvery(Some(every))),
        Err(_) => bail!(node.error("expected a duration such as 5m, or off")),
      },
    }
  }
}

/// The console's colors: `auto` follows the attached terminal's
/// background, light or dark.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeMode {
  #[default]
  Auto,
  Dark,
  Light,
}

impl ThemeMode {
  pub fn parse(s: &str) -> Option<Self> {
    match s {
      "auto" => Some(ThemeMode::Auto),
      "dark" => Some(ThemeMode::Dark),
      "light" => Some(ThemeMode::Light),
      _ => None,
    }
  }
}

impl FromCfg for ThemeMode {
  fn from_cfg(node: &CfgNode<'_>, _cx: &CfgCx) -> Result<Self> {
    match node.raw().as_str().and_then(ThemeMode::parse) {
      Some(mode) => Ok(mode),
      None => bail!(node.error("expected auto, dark, or light")),
    }
  }
}

#[derive(Clone)]
pub struct SidebarConfig {
  pub title: String,
  pub width: usize,
}

#[derive(Clone)]
pub struct TipsConfig {
  pub show: bool,
}

impl TuiConfig {
  pub(crate) fn builtin() -> Self {
    TuiConfig {
      sidebar: SidebarConfig {
        title: DEFAULT_SIDEBAR_TITLE.to_string(),
        width: DEFAULT_SIDEBAR_WIDTH,
      },
      tips: TipsConfig { show: true },
      zoom_tip: true,
      theme: ThemeMode::Auto,
      git: GitConfig {
        fetch_every: Some(DEFAULT_FETCH_EVERY),
      },
    }
  }

  pub(crate) fn merge(&mut self, obj: &CfgObj<'_>, cx: &CfgCx) -> Result<()> {
    let tui_obj = match obj.get("tui") {
      Some(node) => node.as_obj()?,
      None => return Ok(()),
    };
    tui_obj.known_keys(&["sidebar", "tips", "zoom_tip", "theme", "git"])?;

    if let Some(pl) = tui_obj.get("sidebar") {
      let pl = pl.as_obj()?;
      pl.known_keys(&["title", "width"])?;
      self.sidebar.title =
        pl.default("title", self.sidebar.title.clone(), cx)?;
      self.sidebar.width = pl.default("width", self.sidebar.width, cx)?;
    }

    if let Some(tips) = tui_obj.get("tips") {
      let tips = tips.as_obj()?;
      tips.known_keys(&["show"])?;
      self.tips.show = tips.default("show", self.tips.show, cx)?;
    }

    self.zoom_tip = tui_obj.default("zoom_tip", self.zoom_tip, cx)?;
    self.theme = tui_obj.default("theme", self.theme, cx)?;

    if let Some(git) = tui_obj.get("git") {
      let git = git.as_obj()?;
      git.known_keys(&["fetch_every"])?;
      if let Some(every) = git.optional::<FetchEvery>("fetch_every", cx)? {
        self.git.fetch_every = every.0;
      }
    }

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use crate::cfg::{CfgCx, CfgDoc};

  use super::*;

  fn merged(yaml: &str) -> Result<TuiConfig> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
    let cx = CfgCx::new(std::path::PathBuf::from("."));
    let doc = CfgDoc::from_value(value);
    let mut config = TuiConfig::builtin();
    config.merge(&doc.root().as_obj().unwrap(), &cx)?;
    Ok(config)
  }

  #[test]
  fn theme_defaults_to_auto_and_takes_each_value() {
    assert_eq!(TuiConfig::builtin().theme, ThemeMode::Auto);
    assert_eq!(merged("tui: {}").unwrap().theme, ThemeMode::Auto);
    assert_eq!(merged("tui: {theme: auto}").unwrap().theme, ThemeMode::Auto);
    assert_eq!(merged("tui: {theme: dark}").unwrap().theme, ThemeMode::Dark);
    assert_eq!(
      merged("tui: {theme: light}").unwrap().theme,
      ThemeMode::Light
    );
    let err = merged("tui: {theme: solarized}").err().unwrap();
    assert!(
      format!("{err:#}").contains("auto, dark, or light"),
      "{err:#}"
    );
  }

  #[test]
  fn the_fetch_takes_a_duration_or_off() {
    let every = |yaml: &str| merged(yaml).map(|tui| tui.git.fetch_every);
    assert_eq!(
      TuiConfig::builtin().git.fetch_every,
      Some(Duration::from_secs(300))
    );
    assert_eq!(
      every("tui: {git: {fetch_every: 30s}}").unwrap(),
      Some(Duration::from_secs(30))
    );
    assert_eq!(every("tui: {git: {fetch_every: off}}").unwrap(), None);
    assert_eq!(every("tui: {git: {fetch_every: false}}").unwrap(), None);
    assert_eq!(
      every("tui: {git: {}}").unwrap(),
      Some(Duration::from_secs(300))
    );
    let err = every("tui: {git: {fetch_every: soon}}").err().unwrap();
    assert!(format!("{err:#}").contains("5m, or off"), "{err:#}");
  }
}
