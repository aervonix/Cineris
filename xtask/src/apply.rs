//! `cargo xtask apply SCRIPT`: the changes of a script and the checks of
//! them, in one command (`docs/shell-guard.md`, "One command for an edit
//! and its check").
//!
//! In a Claude Code worktree session the shell guard refuses a heredoc
//! beside git or an interpreter, a `&&` chain, `xargs` into git and more,
//! so every edit of wave 11's sessions became a script run in a call of its
//! own, and its check another call. This takes both: the changes, in the
//! form of `replace --script` with two more (a whole file written, a file
//! deleted), and the checks to run on them (formatting, the repo rules, a
//! crate's build, lints or tests). It is one command with one argument,
//! which the guard accepts.
//!
//! **All or none.** Every change is made in memory first: if one cannot be
//! made, no file is written and no check runs. If a check fails, every file
//! the script touched is put back as it was before it (a file it made is
//! removed, one it deleted comes back), unless `--keep` says to leave the
//! changes to be mended.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::replace;

const USAGE: &str = "\
usage: cargo xtask apply SCRIPT [--dry-run] [--keep]

Makes every change of SCRIPT in order, then runs its checks in order. All or
none: if a change cannot be made, nothing is written and no check runs; if a
check fails, every file the script touched is put back as it was (a file it
made is removed, one it deleted comes back) and the command fails, naming the
check. --keep leaves the changes in place when a check fails, to be mended.
--dry-run says whether every change can be made, and which checks would run,
and writes and runs nothing.

A change is an edit of `replace --script` (see `cargo xtask replace --script
--help`):

    @@ file: PATH
    @@ in: ITEM  /  @@ occurrence: N  /  @@ text      (optional)
    @@ from
    the code or text to find
    @@ to
    what to put there
    @@ end

a whole file, made or replaced, its lines as they are:

    @@ write: PATH
    the file's lines
    @@ end

or a file deleted, on one line: `@@ delete: PATH`.

A check is one line outside a change, and they run in the order written:

    @@ check: fmt                    rustfmt on the .rs files the script touched
    @@ check: rules                  cargo xtask rules
    @@ check: links                  cargo xtask links
    @@ check: check STEP...          cargo xtask check of those steps (`breaks`,
                                     `generated`, ...)
    @@ check: build -p CRATE...      cargo check --all-targets of the crates
    @@ check: clippy -p CRATE...     clippy of the crates' targets, warnings denied
    @@ check: test ARGUMENT...       cargo xtask test with these arguments, such as
                                     test -p cortex-tui -E 'test(/dialog/)'

A check's words are split at spaces; quote a word that holds one ('…' or \"…\").
Lines outside a change that begin with `#`, and empty ones, are comments.
Paths are relative to the directory the command runs in.

At the desk, run it on the laptop, or as `rx --back -- cargo xtask apply
SCRIPT` so that what it wrote on the desktop comes back.";

/// One check of a script.
#[derive(Debug, Clone, PartialEq)]
enum Check {
    /// rustfmt on the `.rs` files the script touched.
    Fmt,
    /// `cargo xtask rules`.
    Rules,
    /// `cargo xtask links`.
    Links,
    /// `cargo xtask check` of these steps.
    Steps(Vec<String>),
    /// `cargo check --all-targets` with these arguments.
    Build(Vec<String>),
    /// clippy with these arguments, warnings denied.
    Clippy(Vec<String>),
    /// `cargo xtask test` with these arguments.
    Test(Vec<String>),
}

impl Check {
    /// How the check is shown.
    fn shown(&self) -> String {
        let with = |name: &str, words: &[String]| {
            if words.is_empty() {
                name.to_owned()
            } else {
                format!("{name} {}", words.join(" "))
            }
        };
        match self {
            Check::Fmt => "fmt".to_owned(),
            Check::Rules => "rules".to_owned(),
            Check::Links => "links".to_owned(),
            Check::Steps(words) => with("check", words),
            Check::Build(words) => with("build", words),
            Check::Clippy(words) => with("clippy", words),
            Check::Test(words) => with("test", words),
        }
    }
}

