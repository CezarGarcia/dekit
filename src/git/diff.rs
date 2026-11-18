//! A file's diff as `git diff` prints it, parsed into lines with their
//! numbers on each side.

/// More lines than this are not kept: the view says how many were left
/// out.
const MAX_LINES: usize = 20_000;
/// Tab stops in a diff's text.
const TAB: usize = 4;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Diff {
  pub lines: Vec<DiffLine>,
  /// Said in place of lines when there are none: a binary file, a mode
  /// change.
  pub note: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffLine {
  /// `@@ -12,7 +12,9 @@ fn name`
  Hunk(String),
  Context {
    old: u32,
    new: u32,
    text: String,
  },
  Added {
    new: u32,
    text: String,
  },
  Removed {
    old: u32,
    text: String,
  },
  /// Said about the lines around it: no newline at the end of the file.
  Note(String),
}

impl Diff {
  pub fn note(text: impl Into<String>) -> Self {
    Diff {
      lines: Vec::new(),
      note: Some(text.into()),
    }
  }

  /// The highest line number on either side.
  pub fn last_line(&self) -> u32 {
    self
      .lines
      .iter()
      .map(|line| match line {
        DiffLine::Context { old, new, .. } => (*old).max(*new),
        DiffLine::Added { new, .. } => *new,
        DiffLine::Removed { old, .. } => *old,
        DiffLine::Hunk(_) | DiffLine::Note(_) => 0,
      })
      .max()
      .unwrap_or(0)
  }
}

/// Text safe and steady to draw in cells: tabs as spaces, no carriage
/// return at the end, and no control characters, which a file could use
/// to write escape sequences to the terminals attached.
pub fn clean(text: &str) -> String {
  let text = text.strip_suffix('\r').unwrap_or(text);
  let mut out = String::with_capacity(text.len());
  let mut col = 0;
  for c in text.chars() {
    if c == '\t' {
      let to = (col / TAB + 1) * TAB;
      out.extend(std::iter::repeat_n(' ', to - col));
      col = to;
    } else {
      out.push(if c.is_control() { '\u{fffd}' } else { c });
      col += 1;
    }
  }
  out
}

/// Reads the output of `git diff` for one file.
pub fn parse(out: &[u8]) -> Diff {
  let text = String::from_utf8_lossy(out);
  let mut diff = Diff::default();
  let mut binary = false;
  let mut mode = false;
  let mut renamed = false;
  // How many sides a hunk's lines are marked for: 1, or the parents of
  // a merge in a combined diff.
  let mut marks = 0;
  let (mut old, mut new) = (0, 0);
  let mut left_out = 0;

  for line in text.split('\n') {
    if line.starts_with("@@") {
      let at = line.chars().take_while(|c| *c == '@').count();
      marks = at.saturating_sub(1).max(1);
      (old, new) = hunk_start(line);
      push(&mut diff, &mut left_out, DiffLine::Hunk(clean(line)));
      continue;
    }
    if marks == 0 || line.starts_with("diff --") {
      // The header before the first hunk.
      marks = 0;
      binary |= line.starts_with("Binary files ")
        || line.starts_with("GIT binary patch");
      mode |= line.starts_with("old mode ") || line.starts_with("new mode ");
      renamed |= line.starts_with("rename from ");
      continue;
    }
    if let Some(note) = line.strip_prefix('\\') {
      let note = DiffLine::Note(clean(note.trim_start()));
      push(&mut diff, &mut left_out, note);
      continue;
    }
    let split = line
      .char_indices()
      .nth(marks)
      .map_or(line.len(), |(at, _)| at);
    let (prefix, body) = line.split_at(split);
    // The text ends with a newline, so the last piece is empty.
    if prefix.is_empty() && body.is_empty() {
      continue;
    }
    let text = clean(body);
    let line = if prefix.contains('+') {
      new += 1;
      DiffLine::Added { new: new - 1, text }
    } else if prefix.contains('-') {
      old += 1;
      DiffLine::Removed { old: old - 1, text }
    } else {
      old += 1;
      new += 1;
      DiffLine::Context {
        old: old - 1,
        new: new - 1,
        text,
      }
    };
    push(&mut diff, &mut left_out, line);
  }

  if left_out > 0 {
    diff
      .lines
      .push(DiffLine::Note(format!("{left_out} more lines not shown")));
  }
  if diff.lines.is_empty() {
    diff.note = Some(
      if binary {
        "Binary file"
      } else if mode {
        "Only the file's mode changed"
      } else if renamed {
        "Renamed, with no change inside"
      } else {
        "No lines changed"
      }
      .to_string(),
    );
  }
  diff
}

/// A file that is not in the repository yet, as if every line of it had
/// been added.
pub fn of_new_file(bytes: &[u8]) -> Diff {
  if is_binary(bytes) {
    return Diff::note("Binary file");
  }
  if bytes.is_empty() {
    return Diff::note("Empty file");
  }
  let text = String::from_utf8_lossy(bytes);
  let text = text.strip_suffix('\n').unwrap_or(&text);
  let mut diff = Diff::default();
  let mut left_out = 0;
  for (at, line) in text.split('\n').enumerate() {
    let line = DiffLine::Added {
      new: at as u32 + 1,
      text: clean(line),
    };
    push(&mut diff, &mut left_out, line);
  }
  if left_out > 0 {
    diff
      .lines
      .push(DiffLine::Note(format!("{left_out} more lines not shown")));
  }
  diff
}

/// As git tells: a NUL among the first bytes.
pub fn is_binary(bytes: &[u8]) -> bool {
  bytes.iter().take(8000).any(|b| *b == 0)
}

fn push(diff: &mut Diff, left_out: &mut usize, line: DiffLine) {
  if diff.lines.len() < MAX_LINES {
    diff.lines.push(line);
  } else {
    *left_out += 1;
  }
}

/// Where a hunk starts on the old and the new side: the first `-n` and
/// the last `+n` of its header.
fn hunk_start(header: &str) -> (u32, u32) {
  let number = |range: &str| -> u32 {
    range[1..]
      .split(',')
      .next()
      .and_then(|n| n.parse().ok())
      .unwrap_or(0)
  };
  let mut old = None;
  let mut new = 0;
  for word in header.split(' ').skip(1) {
    if word.starts_with('@') {
      break;
    }
    if word.starts_with('-') && old.is_none() {
      old = Some(number(word));
    } else if word.starts_with('+') {
      new = number(word);
    }
  }
  (old.unwrap_or(0), new)
}

#[cfg(test)]
mod tests {
  use super::*;

  const MODIFIED: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -12,4 +12,5 @@ fn load() {
   let a = 1;
-  let b = 2;
+  let b = 3;
+  let c = 4;
   done();
@@ -40,2 +41,2 @@ fn other() {
-\told();
+\tnew();\r
 }
\\ No newline at end of file
";

  #[test]
  fn numbers_each_line_on_its_side() {
    let diff = parse(MODIFIED.as_bytes());
    assert_eq!(diff.note, None);
    let text = |s: &str| s.to_string();
    assert_eq!(
      diff.lines,
      vec![
        DiffLine::Hunk(text("@@ -12,4 +12,5 @@ fn load() {")),
        DiffLine::Context {
          old: 12,
          new: 12,
          text: text("  let a = 1;")
        },
        DiffLine::Removed {
          old: 13,
          text: text("  let b = 2;")
        },
        DiffLine::Added {
          new: 13,
          text: text("  let b = 3;")
        },
        DiffLine::Added {
          new: 14,
          text: text("  let c = 4;")
        },
        DiffLine::Context {
          old: 14,
          new: 15,
          text: text("  done();")
        },
        DiffLine::Hunk(text("@@ -40,2 +41,2 @@ fn other() {")),
        // A tab is spaces, and the carriage return is dropped.
        DiffLine::Removed {
          old: 40,
          text: text("    old();")
        },
        DiffLine::Added {
          new: 41,
          text: text("    new();")
        },
        DiffLine::Context {
          old: 41,
          new: 42,
          text: text("}")
        },
        DiffLine::Note(text("No newline at end of file")),
      ]
    );
    assert_eq!(diff.last_line(), 42);
  }

  /// A file's own escape sequences never reach a terminal.
  #[test]
  fn control_characters_are_not_kept() {
    assert_eq!(clean("a\x1b[31mb\x07"), "a\u{fffd}[31mb\u{fffd}");
    assert_eq!(clean("\tx\ty"), "    x   y");
  }

  #[test]
  fn says_why_there_are_no_lines() {
    let binary = "diff --git a/logo.png b/logo.png\nindex 1..2 100644\n\
                  Binary files a/logo.png and b/logo.png differ\n";
    assert_eq!(
      parse(binary.as_bytes()).note.as_deref(),
      Some("Binary file")
    );
    let mode =
      "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n";
    assert_eq!(
      parse(mode.as_bytes()).note.as_deref(),
      Some("Only the file's mode changed")
    );
    assert_eq!(parse(b"").note.as_deref(), Some("No lines changed"));
  }

  /// A merge's diff marks each line once per parent.
  #[test]
  fn reads_a_combined_diff() {
    let combined = "\
diff --cc both.rs
index 1111111,2222222..0000000
--- a/both.rs
+++ b/both.rs
@@@ -1,3 -1,3 +1,7 @@@
  fn main() {
++<<<<<<< HEAD
 +  ours();
++=======
+   theirs();
++>>>>>>> other
  }
";
    let diff = parse(combined.as_bytes());
    let kinds: String = diff
      .lines
      .iter()
      .map(|line| match line {
        DiffLine::Hunk(_) => '@',
        DiffLine::Context { .. } => ' ',
        DiffLine::Added { .. } => '+',
        DiffLine::Removed { .. } => '-',
        DiffLine::Note(_) => '\\',
      })
      .collect();
    assert_eq!(kinds, "@ +++++ ");
    assert_eq!(
      diff.lines[2],
      DiffLine::Added {
        new: 2,
        text: "<<<<<<< HEAD".to_string()
      }
    );
  }

  #[test]
  fn a_new_file_is_all_added() {
    let diff = of_new_file(b"one\ntwo\n");
    assert_eq!(
      diff.lines,
      vec![
        DiffLine::Added {
          new: 1,
          text: "one".to_string()
        },
        DiffLine::Added {
          new: 2,
          text: "two".to_string()
        },
      ]
    );
    assert_eq!(of_new_file(b"").note.as_deref(), Some("Empty file"));
    assert_eq!(
      of_new_file(b"\x89PNG\0\0").note.as_deref(),
      Some("Binary file")
    );
  }
}
