//! `git status --porcelain=v2 --branch -z` and `git diff --numstat -z`,
//! parsed: the branch, how it stands with its upstream, and the files
//! changed.

use std::collections::HashMap;

/// What is checked out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Head {
  Branch(String),
  /// No branch: the commit, shortened.
  Detached(String),
  /// A branch without a commit yet.
  Unborn(String),
}

/// The branch the checked-out one follows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Upstream {
  pub name: String,
  /// Commits here that it does not have.
  pub ahead: u32,
  /// Commits it has that are not here.
  pub behind: u32,
  /// Set as the upstream, but not among the remote's branches as last
  /// fetched.
  pub gone: bool,
}

/// Where a change is: a file changed both in the index and in the
/// working tree is two changes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Area {
  /// Unmerged: both sides changed it.
  Conflict,
  /// In the index, to be committed.
  Staged,
  /// In the working tree only; untracked files are here too.
  Unstaged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
  Modified,
  Added,
  Deleted,
  Renamed,
  Copied,
  TypeChanged,
  Untracked,
  Conflict,
}

impl Kind {
  /// The letter git shows for it.
  pub fn letter(self) -> char {
    match self {
      Kind::Modified => 'M',
      Kind::Added => 'A',
      Kind::Deleted => 'D',
      Kind::Renamed => 'R',
      Kind::Copied => 'C',
      Kind::TypeChanged => 'T',
      Kind::Untracked => '?',
      Kind::Conflict => 'U',
    }
  }