/// One change, in the order the script gives it.
#[derive(Debug, PartialEq)]
enum Change {
    /// The edit of `replace --script` whose number (from 1, among the
    /// script's edits) this is.
    Edit(usize),
    /// A whole file.
    Write { path: PathBuf, text: String },
    /// A file deleted.
    Delete(PathBuf),
}

/// A script, read.
#[derive(Debug, PartialEq)]
struct Script {
    changes: Vec<(usize, Change)>,
    checks: Vec<(usize, Check)>,
    /// The script's text with every line that is not of an edit made
    /// empty, for `replace`'s reader, whose line numbers are then the
    /// script's own.
    edits: String,
}

/// The words of a check, split at spaces, with `'…'` and `"…"` keeping a
/// word together.
fn words(line: &str) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for character in line.chars() {
        match (quote, character) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(character);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    found.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if let Some(open) = quote {
        return Err(format!("a quote ({open}) is not closed"));
    }
    if started {
        found.push(word);
    }
    Ok(found)
}

/// Reads a check's line, the words after `@@ check:`.
fn check(text: &str) -> Result<Check> {
    let mut words = words(text)?;
    if words.is_empty() {
        return Err("`@@ check:` names no check".to_owned());
    }
    let name = words.remove(0);
    let needs_crates = |words: &[String], what: &str| {
        if words.is_empty() {
            Err(format!(
                "`{what}` needs the crates: `{what} -p CRATE` (the whole workspace is `check \
                 clippy`, or `test` alone)"
            ))
        } else {
            Ok(())
        }
    };
    let no_words = |words: &[String], what: &str| {
        if words.is_empty() {
            Ok(())
        } else {
            Err(format!("`{what}` takes no words: `{}`", words.join(" ")))
        }
    };
    match name.as_str() {
        "fmt" => no_words(&words, "fmt").map(|()| Check::Fmt),
        "rules" => no_words(&words, "rules").map(|()| Check::Rules),
        "links" => no_words(&words, "links").map(|()| Check::Links),
        "check" => Ok(Check::Steps(words)),
        "build" => needs_crates(&words, "build").map(|()| Check::Build(words)),
        "clippy" => needs_crates(&words, "clippy").map(|()| Check::Clippy(words)),
        "test" => Ok(Check::Test(words)),
        other => Err(format!(
            "`{other}` is not a check: fmt, rules, links, check STEP..., build -p CRATE, \
             clippy -p CRATE, test ARGUMENT..."
        )),
    }
}

