//! `cargo xtask git changed|commits|added`: three questions sessions ask of
//! git, each one command with plain arguments (`docs/testing-and-ci.md`,
//! "Questions for git in one command").
//!
//! Each was a compound shell command: `git diff --name-only $(git merge-base
//! origin/main HEAD)` with `git ls-files --others` beside it, `git log …
//! | grep`, `git diff -U0 … | grep '^+'` with the line numbers worked out by
//! hand. A shell that checks the commands it runs refuses `$(git …)` and
//! pipes of git into other programs, and the line numbers were wrong as
//! often as not. Here each is one call, read the same way on every system.
//!
//! The names say the question: `changed` (which files), `commits` (which
//! commits touched a path), `added` (which lines were added that match).
//! They sit under `git` because what they read is git's, and they only
//! read: nothing here writes to the repository.

use std::path::Path;

use crate::Result;
use crate::branch::{Call, Git};
use crate::pattern::Pattern;

const USAGE: &str = "\
usage: cargo xtask git changed [--against REF]
       cargo xtask git commits PATH... [--since REF] [--max N]
       cargo xtask git added --grep RE [--against REF] [PATH...]

changed   Every file that differs from where this branch left REF
          (origin/main when not given; the merge base of REF and HEAD):
          committed, staged, changed in the working tree, or new and not
          ignored. A line each: its status (A, M, D, T, or ? for a new file
          git does not track), its path, and `(not committed)` when the
          change is not all in HEAD.
commits   The commits that touched any PATH, newest first: hash, date,
          subject. --since REF: only those after REF (REF..HEAD).
          --max N: at most N (30 when not given; 0 is all).
added     Each line added since where this branch left REF, committed or
          not, new files' lines included, that matches the regular
          expression RE (the syntax of `ci-log --grep`), as PATH:LINE: TEXT,
          LINE being its line in the file as it is now. PATHs limit it to
          those files or folders.";

/// `cargo xtask git …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let top = crate::workspace::root();
    if !top.join(".git").exists() {
        return Err("this tree has no .git: run it in a git checkout".to_owned());
    }
    let git = Git::at(&top);
    let text = match arguments.split_first() {
        Some((verb, rest)) if verb == "changed" => changed(&git, &Against::parse(rest)?)?,
        Some((verb, rest)) if verb == "commits" => commits(&git, &CommitsRequest::parse(rest)?)?,
        Some((verb, rest)) if verb == "added" => added(&git, &top, &AddedRequest::parse(rest)?)?,
        Some((verb, _)) if verb == "--help" || verb == "-h" => USAGE.to_owned(),
        Some((verb, _)) => return Err(format!("unknown question `{verb}`\n\n{USAGE}")),
        None => return Err(USAGE.to_owned()),
    };
    print!("{text}");
    Ok(())
}

/// `--against REF`, alone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Against(pub(crate) String);

impl Against {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        match arguments {
            [] => Ok(Self("origin/main".to_owned())),
            [flag, reference] if flag == "--against" && !reference.starts_with('-') => {
                Ok(Self(reference.clone()))
            }
            _ => Err(format!("`changed` takes only --against REF\n\n{USAGE}")),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitsRequest {
    pub(crate) paths: Vec<String>,
    pub(crate) since: Option<String>,
    pub(crate) max: usize,
}

impl CommitsRequest {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            paths: Vec::new(),
            since: None,
            max: 30,
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = || {
                words
                    .next()
                    .filter(|value| !value.starts_with('-'))
                    .ok_or_else(|| format!("`{word}` needs a value\n\n{USAGE}"))
            };
            match word.as_str() {
                "--since" => request.since = Some(value()?.clone()),
                "--max" => {
                    let text = value()?;
                    request.max = text
                        .parse()
                        .map_err(|_| format!("`--max {text}`: a number\n\n{USAGE}"))?;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag `{other}`\n\n{USAGE}"));
                }
                path => request.paths.push(path.to_owned()),
            }
        }
        if request.paths.is_empty() {
            return Err(format!("name the paths: `commits PATH...`\n\n{USAGE}"));
        }
        Ok(request)
    }
}

#[derive(Debug)]
pub(crate) struct AddedRequest {
    pub(crate) pattern: Pattern,
    pub(crate) against: String,
    pub(crate) paths: Vec<String>,
}