  fn of(code: u8) -> Option<Kind> {
    match code {
      b'M' => Some(Kind::Modified),
      b'A' => Some(Kind::Added),
      b'D' => Some(Kind::Deleted),
      b'R' => Some(Kind::Renamed),
      b'C' => Some(Kind::Copied),
      b'T' => Some(Kind::TypeChanged),
      _ => None,
    }
  }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChange {
  /// From the repository's root, with `/`; an untracked directory ends
  /// in `/`.
  pub path: String,
  /// Where a renamed or copied file was.
  pub orig: Option<String>,
  pub kind: Kind,
  pub area: Area,
  /// Lines added and removed; none for a binary file, or when not
  /// counted.
  pub added: Option<u32>,
  pub removed: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
  pub head: Head,
  pub upstream: Option<Upstream>,
  /// Conflicts first, then what is staged, then the rest.
  pub files: Vec<FileChange>,
}

/// Reads the output of `git status --porcelain=v2 --branch -z`. Lines it
/// does not know are skipped: git adds header lines over time.
pub fn parse(out: &[u8]) -> Status {
  let mut oid = String::new();
  let mut head = String::new();
  let mut upstream: Option<String> = None;
  let mut ab: Option<(u32, u32)> = None;
  let mut conflicts = Vec::new();
  let mut staged = Vec::new();
  let mut unstaged = Vec::new();
  let mut untracked = Vec::new();

  let mut records = out.split(|b| *b == 0).filter(|r| !r.is_empty());
  while let Some(record) = records.next() {
    let text = String::from_utf8_lossy(record);
    let mut change = |path: &str, orig: Option<String>, kind, area| {
      let change = FileChange {
        path: path.to_string(),
        orig,
        kind,
        area,
        added: None,
        removed: None,
      };
      match (area, kind) {
        (Area::Conflict, _) => conflicts.push(change),
        (Area::Staged, _) => staged.push(change),
        (Area::Unstaged, Kind::Untracked) => untracked.push(change),
        (Area::Unstaged, _) => unstaged.push(change),
      }
    };
    if let Some(header) = text.strip_prefix("# ") {
      let (key, value) = header.split_once(' ').unwrap_or((header, ""));
      match key {
        "branch.oid" => oid = value.to_string(),
        "branch.head" => head = value.to_string(),
        "branch.upstream" => upstream = Some(value.to_string()),
        "branch.ab" => {
          let mut counts = value.split(' ').map(|n| {
            n.trim_start_matches(['+', '-']).parse::<u32>().unwrap_or(0)
          });
          ab = Some((counts.next().unwrap_or(0), counts.next().unwrap_or(0)));
        }
        _ => (),
      }
    } else if let Some(entry) = text.strip_prefix("1 ") {
      // XY sub mH mI mW hH hI path
      let mut fields = entry.splitn(8, ' ');
      let xy = fields.next().unwrap_or("").as_bytes();
      let Some(path) = fields.nth(6) else { continue };
      for (code, area) in [(0, Area::Staged), (1, Area::Unstaged)] {
        if let Some(kind) = xy.get(code).copied().and_then(Kind::of) {
          change(path, None, kind, area);
        }
      }
    } else if let Some(entry) = text.strip_prefix("2 ") {
      // XY sub mH mI mW hH hI Xscore path, then the original path.
      let orig = records
        .next()
        .map(|orig| String::from_utf8_lossy(orig).into_owned());
      let mut fields = entry.splitn(9, ' ');
      let xy = fields.next().unwrap_or("").as_bytes();
      let Some(path) = fields.nth(7) else { continue };
      for (code, area) in [(0, Area::Staged), (1, Area::Unstaged)] {
        if let Some(kind) = xy.get(code).copied().and_then(Kind::of) {
          let moved = matches!(kind, Kind::Renamed | Kind::Copied);
          change(path, orig.clone().filter(|_| moved), kind, area);
        }
      }
    } else if let Some(entry) = text.strip_prefix("u ") {
      // XY sub m1 m2 m3 mW h1 h2 h3 path
      if let Some(path) = entry.splitn(10, ' ').nth(9) {
        change(path, None, Kind::Conflict, Area::Conflict);
      }
    } else if let Some(path) = text.strip_prefix("? ") {
      change(path, None, Kind::Untracked, Area::Unstaged);
    }
  }

  let head = if oid == "(initial)" {
    Head::Unborn(head)
  } else if head == "(detached)" || head.is_empty() {
    Head::Detached(oid.chars().take(7).collect())
  } else {
    Head::Branch(head)
  };
  let upstream = upstream.map(|name| {
    let (ahead, behind) = ab.unwrap_or((0, 0));
    Upstream {
      name,
      ahead,
      behind,
      gone: ab.is_none(),
    }
  });
  let mut files = conflicts;
  files.append(&mut staged);
  files.append(&mut unstaged);
  files.append(&mut untracked);
  Status {
    head,
    upstream,
    files,
  }
}

/// Reads the output of `git diff --numstat -z`: for each path, the lines
/// added and removed, or none for a binary file. A renamed file is under
/// its new path.
pub fn parse_numstat(
  out: &[u8],
) -> HashMap<String, (Option<u32>, Option<u32>)> {
  let mut counts = HashMap::new();
  let mut records = out.split(|b| *b == 0);
  while let Some(record) = records.next() {
    let text = String::from_utf8_lossy(record);
    let mut fields = text.splitn(3, '\t');
    let (Some(added), Some(removed), Some(path)) =
      (fields.next(), fields.next(), fields.next())
    else {
      continue;
    };
    // A rename has no path here: the old and the new one follow.
    let path = if path.is_empty() {
      let _old = records.next();
      match records.next() {
        Some(new) => String::from_utf8_lossy(new).into_owned(),
        None => continue,
      }
    } else {
      path.to_string()
    };
    counts.insert(path, (added.parse().ok(), removed.parse().ok()));
  }
  counts
}

#[cfg(test)]
mod tests {
  use super::*;

  fn status(records: &[&str]) -> Status {
    let mut out = Vec::new();
    for record in records {
      out.extend_from_slice(record.as_bytes());
      out.push(0);
    }
    parse(&out)
  }

  const HASHES: &str = "N... 100644 100644 100644 1111111 2222222";