/// Reads a script ([`USAGE`]).
fn parse(text: &str) -> Result<Script> {
    let mut changes = Vec::new();
    let mut checks = Vec::new();
    let mut edits = Vec::new();
    let mut edit_count = 0;
    // Inside `replace`'s edit, or inside a `write:`, from its line.
    let mut in_edit = false;
    let mut writing: Option<(usize, PathBuf, Vec<&str>)> = None;
    for (index, line) in text.lines().enumerate() {
        let at = index + 1;
        let fail = |what: &str| Err(format!("the script's line {at}: {what}\n\n{USAGE}"));
        let marker = line.strip_prefix("@@ ").map(str::trim_end);
        if let Some((_, _, lines)) = writing.as_mut() {
            if marker == Some("end") {
                let (from, path, lines) = writing.take().unwrap_or_default();
                let mut body = lines.join("\n");
                if !lines.is_empty() {
                    body.push('\n');
                }
                changes.push((from, Change::Write { path, text: body }));
            } else {
                lines.push(line);
            }
            edits.push("");
            continue;
        }
        if in_edit {
            if marker == Some("end") {
                in_edit = false;
            }
            edits.push(line);
            continue;
        }
        match marker {
            Some(marker) if marker.starts_with("file:") => {
                edit_count += 1;
                changes.push((at, Change::Edit(edit_count)));
                in_edit = true;
                edits.push(line);
            }
            Some(marker) if marker.starts_with("write:") => {
                let path = marker["write:".len()..].trim();
                if path.is_empty() {
                    return fail("`@@ write:` names no file");
                }
                writing = Some((at, PathBuf::from(path), Vec::new()));
                edits.push("");
            }
            Some(marker) if marker.starts_with("delete:") => {
                let path = marker["delete:".len()..].trim();
                if path.is_empty() {
                    return fail("`@@ delete:` names no file");
                }
                changes.push((at, Change::Delete(PathBuf::from(path))));
                edits.push("");
            }
            Some(marker) if marker.starts_with("check:") => {
                let found = check(&marker["check:".len()..])
                    .map_err(|why| format!("the script's line {at}: {why}\n\n{USAGE}"))?;
                checks.push((at, found));
                edits.push("");
            }
            Some(other) => {
                return fail(&format!(
                    "`@@ {other}`: a change begins with `@@ file:`, `@@ write:` or `@@ delete:`, \
                     and a check is `@@ check: …`"
                ));
            }
            None if line.trim().is_empty() || line.starts_with('#') => edits.push(""),
            None => return fail("text outside a change: begin one with `@@ file:` or `@@ write:`"),
        }
    }
    if let Some((from, _, _)) = writing {
        return Err(format!(
            "the script's `@@ write:` of line {from} has no `@@ end`\n\n{USAGE}"
        ));
    }
    if changes.is_empty() {
        return Err(format!("the script makes no change\n\n{USAGE}"));
    }
    Ok(Script {
        changes,
        checks,
        edits: edits.join("\n"),
    })
}

/// What the files will be: each path the script touches, with its text
/// before (`None`: it was not there) and after (`None`: deleted), and its
/// permissions before, in the order the script first touches them.
type Plan = Vec<(
    PathBuf,
    Option<String>,
    Option<String>,
    Option<fs::Permissions>,
)>;

/// A file's text and permissions as it is now: `None` when it is not
/// there. A file that is there and cannot be read is an error, never "not
/// there", or putting it back would delete it.
type Read = Result<(Option<String>, Option<fs::Permissions>)>;

/// `path` with its `.` left out and each `..` taken back, so that one file
/// spelt two ways (`a.md`, `./a.md`, `d/../a.md`) is one file of the plan.
fn same_file(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut kept = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if kept
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_))) =>
            {
                kept.pop();
            }
            other => kept.push(other),
        }
    }
    kept
}

/// Makes every change in memory, in order, with `read` giving a file as it
/// is now. The first change that cannot be made, or file that cannot be
/// read, is named, and nothing is planned. Also gives what to print.
fn plan(
    script: &Script,
    edits: &[replace::ScriptEdit],
    mut read: impl FnMut(&Path) -> Read,
) -> Result<(Plan, String)> {
    let mut files: Plan = Vec::new();
    let mut shown = String::new();
    let index_of =
        |files: &mut Plan, path: &Path, read: &mut dyn FnMut(&Path) -> Read| -> Result<usize> {
            let path = same_file(path);
            if let Some(index) = files.iter().position(|(known, ..)| *known == path) {
                return Ok(index);
            }
            let (before, permissions) = read(&path).map_err(|error| {
                format!("{error}: a file that cannot be read is not changed. Nothing was written.")
            })?;
            files.push((path, before.clone(), before, permissions));
            Ok(files.len() - 1)
        };
    for (line, change) in &script.changes {
        match change {
            Change::Edit(number) => {
                let edit = &edits[number - 1];
                let path = &edit.request.file;
                let index = index_of(&mut files, path, &mut read)?;
                let Some(current) = files[index].2.as_deref() else {
                    return Err(format!(
                        "the script's line {line}: {} is not there (deleted by the script, or \
                         never made). Nothing was written.",
                        path.display()
                    ));
                };
                let (edited, lines) =
                    replace::edit(current, &edit.request, &path.display().to_string()).map_err(
                        |error| {
                            format!(
                                "the script's line {line}: {error} Nothing was written, and no \
                                 check ran."
                            )
                        },
                    )?;
                files[index].2 = Some(edited);
                shown.push_str(&lines);
            }
            Change::Write { path, text } => {
                let index = index_of(&mut files, path, &mut read)?;
                shown.push_str(&format!(
                    "{}: {} ({} line(s))\n",
                    path.display(),
                    if files[index].2.is_some() {
                        "written over"
                    } else {
                        "made"
                    },
                    text.lines().count()
                ));
                files[index].2 = Some(text.clone());
            }
            Change::Delete(path) => {
                let index = index_of(&mut files, path, &mut read)?;
                if files[index].2.is_none() {
                    return Err(format!(
                        "the script's line {line}: {} is not there to delete. Nothing was \
                         written.",
                        path.display()
                    ));
                }
                files[index].2 = None;
                shown.push_str(&format!("{}: deleted\n", path.display()));
            }
        }
    }
    Ok((files, shown))
}