impl AddedRequest {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let (mut pattern, mut against, mut paths) = (None, None, Vec::new());
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = || {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs a value\n\n{USAGE}"))
            };
            match word.as_str() {
                "--grep" => pattern = Some(Pattern::new(value()?)?),
                "--against" => against = Some(value()?.clone()),
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag `{other}`\n\n{USAGE}"));
                }
                path => paths.push(path.to_owned()),
            }
        }
        Ok(Self {
            pattern: pattern
                .ok_or_else(|| format!("say what to look for: --grep RE\n\n{USAGE}"))?,
            against: against.unwrap_or_else(|| "origin/main".to_owned()),
            paths,
        })
    }
}

/// What git printed, as text, when it succeeded; bytes that are not text
/// are read as the replacement character.
fn read(git: &Git, arguments: &[&str]) -> Result<String> {
    let output = git.output(arguments, &Call::default())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "`git {}` failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Where HEAD left `against`: their merge base.
fn base(git: &Git, against: &str) -> Result<String> {
    git.run(&["merge-base", against, "HEAD"])
        .map_err(|error| format!("no merge base of `{against}` and HEAD: {error}"))
}

/// The paths of a `-z` listing.
fn paths_of(listed: &str) -> impl Iterator<Item = &str> {
    listed.split('\0').filter(|path| !path.is_empty())
}

/// `git diff --name-status -z` read: each file's status letter and path.
pub(crate) fn name_status(listed: &str) -> Vec<(String, String)> {
    let mut words = paths_of(listed);
    let mut found = Vec::new();
    while let (Some(status), Some(path)) = (words.next(), words.next()) {
        found.push((status.chars().take(1).collect(), path.to_owned()));
    }
    found
}

/// The lines of `changed`: from the merge base to the working tree, each
/// marked when not all of it is in HEAD, then each new file git does not
/// track.
pub(crate) fn changed_lines(
    since_base: &[(String, String)],
    not_committed: &[String],
    untracked: &[String],
) -> String {
    let mut text = String::new();
    for (status, path) in since_base {
        let mark = if not_committed.contains(path) {
            "   (not committed)"
        } else {
            ""
        };
        text.push_str(&format!("{status} {path}{mark}\n"));
    }
    for path in untracked {
        text.push_str(&format!("? {path}   (not committed)\n"));
    }
    if text.is_empty() {
        text.push_str("Nothing differs.\n");
    }
    text
}

const DIFF: [&str; 6] = [
    "-c",
    "core.quotePath=false",
    "diff",
    "--no-renames",
    "--no-ext-diff",
    "--no-color",
];

fn changed(git: &Git, against: &Against) -> Result<String> {
    let base = base(git, &against.0)?;
    let mut arguments = DIFF.to_vec();
    arguments.extend(["--name-status", "-z", base.as_str()]);
    let since_base = name_status(&read(git, &arguments)?);
    let mut arguments = DIFF.to_vec();
    arguments.extend(["--name-only", "-z", "HEAD"]);
    let not_committed: Vec<String> = paths_of(&read(git, &arguments)?)
        .map(str::to_owned)
        .collect();
    let untracked: Vec<String> = paths_of(&read(
        git,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?)
    .map(str::to_owned)
    .collect();
    Ok(format!(
        "Against {} (where HEAD left it, {}):\n{}",
        against.0,
        &base[..base.len().min(12)],
        changed_lines(&since_base, &not_committed, &untracked)
    ))
}

fn commits(git: &Git, request: &CommitsRequest) -> Result<String> {
    let max = format!("--max-count={}", request.max);
    let range = request.since.as_ref().map(|since| format!("{since}..HEAD"));
    let mut arguments = vec!["log", "--format=%h %ad %s", "--date=short"];
    if request.max > 0 {
        arguments.push(&max);
    }
    arguments.push(range.as_deref().unwrap_or("HEAD"));
    arguments.push("--");
    arguments.extend(request.paths.iter().map(String::as_str));
    let listed = read(git, &arguments)?;
    let count = listed.lines().count();
    let mut text = listed;
    if count == 0 {
        text.push_str("No commit touched them.\n");
    } else if request.max > 0 && count == request.max {
        text.push_str(&format!(
            "(the first {count}: --max N for more, --max 0 for all)\n"
        ));
    }
    Ok(text)
}

/// One added line: its file, its line in the file as it is now, its text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Added {
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// The lines a `git diff -U0` adds, each with its file and its line in the
/// new text: read from each hunk's `+start` and counted on.
pub(crate) fn added_lines(diff: &str) -> Vec<Added> {
    let mut found = Vec::new();
    let mut path: Option<String> = None;
    let mut line = 0;
    let mut in_hunk = false;
    for each in diff.lines() {
        if each.starts_with("diff --git ") {
            path = None;
            in_hunk = false;
        } else if !in_hunk && let Some(new) = each.strip_prefix("+++ ") {
            path = new.strip_prefix("b/").map(str::to_owned);
        } else if let Some(header) = each.strip_prefix("@@ ") {
            in_hunk = true;
            // `-a,b +c,d @@`: the new text's lines start at c.
            line = header
                .split_whitespace()
                .find_map(|word| word.strip_prefix('+'))
                .and_then(|range| range.split(',').next())
                .and_then(|start| start.parse().ok())
                .unwrap_or(0);
        } else if in_hunk && let Some(text) = each.strip_prefix('+') {
            if let Some(path) = &path {
                found.push(Added {
                    path: path.clone(),
                    line,
                    text: text.trim_end_matches('\r').to_owned(),
                });
            }
            line += 1;
        } else if in_hunk && each.starts_with(' ') {
            line += 1;
        }
    }
    found
}

fn added(git: &Git, top: &Path, request: &AddedRequest) -> Result<String> {
    let base = base(git, &request.against)?;
    let mut arguments = DIFF.to_vec();
    arguments.extend([
        "-U0",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        base.as_str(),
        "--",
    ]);
    arguments.extend(request.paths.iter().map(String::as_str));
    let mut lines = added_lines(&read(git, &arguments)?);
    // A new file git does not track is all added.
    let mut listing = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    listing.extend(request.paths.iter().map(String::as_str));
    for path in paths_of(&read(git, &listing)?) {
        let Ok(text) = std::fs::read_to_string(top.join(path)) else {
            continue;
        };
        lines.extend(text.lines().enumerate().map(|(at, each)| Added {
            path: path.to_owned(),
            line: at + 1,
            text: each.trim_end_matches('\r').to_owned(),
        }));
    }
    let mut text = String::new();
    let mut count = 0;
    for each in lines
        .iter()
        .filter(|each| request.pattern.is_match(&each.text))
    {
        text.push_str(&format!("{}:{}: {}\n", each.path, each.line, each.text));
        count += 1;
    }
    text.push_str(&format!(
        "{count} added line(s) match, against {} (where HEAD left it, {}).\n",
        request.against,
        &base[..base.len().min(12)]
    ));
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cineris_testkit::Scratch;
    use cineris_testkit::scratch::git;

    use super::{
        Added, AddedRequest, Against, CommitsRequest, added, added_lines, changed, changed_lines,
        commits, name_status,
    };
    use crate::branch::Git;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn each_question_reads_its_arguments_and_refuses_what_it_cannot_use() {
        assert_eq!(Against::parse(&[]).unwrap(), Against("origin/main".into()));
        assert_eq!(
            Against::parse(&strings(&["--against", "main"])).unwrap(),
            Against("main".into())
        );
        assert!(Against::parse(&strings(&["main"])).is_err());
        assert_eq!(
            CommitsRequest::parse(&strings(&["a.rs", "--since", "main", "b/", "--max", "0"]))
                .unwrap(),
            CommitsRequest {
                paths: strings(&["a.rs", "b/"]),
                since: Some("main".into()),
                max: 0
            }
        );
        for (words, said) in [
            (&[][..], "name the paths"),
            (&["a", "--max", "x"], "a number"),
            (&["a", "--since"], "needs a value"),
            (&["a", "--all"], "unknown flag"),
        ] {
            let error = CommitsRequest::parse(&strings(words)).unwrap_err();
            assert!(error.contains(said), "{words:?}: {error}");
        }
        let request = AddedRequest::parse(&strings(&["--grep", "todo", "src"])).unwrap();
        assert_eq!(
            (request.against.as_str(), request.paths.len()),
            ("origin/main", 1)
        );
        assert!(
            AddedRequest::parse(&strings(&["src"]))
                .unwrap_err()
                .contains("--grep RE")
        );
        assert!(AddedRequest::parse(&strings(&["--grep", "(?=x)"])).is_err());
    }

    #[test]
    fn an_added_line_is_numbered_as_the_file_has_it_now() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -3,0 +4,2 @@ fn a() {
+    let x = 1;\r
+    let y = 2;
@@ -10 +12 @@ fn b() {
-    old();
+    new();
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-gone
diff --git a/docs/new page.md b/docs/new page.md
new file mode 100644
--- /dev/null
+++ b/docs/new page.md
@@ -0,0 +1,2 @@
+# A page
++++ not a header
";
        let found = added_lines(diff);
        let at = |path: &str, line: usize, text: &str| Added {
            path: path.to_owned(),
            line,
            text: text.to_owned(),
        };
        assert_eq!(
            found,
            [
                at("src/a.rs", 4, "    let x = 1;"),
                at("src/a.rs", 5, "    let y = 2;"),
                at("src/a.rs", 12, "    new();"),
                at("docs/new page.md", 1, "# A page"),
                at("docs/new page.md", 2, "+++ not a header"),
            ]
        );
    }

    #[test]
    fn changed_files_are_listed_with_what_is_not_committed_marked() {
        let listed = "M\0src/a.rs\0A\0docs/b c.md\0D\0old.rs\0";
        let since = name_status(listed);
        assert_eq!(
            since,
            [
                ("M".to_owned(), "src/a.rs".to_owned()),
                ("A".to_owned(), "docs/b c.md".to_owned()),
                ("D".to_owned(), "old.rs".to_owned())
            ]
        );
        let text = changed_lines(&since, &strings(&["src/a.rs"]), &strings(&["new.rs"]));
        assert_eq!(
            text,
            "M src/a.rs   (not committed)\nA docs/b c.md\nD old.rs\n? new.rs   (not committed)\n"
        );
        assert_eq!(changed_lines(&[], &[], &[]), "Nothing differs.\n");
    }

    fn isolated(dir: &Path) -> Git {
        Git::with_env(
            dir,
            vec![
                ("GIT_CONFIG_NOSYSTEM", Some("1".into())),
                (
                    "GIT_CONFIG_GLOBAL",
                    Some(dir.join("no-such-git-config").into_os_string()),
                ),
            ],
        )
    }

    /// The three questions on one small repository: a branch off `main`
    /// with a commit, a change staged, one not, and a new file.
    #[test]
    fn sys_the_three_questions_read_a_branch_its_changes_and_its_new_files() {
        let scratch = Scratch::outside("xtask-git-read", "three");
        let repo = scratch.git_init("repo");
        scratch.file("repo/src/a.rs", "fn a() {}\n");
        scratch.file("repo/docs/page.md", "# A page\n");
        scratch.git_commit_all("repo", "Start");
        git(&repo, &["checkout", "--quiet", "-b", "work"]);
        scratch.file("repo/src/a.rs", "fn a() {}\n// TODO: one\n");
        scratch.git_commit_all("repo", "Add a note to a");
        scratch.file("repo/docs/page.md", "# A page\n\nTODO: two\n");
        scratch.file("repo/src/b.rs", "// TODO: three\n");
        let here = isolated(&repo);

        let text = changed(&here, &Against("main".into())).unwrap();
        assert!(
            text.contains("M docs/page.md   (not committed)\n"),
            "{text}"
        );
        assert!(text.contains("M src/a.rs\n"), "{text}");
        assert!(text.contains("? src/b.rs   (not committed)\n"), "{text}");

        let text = commits(
            &here,
            &CommitsRequest {
                paths: vec!["src".into()],
                since: Some("main".into()),
                max: 30,
            },
        )
        .unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("Add a note to a"), "{text}");

        let request = AddedRequest::parse(&[
            "--grep".to_owned(),
            "TODO".to_owned(),
            "--against".to_owned(),
            "main".to_owned(),
        ])
        .unwrap();
        let text = added(&here, &repo, &request).unwrap();
        for said in [
            "docs/page.md:3: TODO: two\n",
            "src/a.rs:2: // TODO: one\n",
            "src/b.rs:1: // TODO: three\n",
            "3 added line(s) match, against main",
        ] {
            assert!(text.contains(said), "{said}\n{text}");
        }
    }
}
