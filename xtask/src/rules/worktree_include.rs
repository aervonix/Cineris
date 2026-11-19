//! `.worktreeinclude` names only files git ignores (since the twelfth
//! tooling step).
//!
//! An agent tool reads `.worktreeinclude` to copy files git does not carry
//! (a maintainer's private notes) into each new worktree. A tracked file
//! named there is copied over the checked-out one, and a file that is
//! neither tracked nor ignored would show as new in every worktree, one
//! `git add` away from a commit. So every entry must be ignored by the
//! root's `.gitignore`, and be no file or folder git tracks.
//!
//! The patterns are read as git reads them, for the forms the root's
//! `.gitignore` uses: a leading `/` anchors a pattern, a trailing `/`
//! matches only a folder (and so everything under it: an entry under it, or
//! an entry that ends in `/`), `*` and `?` match within one part of a path,
//! and a `!` line takes a path back out. A pattern with no `/` matches a
//! name at any depth. A `.gitignore` deeper in the tree is not read. An
//! entry with `*` or `?` in it is checked against every tracked file it
//! matches. In a copy of the tree with no repository the rule is not run
//! (`rules::check_docs`): nothing there says what git tracks.

use super::Violation;

/// The file the rule checks.
pub(crate) const FILE: &str = ".worktreeinclude";

/// The ignore file read.
pub(crate) const IGNORE: &str = ".gitignore";

/// Checks each entry of `include` against `ignore` and the tracked `files`.
pub(crate) fn check(include: &str, ignore: &str, files: &[String]) -> Vec<Violation> {
    let patterns = patterns(ignore);
    let mut violations = Vec::new();
    for (index, line) in include.lines().enumerate() {
        let entry = line.trim_end();
        if entry.trim_start().is_empty() || entry.starts_with('#') {
            continue;
        }
        let folder = entry.ends_with('/');
        let path = entry.trim_start_matches('/').trim_end_matches('/');
        let glob_entry = path.contains(['*', '?']);
        let tracked = files.iter().any(|file| {
            if glob_entry {
                matches(entry, file, false)
            } else {
                file == path || file.starts_with(&format!("{path}/"))
            }
        });
        let problem = if tracked {
            Some(format!(
                "`{entry}` is a file git tracks: a worktree would get the copy over the \
                 checked-out one. List only files `{IGNORE}` ignores."
            ))
        } else if !ignored(path, folder, &patterns) {
            Some(format!(
                "`{entry}` is not ignored by `{IGNORE}`: every worktree would show it as a new \
                 file, one `git add` from a commit. Add it to `{IGNORE}`, or take it out of \
                 `{FILE}`."
            ))
        } else {
            None
        };
        if let Some(problem) = problem {
            violations.push(Violation::new(FILE, Some(index + 1), problem));
        }
    }
    violations
}

/// The patterns of an ignore file's text.
fn patterns(ignore: &str) -> Vec<&str> {
    ignore
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim_start().is_empty() && !line.starts_with('#'))
        .collect()
}

/// Whether a file of the repository, `path`, is ignored by the ignore file
/// `ignore`: for a copy of the tree with no repository, whose walk finds
/// the files git would not have listed.
pub(crate) fn ignored_by(path: &str, ignore: &str) -> bool {
    ignored(path, false, &patterns(ignore))
}

/// Whether `path` (relative, no leading `/`; a folder when `folder`) is
/// ignored by `patterns`, in order, the last that matches deciding.
fn ignored(path: &str, folder: bool, patterns: &[&str]) -> bool {
    let mut ignored = false;
    for pattern in patterns {
        let (negated, pattern) = match pattern.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, *pattern),
        };
        if matches(pattern, path, folder) {
            ignored = !negated;
        }
    }
    ignored
}

/// Whether one pattern matches `path` (a folder when `folder`) or a folder
/// above it.
fn matches(pattern: &str, path: &str, folder: bool) -> bool {
    // A trailing `/` matches a folder only: one above the path, or the
    // path itself when it is one.
    let folder_only = pattern.ends_with('/');
    let pattern = pattern.trim_end_matches('/');
    let anchored = pattern.contains('/');
    let pattern = pattern.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    let wanted: Vec<&str> = pattern.split('/').collect();
    // Each prefix of the path is the path itself or a folder above it.
    for length in 1..=parts.len() {
        if folder_only && length == parts.len() && !folder {
            break;
        }
        let prefix = &parts[..length];
        let hit = if anchored {
            prefix.len() == wanted.len() && prefix.iter().zip(&wanted).all(|(p, w)| glob(w, p))
        } else {
            glob(wanted[0], prefix[length - 1])
        };
        if hit {
            return true;
        }
    }
    false
}

/// `*` and `?` within one part of a path.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(at) = star {
            pi = at + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::check;

    const IGNORE: &str = "# build\n/target/\n/CLAUDE.local.md\n*.log\n!keep.log\n";

    /// An ignored file passes; a tracked one and one git would show as new
    /// each fail, saying which.
    #[test]
    fn only_files_git_ignores_may_be_listed() {
        let files = vec!["AGENTS.md".to_owned(), "docs/a.md".to_owned()];
        assert!(check("CLAUDE.local.md\n# note\n\n", IGNORE, &files).is_empty());
        assert!(check("target/notes.txt\nsub/x.log\n", IGNORE, &files).is_empty());

        let found = check("AGENTS.md\ndocs\n.env\nkeep.log\n", IGNORE, &files);
        let lines: Vec<_> = found.iter().map(|v| (v.line, v.message.clone())).collect();
        assert_eq!(found.len(), 4, "{lines:?}");
        assert!(found[0].message.contains("git tracks"));
        assert!(found[1].message.contains("git tracks"));
        assert!(found[2].message.contains("not ignored"));
        assert!(
            found[3].message.contains("not ignored"),
            "a `!` line takes it back"
        );
    }

    /// A folder-only pattern ignores what is under the folder, and the
    /// folder as an entry that ends in `/`, but not a file of that name.
    #[test]
    fn a_folder_pattern_does_not_ignore_a_file_of_its_name() {
        let ignore = "/notes/\n";
        assert!(check("notes/a.md\nnotes/\n", ignore, &[]).is_empty());
        let found = check("notes\n", ignore, &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("not ignored"));
    }

    /// An entry with a wildcard is held to every tracked file it matches.
    #[test]
    fn a_wildcard_entry_may_match_no_tracked_file() {
        let ignore = "*.local.md\n";
        assert!(check("*.local.md\n", ignore, &["docs/a.md".to_owned()]).is_empty());
        let found = check("*.local.md\n", ignore, &["docs/zz.local.md".to_owned()]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("git tracks"));
    }

    /// For a copy of the tree with no repository: what the ignore file
    /// ignores.
    #[test]
    fn a_file_the_ignore_file_names_is_ignored() {
        assert!(super::ignored_by("CLAUDE.local.md", IGNORE));
        assert!(super::ignored_by("target/x/y.md", IGNORE));
        assert!(!super::ignored_by("docs/a.md", IGNORE));
    }

    /// An anchored pattern matches from the root only.
    #[test]
    fn an_anchored_pattern_matches_only_at_the_root() {
        assert_eq!(check("sub/CLAUDE.local.md\n", IGNORE, &[]).len(), 1);
    }
}