/// Puts each file of `plan` in its state `after` (true) or `before`
/// (false). Says which it could not.
fn put(plan: &Plan, after: bool) -> Result<()> {
    let mut failed = Vec::new();
    for (path, before, made, permissions) in plan {
        let want = if after { made } else { before };
        let done = match want {
            Some(text) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    let _ = fs::create_dir_all(parent);
                }
                // Beside it first and then moved over it, so that a file is
                // never seen half written.
                let mut name = path.file_name().unwrap_or_default().to_os_string();
                name.push(".apply-new");
                let beside = path.with_file_name(name);
                // The file keeps its permissions, and one the script deleted
                // comes back with the ones it had (an executable hook stays
                // one).
                fs::write(&beside, text).and_then(|()| {
                    let kept = fs::metadata(path)
                        .ok()
                        .map(|metadata| metadata.permissions())
                        .or_else(|| permissions.clone());
                    if let Some(kept) = kept {
                        fs::set_permissions(&beside, kept)?;
                    }
                    fs::rename(&beside, path)
                })
            }
            None if path.exists() => fs::remove_file(path),
            None => Ok(()),
        };
        if let Err(error) = done {
            failed.push(format!("{}: {error}", path.display()));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// Runs one check. `touched` are the files the script left.
fn run_check(check: &Check, touched: &[PathBuf]) -> Result<()> {
    match check {
        Check::Fmt => {
            let rust: Vec<&PathBuf> = touched
                .iter()
                .filter(|path| path.extension().is_some_and(|ext| ext == "rs") && path.exists())
                .collect();
            if rust.is_empty() {
                println!("(no Rust file touched: nothing to format)");
                return Ok(());
            }
            // rustfmt finds the repository's rustfmt.toml from each file.
            let status = std::process::Command::new("rustfmt")
                .args(["--edition", "2024"])
                .args(&rust)
                .status()
                .map_err(|error| format!("could not run rustfmt: {error}"))?;
            if status.success() {
                Ok(())
            } else {
                Err("rustfmt failed".to_owned())
            }
        }
        Check::Rules => crate::checks::rules(),
        Check::Links => crate::checks::links(),
        Check::Steps(steps) => crate::checks::check(steps),
        Check::Build(words) => {
            let mut arguments = vec!["check".to_owned(), "--all-targets".to_owned()];
            arguments.extend(words.iter().cloned());
            crate::steps::cargo(&arguments)
        }
        Check::Clippy(words) => {
            let mut arguments = vec!["clippy".to_owned(), "--all-targets".to_owned()];
            arguments.extend(words.iter().cloned());
            arguments.extend(["--".to_owned(), "-D".to_owned(), "warnings".to_owned()]);
            crate::steps::cargo(&arguments)
        }
        Check::Test(words) => crate::checks::test(words),
    }
}

/// `cargo xtask apply SCRIPT [--dry-run] [--keep]`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    run_with(arguments, &mut |check, touched| run_check(check, touched))
}