  #[test]
  fn reads_the_branch_and_how_it_stands_with_its_upstream() {
    let st = status(&[
      "# branch.oid 3fa92c1d0000000000000000000000000000beef",
      "# branch.head feat-#13193",
      "# branch.upstream origin/feat-#13193",
      "# branch.ab +2 -3",
    ]);
    assert_eq!(st.head, Head::Branch("feat-#13193".to_string()));
    assert_eq!(
      st.upstream,
      Some(Upstream {
        name: "origin/feat-#13193".to_string(),
        ahead: 2,
        behind: 3,
        gone: false,
      })
    );
    assert!(st.files.is_empty());
  }

  /// An upstream that is set but was deleted on the remote has no counts.
  #[test]
  fn an_upstream_without_counts_is_gone() {
    let st = status(&[
      "# branch.oid 3fa92c1d0000000000000000000000000000beef",
      "# branch.head bug-#13159",
      "# branch.upstream origin/bug-#13159",
    ]);
    assert!(st.upstream.unwrap().gone);
  }

  #[test]
  fn tells_a_detached_head_and_a_branch_without_commits() {
    let st = status(&[
      "# branch.oid 3fa92c1d0000000000000000000000000000beef",
      "# branch.head (detached)",
    ]);
    assert_eq!(st.head, Head::Detached("3fa92c1".to_string()));
    let st = status(&["# branch.oid (initial)", "# branch.head main"]);
    assert_eq!(st.head, Head::Unborn("main".to_string()));
    assert_eq!(st.upstream, None);
  }

  /// A file changed in the index and again in the working tree is listed
  /// in both; the groups come conflicts, staged, unstaged, untracked.
  #[test]
  fn lists_each_change_in_its_area() {
    let st = status(&[
      "# branch.oid 3fa92c1d0000000000000000000000000000beef",
      "# branch.head main",
      &format!("1 .M {HASHES} src/b.rs"),
      &format!("1 MM {HASHES} src/a file.rs"),
      &format!("1 A. {HASHES} new.rs"),
      &format!("1 .D {HASHES} gone.rs"),
      "? notes/",
      "u UU N... 100644 100644 100644 100644 1111111 2222222 3333333 both.rs",
    ]);
    let listed: Vec<(&str, Area, char)> = st
      .files
      .iter()
      .map(|f| (f.path.as_str(), f.area, f.kind.letter()))
      .collect();
    assert_eq!(
      listed,
      vec![
        ("both.rs", Area::Conflict, 'U'),
        ("src/a file.rs", Area::Staged, 'M'),
        ("new.rs", Area::Staged, 'A'),
        ("src/b.rs", Area::Unstaged, 'M'),
        ("src/a file.rs", Area::Unstaged, 'M'),
        ("gone.rs", Area::Unstaged, 'D'),
        ("notes/", Area::Unstaged, '?'),
      ]
    );
  }

  #[test]
  fn a_rename_keeps_where_the_file_was() {
    let st = status(&[
      &format!("2 R. {HASHES} R100 src/new name.rs"),
      "src/old.rs",
      &format!("1 .M {HASHES} after.rs"),
    ]);
    assert_eq!(st.files.len(), 2);
    assert_eq!(st.files[0].path, "src/new name.rs");
    assert_eq!(st.files[0].orig.as_deref(), Some("src/old.rs"));
    assert_eq!(st.files[0].kind, Kind::Renamed);
    assert_eq!(st.files[1].path, "after.rs");
  }

  #[test]
  fn counts_lines_per_path() {
    let out = b"9\t3\tsrc/a.rs\0-\t-\timg/logo.png\x000\t0\t\0old.rs\0new.rs\0";
    let counts = parse_numstat(out);
    assert_eq!(counts["src/a.rs"], (Some(9), Some(3)));
    assert_eq!(counts["img/logo.png"], (None, None));
    assert_eq!(counts["new.rs"], (Some(0), Some(0)));
    assert_eq!(counts.len(), 3);
  }
}
