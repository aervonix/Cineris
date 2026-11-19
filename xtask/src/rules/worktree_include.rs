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
//! matches a folder (and so everything under it), `*` and `?` match within
//! one part of a path, and a `!` line takes a path back out. A pattern with
//! no `/` matches a name at any depth. A `.gitignore` deeper in the tree is
//! not read.

use super::Violation;

/// The file the rule checks.
pub(crate) const FILE: &str = ".worktreeinclude";

/// The ignore file read.
pub(crate) const IGNORE: &str = ".gitignore";

/// Checks each entry of `include` against `ignore` and the tracked `files`.
pub(crate) fn check(include: &str, ignore: &str, files: &[String]) -> Vec<Violation> {
    let patterns: Vec<&str> = ignore
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut violations = Vec::new();
    for (index, line) in include.lines().enumerate() {
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        let path = entry.trim_start_matches('/').trim_end_matches('/');
        let tracked = files
            .iter()
            .any(|file| file == path || file.starts_with(&format!("{path}/")));
        let problem = if tracked {
            Some(format!(
                "`{entry}` is a file git tracks: a worktree would get the copy over the \
                 checked-out one. List only files `{IGNORE}` ignores."
            ))
        } else if !ignored(path, &patterns) {
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

/// Whether `path` (relative, no leading `/`) is ignored by `patterns`, in
/// order, the last that matches deciding.
fn ignored(path: &str, patterns: &[&str]) -> bool {
    let mut ignored = false;
    for pattern in patterns {
        let (negated, pattern) = match pattern.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, *pattern),
        };
        if matches(pattern, path) {
            ignored = !negated;
        }
    }
    ignored
}

/// Whether one pattern matches `path` or a folder above it.
fn matches(pattern: &str, path: &str) -> bool {
    // A trailing `/` (a folder only) is not told apart: an entry is a file
    // or a folder, which the text does not say, and a folder above it
    // matches either way.
    let pattern = pattern.trim_end_matches('/');
    let anchored = pattern.contains('/');
    let pattern = pattern.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    let wanted: Vec<&str> = pattern.split('/').collect();
    // Each prefix of the path is the path itself or a folder above it.
    for length in 1..=parts.len() {
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

    /// An anchored pattern matches from the root only.
    #[test]
    fn an_anchored_pattern_matches_only_at_the_root() {
        assert_eq!(check("sub/CLAUDE.local.md\n", IGNORE, &[]).len(), 1);
    }
}