/// [`run`], with the checks run by `checking`, which the tests replace.
fn run_with(
    arguments: &[String],
    checking: &mut dyn FnMut(&Check, &[PathBuf]) -> Result<()>,
) -> Result<()> {
    let (mut path, mut dry_run, mut keep) = (None, false, false);
    for argument in arguments {
        match argument.as_str() {
            "--dry-run" => dry_run = true,
            "--keep" => keep = true,
            "--help" | "-h" => return Err(USAGE.to_owned()),
            flag if flag.starts_with('-') => {
                return Err(format!("unknown flag `{flag}`\n\n{USAGE}"));
            }
            script if path.is_none() => path = Some(script.to_owned()),
            extra => return Err(format!("one script at a time: `{extra}`\n\n{USAGE}")),
        }
    }
    let path = path.ok_or_else(|| USAGE.to_owned())?;
    let text =
        fs::read_to_string(&path).map_err(|error| format!("could not read {path}: {error}"))?;
    let script = parse(&text)?;
    let edits = if script
        .changes
        .iter()
        .any(|(_, c)| matches!(c, Change::Edit(_)))
    {
        replace::parse_script(&script.edits)?
    } else {
        Vec::new()
    };
    let (plan, shown) = plan(&script, &edits, |path| match fs::read_to_string(path) {
        Ok(text) => Ok((
            Some(text),
            fs::metadata(path)
                .ok()
                .map(|metadata| metadata.permissions()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((None, None)),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    })?;
    print!("{shown}");
    if dry_run {
        println!(
            "(a dry run: every change can be made, to {} file(s); nothing was written)",
            plan.len()
        );
        for (_, check) in &script.checks {
            println!("would check: {}", check.shown());
        }
        return Ok(());
    }
    if let Err(error) = put(&plan, true) {
        // A file could not be written: the rest go back.
        let back = put(&plan, false);
        return Err(format!(
            "could not write every file ({error}). {}",
            match back {
                Ok(()) => "Every file was put back as it was.".to_owned(),
                Err(left) => format!("These could not be put back: {left}"),
            }
        ));
    }
    println!(
        "{} change(s) made, to {} file(s).",
        script.changes.len(),
        plan.len()
    );
    let touched: Vec<PathBuf> = plan.iter().map(|(path, ..)| path.clone()).collect();
    for (line, check) in &script.checks {
        println!(
            "\n== check: {} (the script's line {line}) ==",
            check.shown()
        );
        if let Err(error) = checking(check, &touched) {
            if keep {
                return Err(format!(
                    "the check `{}` failed: {error}. The changes are left in place (--keep).",
                    check.shown()
                ));
            }
            return match put(&plan, false) {
                Ok(()) => Err(format!(
                    "the check `{}` failed: {error}. Every file the script touched was put \
                     back as it was ({}).",
                    check.shown(),
                    touched
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Err(left) => Err(format!(
                    "the check `{}` failed: {error}. These could not be put back: {left}",
                    check.shown()
                )),
            };
        }
    }
    if !script.checks.is_empty() {
        println!("\nEvery check passed: the changes stay.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_checks_words_are_split_at_spaces_and_kept_together_by_quotes() {
        assert_eq!(
            words(" test -p cortex-tui -E 'test(/a b/)' \"x y\" ").unwrap(),
            strings(&["test", "-p", "cortex-tui", "-E", "test(/a b/)", "x y"])
        );
        assert_eq!(words("a '' b").unwrap(), strings(&["a", "", "b"]));
        assert!(words("test 'open").is_err());
    }

    #[test]
    fn every_check_is_read_and_a_wrong_one_is_refused() {
        assert_eq!(check("fmt").unwrap(), Check::Fmt);
        assert_eq!(check("rules").unwrap(), Check::Rules);
        assert_eq!(check("links").unwrap(), Check::Links);
        assert_eq!(
            check("check breaks generated").unwrap(),
            Check::Steps(strings(&["breaks", "generated"]))
        );
        assert_eq!(
            check("build -p xtask").unwrap(),
            Check::Build(strings(&["-p", "xtask"]))
        );
        assert_eq!(
            check("clippy -p xtask").unwrap(),
            Check::Clippy(strings(&["-p", "xtask"]))
        );
        assert_eq!(
            check("test -p xtask -E 'test(/apply::/)'").unwrap(),
            Check::Test(strings(&["-p", "xtask", "-E", "test(/apply::/)"]))
        );
        assert!(check("").is_err());
        assert!(check("fmt now").is_err());
        assert!(check("build").is_err());
        assert!(check("clippy").is_err());
        assert!(check("deploy").is_err());
    }

    const SCRIPT: &str = "\
# A comment.
@@ file: a.rs
@@ from
fn a() {}
@@ to
fn a() { b(); }
@@ end

@@ write: new.md
# New

text
@@ end
@@ delete: old.md
@@ check: fmt
@@ check: test -p xtask
";

    #[test]
    fn a_script_is_read_in_order_with_the_edits_lines_where_they_were() {
        let script = parse(SCRIPT).unwrap();
        assert_eq!(
            script.changes,
            vec![
                (2, Change::Edit(1)),
                (
                    9,
                    Change::Write {
                        path: PathBuf::from("new.md"),
                        text: "# New\n\ntext\n".to_owned()
                    }
                ),
                (14, Change::Delete(PathBuf::from("old.md"))),
            ]
        );
        assert_eq!(
            script.checks,
            vec![
                (15, Check::Fmt),
                (16, Check::Test(strings(&["-p", "xtask"])))
            ]
        );
        // `replace` reads the edit at the script's own line.
        let edits = replace::parse_script(&script.edits).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].line, 2);
        // A line of `write:` that looks like a comment or a marker is the
        // file's.
        let script = parse("@@ write: f\n@@ check: fmt\n# x\n@@ end\n").unwrap();
        assert_eq!(
            script.changes[0].1,
            Change::Write {
                path: PathBuf::from("f"),
                text: "@@ check: fmt\n# x\n".to_owned()
            }
        );
        assert!(script.checks.is_empty());
    }

    #[test]
    fn a_script_that_is_not_whole_is_refused() {
        for (text, says) in [
            ("@@ check: fmt\n", "makes no change"),
            ("@@ write: f\nx\n", "has no `@@ end`"),
            ("@@ write:\nx\n@@ end\n", "names no file"),
            ("@@ delete:\n", "names no file"),
            ("stray\n", "text outside a change"),
            ("@@ move: a b\n", "`@@ move: a b`"),
            ("@@ delete: f\n@@ check: nothing\n", "is not a check"),
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }

    fn files(plan: &Plan) -> Vec<(String, Option<String>, Option<String>)> {
        plan.iter()
            .map(|(path, before, after, _)| {
                (path.display().to_string(), before.clone(), after.clone())
            })
            .collect()
    }

    #[test]
    fn the_changes_are_planned_in_order_each_on_what_the_one_before_left() {
        let script = parse(
            "@@ write: a.md\none\n@@ end\n@@ file: a.md\n@@ from\none\n@@ to\ntwo\n@@ end\n\
             @@ delete: b.md\n",
        )
        .unwrap();
        let edits = replace::parse_script(&script.edits).unwrap();
        let (plan, _) = plan(&script, &edits, |path| {
            Ok(((path == Path::new("b.md")).then(|| "b\n".to_owned()), None))
        })
        .unwrap();
        assert_eq!(
            files(&plan),
            vec![
                ("a.md".to_owned(), None, Some("two\n".to_owned())),
                ("b.md".to_owned(), Some("b\n".to_owned()), None),
            ]
        );
    }

    #[test]
    fn a_change_that_cannot_be_made_plans_nothing() {
        let script =
            parse("@@ file: a.md\n@@ from\nmissing\n@@ to\nx\n@@ end\n@@ delete: a.md\n").unwrap();
        let edits = replace::parse_script(&script.edits).unwrap();
        let error = plan(&script, &edits, |_| Ok((Some("text\n".to_owned()), None))).unwrap_err();
        assert!(error.contains("line 1"), "{error}");
        assert!(error.contains("Nothing was written"), "{error}");
        // A file deleted twice, and an edit of one not there.
        let script = parse("@@ delete: a\n@@ delete: a\n").unwrap();
        assert!(plan(&script, &[], |_| Ok((Some(String::new()), None))).is_err());
        let script = parse("@@ file: a.md\n@@ from\nx\n@@ to\ny\n@@ end\n").unwrap();
        let edits = replace::parse_script(&script.edits).unwrap();
        assert!(
            plan(&script, &edits, |_| Ok((None, None)))
                .unwrap_err()
                .contains("is not there")
        );
    }

    /// A folder of the test's own, which goes with it.
    fn scratch(name: &str) -> cortex_testkit::Scratch {
        cortex_testkit::Scratch::outside("xtask-apply", name)
    }

    /// Runs `script` in `dir`, every check answered by `answer`; gives the
    /// result and the checks asked, with the files each was given.
    fn apply(
        dir: &Path,
        script: &str,
        flags: &[&str],
        answer: impl Fn(&Check) -> Result<()>,
    ) -> (Result<()>, Vec<String>) {
        let file = dir.join("script.apply");
        // Every path of the script is made relative to `dir`.
        let script = script.replace("DIR/", &format!("{}/", dir.display()));
        fs::write(&file, script).unwrap();
        let mut arguments = vec![file.display().to_string()];
        arguments.extend(flags.iter().map(|flag| (*flag).to_owned()));
        let mut asked = Vec::new();
        let result = run_with(&arguments, &mut |check, touched| {
            asked.push(format!("{} {}", check.shown(), touched.len()));
            answer(check)
        });
        (result, asked)
    }

    const TWO_FILES: &str = "\
@@ file: DIR/a.md
@@ from
old
@@ to
new
@@ end
@@ write: DIR/made/b.md
made
@@ end
@@ delete: DIR/c.md
@@ check: links
@@ check: test -p xtask
";

    fn seed(dir: &Path) {
        fs::write(dir.join("a.md"), "old\n").unwrap();
        fs::write(dir.join("c.md"), "doomed\n").unwrap();
    }

    #[test]
    fn every_change_stays_when_every_check_passes() {
        let scratch = scratch("passes");
        let dir = scratch.path();
        seed(dir);
        let (result, asked) = apply(dir, TWO_FILES, &[], |_| Ok(()));
        result.unwrap();
        assert_eq!(asked, ["links 3", "test -p xtask 3"]);
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "new\n");
        assert_eq!(fs::read_to_string(dir.join("made/b.md")).unwrap(), "made\n");
        assert!(!dir.join("c.md").exists());
    }

    #[test]
    fn a_failed_check_puts_every_file_back_and_runs_no_later_check() {
        let scratch = scratch("fails");
        let dir = scratch.path();
        seed(dir);
        let (result, asked) = apply(dir, TWO_FILES, &[], |check| match check {
            Check::Links => Err("a broken link".to_owned()),
            _ => Ok(()),
        });
        let error = result.unwrap_err();
        assert!(error.contains("`links` failed: a broken link"), "{error}");
        assert!(error.contains("put back"), "{error}");
        assert_eq!(asked, ["links 3"]);
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "old\n");
        assert!(!dir.join("made/b.md").exists());
        assert_eq!(fs::read_to_string(dir.join("c.md")).unwrap(), "doomed\n");
    }

    #[test]
    fn keep_leaves_the_changes_when_a_check_fails() {
        let scratch = scratch("keep");
        let dir = scratch.path();
        seed(dir);
        let (result, _) = apply(dir, TWO_FILES, &["--keep"], |_| Err("no".to_owned()));
        assert!(result.unwrap_err().contains("left in place"));
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "new\n");
        assert!(!dir.join("c.md").exists());
    }

    #[test]
    fn a_change_that_cannot_be_made_writes_nothing_and_checks_nothing() {
        let scratch = scratch("refused");
        let dir = scratch.path();
        seed(dir);
        fs::write(dir.join("a.md"), "something else\n").unwrap();
        let (result, asked) = apply(dir, TWO_FILES, &[], |_| Ok(()));
        assert!(result.unwrap_err().contains("Nothing was written"));
        assert!(asked.is_empty());
        assert!(!dir.join("made").exists());
        assert_eq!(fs::read_to_string(dir.join("c.md")).unwrap(), "doomed\n");
    }

    #[test]
    fn a_dry_run_writes_nothing_and_checks_nothing() {
        let scratch = scratch("dry");
        let dir = scratch.path();
        seed(dir);
        let (result, asked) = apply(dir, TWO_FILES, &["--dry-run"], |_| Ok(()));
        result.unwrap();
        assert!(asked.is_empty());
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "old\n");
        assert!(dir.join("c.md").exists());
    }

    #[test]
    fn a_file_that_is_there_and_cannot_be_read_is_never_taken_as_absent() {
        let scratch = scratch("unreadable");
        let dir = scratch.path();
        // A folder where the script writes a file: there, and not readable
        // as a file.
        fs::create_dir_all(dir.join("a.md")).unwrap();
        fs::write(dir.join("b.md"), "old\n").unwrap();
        let script = "@@ file: DIR/b.md\n@@ from\nold\n@@ to\nnew\n@@ end\n\
                      @@ write: DIR/a.md\nx\n@@ end\n@@ check: links\n";
        let (result, asked) = apply(dir, script, &[], |_| Err("no".to_owned()));
        let error = result.unwrap_err();
        assert!(error.contains("cannot be read"), "{error}");
        assert!(asked.is_empty(), "a check ran");
        assert!(dir.join("a.md").is_dir(), "the folder was touched");
        assert_eq!(fs::read_to_string(dir.join("b.md")).unwrap(), "old\n");
    }

    #[test]
    fn one_file_spelt_two_ways_is_one_file_of_the_plan() {
        let scratch = scratch("spelt");
        let dir = scratch.path();
        fs::write(dir.join("a.md"), "one\n").unwrap();
        let script = "@@ file: DIR/a.md\n@@ from\none\n@@ to\ntwo\n@@ end\n\
                      @@ file: DIR/./sub/../a.md\n@@ from\ntwo\n@@ to\nthree\n@@ end\n";
        let (result, _) = apply(dir, script, &[], |_| Ok(()));
        result.unwrap();
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "three\n");
        assert_eq!(same_file(Path::new("./x/./y/../z")), Path::new("x/z"));
        assert_eq!(same_file(Path::new("../x")), Path::new("../x"));
    }

    #[cfg(unix)]
    #[test]
    fn a_deleted_file_comes_back_with_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = scratch("permissions");
        let dir = scratch.path();
        let hook = dir.join("hook");
        fs::write(&hook, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let (result, _) = apply(dir, "@@ delete: DIR/hook\n@@ check: links\n", &[], |_| {
            Err("no".to_owned())
        });
        assert!(result.is_err());
        let mode = fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "{mode:o}");
    }

    #[test]
    fn its_flags_are_checked() {
        let scratch = scratch("flags");
        let dir = scratch.path();
        let (result, _) = apply(dir, TWO_FILES, &["--force"], |_| Ok(()));
        assert!(result.unwrap_err().contains("unknown flag `--force`"));
        assert!(run_with(&[], &mut |_, _| Ok(())).is_err());
    }
}
