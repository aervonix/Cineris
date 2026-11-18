//! `cargo xtask rename`: the names the project had before decision 0038
//! (Cortex-Harness, `cortex`, `cortex-*`, `CORTEX_*`, `.cortex/`, the alias
//! `cx`) turned into Cineris's, in a working tree or in each commit of a
//! branch.
//!
//! **The table is here, and only here:** [`NAMES`] for the brand's forms,
//! [`DEV_FOLDER`] for the developer's folders that sat inside the product's
//! folder name, [`is_alias`] for the alias, and [`KEPT`] for the files and
//! lines where an old name stays, each with its reason. Everything else
//! follows from them:
//!
//! - **`apply`** rewrites the working tree: each file's contents, then its
//!   name. It is a function of the tree alone, so a second run changes
//!   nothing, and a branch rewritten later meets the very text this one made.
//! - **`--check`** fails on an old name outside [`KEPT`]. `cargo xtask rules`
//!   runs it on every tracked file, so the lint job keeps the old names out.
//! - **`branch BASE..TIP`** makes each commit of the branch again with its
//!   tree passed through the same function (`rename/branch.rs`).
//!
//! **What "the same function" holds** ([`transform`]): the names, in the
//! text and in the path; then, for a text that changed, the order of a
//! `Cargo.lock`'s packages and dependencies (cargo sorts them by name, and
//! the new prefix sorts elsewhere), and `rustfmt` on a Rust file that was
//! formatted before (a longer name can push a line over the width, and
//! imports are sorted by name). A Rust file `rustfmt` would change as it
//! stands (a test's sample of odd code) is left as the names leave it.
//!
//! The generated files (the protocol's schema, the configuration's
//! reference, the screens and transcripts, the open items' index) are
//! renamed as text too, and `apply --regen` then has their own commands
//! write them again: the screens' layout depends on the names' widths.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::Result;
use crate::rules::Violation;
use crate::workspace;

mod branch;
#[cfg(test)]
mod tests;

const USAGE: &str = "\
usage: cargo xtask rename apply [--regen]
       cargo xtask rename --check
       cargo xtask rename --kept
       cargo xtask rename branch BASE..TIP [--to NAME] [--force]

The project's names before decision 0038, turned into Cineris's: the brand
Cortex-Harness and `cortex` naming the project, the crates `cortex-*`, the
identifiers `cortex_*` and `Cortex*`, `CORTEX_*`, the folders `…/cortex-harness/`
and `.cortex/`, the alias `cx`. The table, and the short list of files and
lines where an old name stays and why, are in xtask/src/rename.rs.

apply       Rewrites the working tree (the files git tracks, and new files it
            does not ignore): every file's text, then the names of files and
            folders, after checking that no two paths become one. A text that
            changed is then put in the form its tools would give it: a
            Cargo.lock's packages in cargo's order, a Rust file that was
            formatted formatted again with the repository's rustfmt.toml.
            A second run changes nothing. --regen then runs `cargo xtask
            regen` and `cargo xtask outstanding`, which write the generated
            files again from the renamed sources.

--check     Fails on any old name left in a tracked file's text or path,
            outside the kept lines. `cargo xtask rules` runs the same check,
            so the lint job does too.

--kept      Lists the kept files and lines, each with its reason.

branch      Makes each commit of BASE..TIP again, its tree passed through what
            `apply` does (without --regen), on top of one commit that is BASE
            passed through it, and points the branch NAME (TIP's name with
            `-renamed`, when TIP is a branch's name) at the last. The messages
            and the identities in the commits are renamed too; the dates are
            kept, so the same range always gives the same commits. Nothing is
            checked out and no other ref moves: the commits are written with
            `git fast-import`. Run it from a checkout of the renamed main, so
            that this is the tool that runs. Then:

              git rebase --onto origin/main <BASE's commit, renamed> NAME

            (the command is printed with the hash). The rebase replays only
            the branch's own changes, already in the new names, so it stops
            only where the branch and main changed the same lines since BASE.
            A generated file that stops it: take either side, finish, and run
            `cargo xtask regen` and `cargo xtask outstanding`, as after any
            rebase. BASE..TIP must be a line of commits with no merge. NAME
            that exists is refused unless it already holds these commits, or
            --force replaces it.";

/// `cargo xtask rename …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match arguments.as_slice() {
        ["apply"] => apply(&workspace::root(), false),
        ["apply", "--regen"] => apply(&workspace::root(), true),
        ["--check"] => check(&workspace::root()),
        ["--kept"] => {
            for kept in &KEPT {
                let place = kept.line.map_or_else(
                    || {
                        if kept.path.ends_with('/') {
                            "every file in it".to_owned()
                        } else {
                            "the whole file".to_owned()
                        }
                    },
                    |line| format!("lines holding `{line}`"),
                );
                println!("{}: {place}: {}", kept.path, kept.why);
            }
            Ok(())
        }
        ["branch", rest @ ..] => branch::run(&workspace::root(), rest),
        ["--help" | "-h" | "help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// The table.

/// The brand's forms, old to new, longest first. `cortex` naming this
/// project, in every case form a file of the repository used; an old name
/// spelled any other way is left as it is, for `--check` to find.
const NAMES: [(&str, &str); 6] = [
    ("Cortex-Harness", "Cineris"),
    ("cortex-harness", "cineris"),
    ("CORTEX-HARNESS", "CINERIS"),
    ("CORTEX", "CINERIS"),
    ("Cortex", "Cineris"),
    ("cortex", "cineris"),
];

/// The folder the developer's own files go to: the owner-terms file and the
/// corpus cache sat under the product's folder name (`cortex-harness`), and
/// go under a name of their own, so that none sits in the product's folder.
const DEV_FOLDER: &str = "cineris-dev";

/// What sits in [`DEV_FOLDER`]: the name that follows the old folder, after
/// a path's separator or a `.join(…)`, marks it as the developer's.
const DEV_ITEMS: [&str; 2] = ["owner-terms", "corpus"];

/// The old folder name that [`DEV_ITEMS`] can follow.
const DEV_OLD: &str = "cortex-harness";

/// Where an old name stays, and why. A `path` that ends in `/` is a folder;
/// with no `line`, the whole file keeps its text, and with one, the lines
/// that hold that text do. Paths are the renamed ones, so that an entry
/// holds before the rename (in a branch's old commits) and after it.
pub(crate) struct Kept {
    pub(crate) path: &'static str,
    pub(crate) line: Option<&'static str>,
    pub(crate) why: &'static str,
}

/// The allow-list: the tool itself, the two history records and the two
/// records of the rename, and three comments where `cx` is a variable.
pub(crate) const KEPT: [Kept; 10] = [
    Kept {
        path: "xtask/src/rename.rs",
        line: None,
        why: "the rename's own table names the old names",
    },
    Kept {
        path: "xtask/src/rename/",
        line: None,
        why: "the rename's branch rewrite and its tests, which need the old names as input",
    },
    Kept {
        path: "docs/decisions/0013-naming.md",
        line: None,
        why: "the decision of the old names, kept as the record of what was decided then",
    },
    Kept {
        path: "docs/decisions/0038-the-project-is-named-cineris.md",
        line: None,
        why: "the decision of the rename says what the names were and why they changed",
    },
    Kept {
        path: "docs/steps/rename-cineris/",
        line: None,
        why: "the rename's own record: its inventory lists the old forms",
    },
    Kept {
        path: "docs/decisions/README.md",
        line: Some("[0013](0013-naming.md)"),
        why: "0013's row in the index carries 0013's own title",
    },
    Kept {
        path: "docs/build-plan.md",
        line: Some("**The name clash step 2 found.**"),
        why: "the owner's decision on the clash with Snowflake's `cortex`, as it was taken",
    },
    Kept {
        path: "crates/cineris-tools/src/contract.rs",
        line: Some("in `cx`, and nothing else"),
        why: "`cx` is the tool's context here, not the alias",
    },
    Kept {
        path: "crates/cineris-tools/src/contract.rs",
        line: Some("the read gate in `cx` keeps"),
        why: "`cx` is the tool's context here, not the alias",
    },
    Kept {
        path: "crates/cineris-tools/src/lib.rs",
        line: Some("within the limits in `cx`"),
        why: "`cx` is the tool's context here, not the alias",
    },
];

fn kept_entries(path: &str) -> impl Iterator<Item = &'static Kept> + '_ {
    KEPT.iter().filter(move |kept| {
        if kept.path.ends_with('/') {
            path.starts_with(kept.path)
        } else {
            path == kept.path
        }
    })
}

/// Whether the whole file at `path` (renamed) keeps its text and its name.
fn kept_whole(path: &str) -> bool {
    kept_entries(path).any(|kept| kept.line.is_none())
}

/// Whether `line` of the file at `path` (renamed) keeps its text.
fn kept_line(path: &str, line: &str) -> bool {
    kept_entries(path).any(|kept| kept.line.is_none_or(|text| line.contains(text)))
}

// ---------------------------------------------------------------------------
// Text and paths.

/// `text` of the file at `path` (its renamed path) with the new names, but
/// for its kept lines.
pub(crate) fn rename_text<'a>(path: &str, text: &'a str) -> Cow<'a, str> {
    if kept_whole(path) || !may_hold_old(text) {
        return Cow::Borrowed(text);
    }
    if kept_entries(path).next().is_none() {
        return rename_run(text);
    }
    // Runs of lines that are not kept are renamed together, so that a name
    // the formatter split over two lines (`.join(…)` on the next) is seen.
    let mut out = String::with_capacity(text.len() + text.len() / 32);
    let mut run_start = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if kept_line(path, line) {
            out.push_str(&rename_run(&text[run_start..at]));
            out.push_str(line);
            run_start = at + line.len();
        }
        at += line.len();
    }
    out.push_str(&rename_run(&text[run_start..]));
    if out == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(out)
    }
}

fn rename_run(text: &str) -> Cow<'_, str> {
    match rename_names(text) {
        Cow::Borrowed(same) => rename_alias(same),
        Cow::Owned(names) => Cow::Owned(rename_alias(&names).into_owned()),
    }
}

/// A path, renamed. A kept file keeps its name.
pub(crate) fn rename_path(path: &str) -> Cow<'_, str> {
    if kept_whole(path) {
        return Cow::Borrowed(path);
    }
    rename_names(path)
}

/// Where the next old name starts in `text` from `from`, in any case.
/// `lower` is `text` in lower case: the search is the standard library's,
/// which is fast in a test's unoptimised build too.
fn next_old_in(lower: &str, from: usize) -> Option<usize> {
    lower.get(from..)?.find("cortex").map(|at| at + from)
}

fn next_old(text: &str, from: usize) -> Option<usize> {
    next_old_in(&text.to_ascii_lowercase(), from)
}

/// Whether `text` may hold an old name or the old alias: most files do not,
/// and are passed over at once.
fn may_hold_old(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("cortex") || lower.contains("cx")
}

/// The brand's forms, [`NAMES`], with [`DEV_FOLDER`] for the developer's
/// folders.
fn rename_names(text: &str) -> Cow<'_, str> {
    let lower = text.to_ascii_lowercase();
    let Some(first) = next_old_in(&lower, 0) else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len() + 64);
    let mut done = 0;
    let mut at = Some(first);
    while let Some(start) = at {
        out.push_str(&text[done..start]);
        let rest = &text[start..];
        // The brand in a spelling the table does not know is left whole,
        // not renamed in part (`cortex-Harness`).
        let brand = rest
            .get(..DEV_OLD.len())
            .filter(|head| head.eq_ignore_ascii_case(DEV_OLD))
            .map(str::len);
        let known = NAMES
            .iter()
            .find(|(old, _)| rest.starts_with(old))
            .filter(|(old, _)| brand.is_none_or(|length| old.len() == length));
        if known.is_none()
            && let Some(length) = brand
        {
            out.push_str(&rest[..length]);
            done = start + length;
            at = next_old_in(&lower, done);
            continue;
        }
        match known {
            Some((old, new)) => {
                if *old == DEV_OLD && names_dev_item(&rest[old.len()..]) {
                    out.push_str(DEV_FOLDER);
                } else {
                    out.push_str(new);
                }
                done = start + old.len();
            }
            // A spelling the table does not know: left for `--check`.
            None => {
                out.push_str(&rest[..6]);
                done = start + 6;
            }
        }
        at = next_old_in(&lower, done);
    }
    out.push_str(&text[done..]);
    if out == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(out)
    }
}

/// Whether what follows the old folder's name is one of [`DEV_ITEMS`]: after
/// `/`, `\`, an escaped `\\`, or `")` and `.join("` with any space between.
fn names_dev_item(rest: &str) -> bool {
    let after = if let Some(after) = rest.strip_prefix('/') {
        after
    } else if let Some(after) = rest.strip_prefix("\\\\") {
        after
    } else if let Some(after) = rest.strip_prefix('\\') {
        after
    } else if let Some(after) = rest.strip_prefix("\")") {
        match after.trim_start().strip_prefix(".join(\"") {
            Some(after) => after,
            None => return false,
        }
    } else {
        return false;
    };
    DEV_ITEMS.iter().any(|item| {
        after.strip_prefix(item).is_some_and(|end| {
            !end.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
    })
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether the `cx` (or `CX`) at `at` in `text` names the alias, rather
/// than a variable or part of a word. The alias is quoted (`` `cx` ``,
/// `"cx"`), part of a path (`bin/cx`, `C:\tools\CX.EXE`), a file (`cx.exe`),
/// a command with what follows it (`` `cx --version` ``, `"cx: error"`), the
/// start of a name made from it (`cx-tree-guard`), or named as the command
/// ("alias cx", "Usage: cx"). `cx.cancel`, `&cx`, `as_cx`, `cx: &RunCx` and
/// `"cx::it"` are not.
pub(crate) fn is_alias(text: &str, at: usize) -> bool {
    let bytes = text.as_bytes();
    let lower = match bytes.get(at..at + 2) {
        Some(b"cx") => true,
        Some(b"CX") => false,
        _ => return false,
    };
    let before = at.checked_sub(1).map(|index| bytes[index]);
    // A word, a field (`self.cx`) or a flag (`--cx`) goes on before it.
    if before.is_some_and(|byte| is_word_byte(byte) || byte == b'.' || byte == b'-') {
        return false;
    }
    let rest = &text[at + 2..];
    let after = rest.bytes().next();
    let file = [".exe", ".EXE", ".cmd"].iter().any(|extension| {
        rest.strip_prefix(extension)
            .is_some_and(|end| !end.bytes().next().is_some_and(is_word_byte))
    });
    if !lower {
        return file && rest.starts_with(".EXE");
    }
    if file {
        return true;
    }
    // A name made from it: `cx-tree-guard`, `cx-child=`, `/tmp/cx-{name}`.
    if rest.starts_with('-')
        && rest[1..]
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'{')
    {
        return true;
    }
    // Quoted, the end or a part of a path, or a command with what follows:
    // `` `cx` ``, `"cx"`, `bin/cx`, `/config/cx/`, `"cx: error"`,
    // `` `cx --version` ``, `binary_id(=cx)`, `format!("cx{}", …)`. Not a
    // path into it (`cx::it`).
    let opened =
        before.is_some_and(|byte| matches!(byte, b'`' | b'"' | b'\'' | b'/' | b'\\' | b'='));
    let closed = after.is_some_and(|byte| {
        matches!(
            byte,
            b'`' | b'"' | b'\'' | b'/' | b'\\' | b'{' | b')' | b' '
        )
    }) || (after == Some(b':') && !rest.starts_with("::"));
    if opened && closed {
        return true;
    }
    // Named as the command in prose or in usage text.
    ["alias ", "Usage: "]
        .iter()
        .any(|word| text[..at].ends_with(word))
        && !after.is_some_and(is_word_byte)
}

/// The alias, `cx`, as `cin` wherever [`is_alias`] says it is the alias.
fn rename_alias(text: &str) -> Cow<'_, str> {
    let positions = alias_positions(text);
    if positions.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + positions.len());
    let mut done = 0;
    for at in positions {
        out.push_str(&text[done..at]);
        out.push_str(if text.as_bytes()[at] == b'c' {
            "cin"
        } else {
            "CIN"
        });
        done = at + 2;
    }
    out.push_str(&text[done..]);
    Cow::Owned(out)
}

fn alias_positions(text: &str) -> Vec<usize> {
    text.to_ascii_lowercase()
        .match_indices("cx")
        .map(|(at, _)| at)
        .filter(|&at| is_alias(text, at))
        .collect()
}

// ---------------------------------------------------------------------------
// One file: the function `apply` and `branch` share.

/// How to format Rust text: `rustfmt` with the repository's configuration,
/// run in the repository so that its pinned toolchain is the one, or not at
/// all (the tests of the names alone).
pub(crate) struct Formatter {
    at: Option<PathBuf>,
}

impl Formatter {
    /// `rustfmt` in `root`, once it has answered: a rename whose formatter
    /// cannot run would give other text than the one that renamed `main`.
    pub(crate) fn at(root: &Path) -> Result<Self> {
        let ran = Command::new("rustfmt")
            .arg("--version")
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !ran {
            return Err(
                "rustfmt does not run here: the rename formats again each Rust file it changes, \
                 and without it its text would differ from the one that renamed main \
                 (`rustup component add rustfmt`)"
                    .to_owned(),
            );
        }
        Ok(Self {
            at: Some(root.to_path_buf()),
        })
    }

    #[cfg(test)]
    pub(crate) fn none() -> Self {
        Self { at: None }
    }

    /// `text` formatted, or `None` when it cannot be (it does not parse).
    fn format(&self, text: &str) -> Option<String> {
        let root = self.at.as_ref()?;
        let mut child = Command::new("rustfmt")
            .args(["--edition", "2024", "--emit", "stdout", "--config-path"])
            .arg(root.join("rustfmt.toml"))
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        let input = text.to_owned();
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let output = child.wait_with_output().ok()?;
        writer.join().ok()?.ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }

    fn formats(&self) -> bool {
        self.at.is_some()
    }
}

/// What one file becomes: its path, and its contents when they change.
pub(crate) struct Renamed {
    pub(crate) path: String,
    pub(crate) contents: Option<Vec<u8>>,
}

/// One file of a tree, renamed: [`rename_path`] for its path, and for a text
/// [`rename_text`], then cargo's order for a `Cargo.lock` and `rustfmt` for a
/// Rust file that was formatted. A file that is not UTF-8 text (an image)
/// keeps its bytes.
pub(crate) fn transform(path: &str, bytes: &[u8], formatter: &Formatter) -> Renamed {
    let new_path = rename_path(path).into_owned();
    let contents = std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
        .and_then(|text| {
            let Cow::Owned(renamed) = rename_text(&new_path, text) else {
                return None;
            };
            let renamed = if new_path.ends_with("Cargo.lock") {
                sort_lock(&renamed)
            } else if new_path.ends_with(".rs") && formatter.formats() {
                format_rust(text, renamed, formatter)
            } else {
                renamed
            };
            Some(renamed.into_bytes())
        });
    Renamed {
        path: new_path,
        contents,
    }
}

/// The renamed text of a Rust file, formatted when the file was.
fn format_rust(original: &str, renamed: String, formatter: &Formatter) -> String {
    if formatter.format(original).as_deref() != Some(original) {
        return renamed;
    }
    formatter.format(&renamed).unwrap_or(renamed)
}

/// A `Cargo.lock` with its packages, and each package's dependencies, in
/// cargo's order: by name, those of one name in the order they had (cargo
/// orders them by version, which a rename does not change). A lock of
/// another shape is left as it is.
pub(crate) fn sort_lock(text: &str) -> String {
    const PACKAGE: &str = "[[package]]\n";
    let Some(start) = text.find(PACKAGE) else {
        return text.to_owned();
    };
    if text.contains("\n[metadata]") || !text.ends_with('\n') {
        return text.to_owned();
    }
    let (head, body) = text.split_at(start);
    let body = &body[PACKAGE.len()..];
    let mut packages: Vec<String> = body
        .split("\n[[package]]\n")
        .map(sort_dependencies)
        .collect();
    packages.sort_by(|a, b| package_name(a).cmp(package_name(b)));
    format!("{head}{PACKAGE}{}", packages.join("\n[[package]]\n"))
}

fn package_name(package: &str) -> &str {
    package
        .lines()
        .find_map(|line| line.strip_prefix("name = \""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or("")
}

fn sort_dependencies(package: &str) -> String {
    let lines: Vec<&str> = package.split_inclusive('\n').collect();
    let mut out = String::with_capacity(package.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        out.push_str(line);
        index += 1;
        if line == "dependencies = [\n" {
            let end = lines[index..]
                .iter()
                .position(|line| line.starts_with(']'))
                .map_or(lines.len(), |offset| index + offset);
            let mut entries = lines[index..end].to_vec();
            entries.sort_by(|a, b| dependency_name(a).cmp(dependency_name(b)));
            entries.iter().for_each(|entry| out.push_str(entry));
            index = end;
        }
    }
    out
}

fn dependency_name(entry: &str) -> &str {
    let entry = entry.trim().trim_start_matches('"');
    entry.split([' ', '"']).next().unwrap_or(entry)
}

// ---------------------------------------------------------------------------
// The check.

/// The old names left in one tracked file, outside [`KEPT`]: in its path, and
/// in each line of its text. Run by `cargo xtask rules` on every file.
pub(crate) fn violations(file: &str, bytes: &[u8]) -> Vec<Violation> {
    let mut found = Vec::new();
    if kept_whole(file) {
        return found;
    }
    if next_old(file, 0).is_some() {
        found.push(Violation {
            file: file.to_owned(),
            line: None,
            message: "the path holds an old name of the project. `cargo xtask rename apply` \
                      renames it (docs/decisions/0038-the-project-is-named-cineris.md)."
                .to_owned(),
        });
    }
    let Some(text) = std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
    else {
        // Bytes that are not text are not renamed: the check says so of one
        // that holds the old name.
        if String::from_utf8_lossy(bytes)
            .to_ascii_lowercase()
            .contains("cortex")
        {
            found.push(Violation {
                file: file.to_owned(),
                line: None,
                message: "a file that is not UTF-8 text holds an old name of the project, \
                          which the rename does not change: make it again with the new name"
                    .to_owned(),
            });
        }
        return found;
    };
    if !may_hold_old(text) {
        return found;
    }
    // Each hit's line, found from the line starts: one pass over the text.
    let lower = text.to_ascii_lowercase();
    let mut hits: BTreeMap<usize, &str> = BTreeMap::new();
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let line_of = |at: usize| starts.partition_point(|&start| start <= at) - 1;
    for at in alias_positions(text) {
        hits.insert(line_of(at), "the old alias");
    }
    for (at, _) in lower.match_indices("cortex") {
        hits.insert(line_of(at), "an old name of the project");
    }
    for (index, what) in hits {
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        if kept_line(file, &text[starts[index]..end]) {
            continue;
        }
        found.push(Violation {
            file: file.to_owned(),
            line: Some(index + 1),
            message: format!(
                "{what}. `cargo xtask rename apply` renames it; a line that must keep it \
                 goes in KEPT in xtask/src/rename.rs, with its reason \
                 (docs/decisions/0038-the-project-is-named-cineris.md)."
            ),
        });
    }
    found
}

/// `cargo xtask rename --check`
fn check(root: &Path) -> Result<()> {
    let mut found = Vec::new();
    for file in workspace::tracked_files(root)? {
        let bytes = fs::read(root.join(&file))
            .map_err(|error| format!("could not read {file}: {error}"))?;
        found.extend(violations(&file, &bytes));
    }
    if found.is_empty() {
        println!("No old name is left outside the kept lines.");
        return Ok(());
    }
    for violation in &found {
        println!("{violation}\n");
    }
    Err(format!("{} old name(s) left", found.len()))
}

// ---------------------------------------------------------------------------
// The working tree.

/// Each of `items` through `work`, on every core, in the order given.
pub(crate) fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads.min(items.len()) {
            scope.spawn(|| {
                let mut mine = Vec::new();
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    mine.push((index, work(item)));
                }
                done.lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .append(&mut mine);
            });
        }
    });
    let mut done = done
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner());
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

/// Where each file goes, checked: no two paths become one, and none lands
/// on a file that is not itself moving. `moves` is (old, new).
pub(crate) fn check_moves<'a>(moves: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<()> {
    let moves: Vec<(&str, &str)> = moves.into_iter().collect();
    let olds: BTreeSet<&str> = moves.iter().map(|(old, _)| *old).collect();
    let mut targets: BTreeMap<&str, &str> = BTreeMap::new();
    for (old, new) in &moves {
        if let Some(other) = targets.insert(new, old) {
            return Err(format!("{other} and {old} would both become {new}"));
        }
    }
    for (old, new) in &moves {
        if old != new && olds.contains(new) && !moves.iter().any(|(o, n)| o == new && n != new) {
            return Err(format!("{old} would become {new}, which is there already"));
        }
    }
    Ok(())
}

/// `cargo xtask rename apply [--regen]`
fn apply(root: &Path, regen: bool) -> Result<()> {
    let applied = apply_tree(root, &Formatter::at(root)?)?;
    if applied.written == 0 && applied.moved == 0 {
        println!("Nothing to change: the tree has the new names.");
    } else {
        println!(
            "Renamed: {} file(s)' text and {} path(s).",
            applied.written, applied.moved
        );
    }
    for dir in &applied.stayed {
        println!("warning: {dir} still holds files git does not track; they were not moved");
    }
    if regen {
        for command in [["regen"].as_slice(), ["outstanding"].as_slice()] {
            let status = Command::new(workspace::cargo())
                .args(["run", "--locked", "--quiet", "--package", "xtask", "--"])
                .args(command)
                .current_dir(root)
                .status()
                .map_err(|error| format!("could not run cargo: {error}"))?;
            if !status.success() {
                return Err(format!("`cargo xtask {}` failed", command.join(" ")));
            }
        }
    }
    Ok(())
}

/// What [`apply_tree`] did.
pub(crate) struct Applied {
    /// Files whose text changed.
    pub(crate) written: usize,
    /// Files that moved.
    pub(crate) moved: usize,
    /// Folders with an old name that still hold files git does not track.
    pub(crate) stayed: Vec<String>,
}

/// The working tree at `root` renamed: every file's text, then the paths.
pub(crate) fn apply_tree(root: &Path, formatter: &Formatter) -> Result<Applied> {
    let files = workspace::repository_files(root)?;
    // A link is read through and written through, and its target is never
    // renamed: refused, as `branch` refuses one. None is tracked.
    if let Some(link) = files.iter().find(|file| {
        fs::symlink_metadata(root.join(file)).is_ok_and(|meta| meta.file_type().is_symlink())
    }) {
        return Err(format!(
            "{link} is a symbolic link: the rename does not rename links"
        ));
    }
    let renamed: Vec<Result<Renamed>> = in_parallel(&files, |file| {
        let bytes =
            fs::read(root.join(file)).map_err(|error| format!("could not read {file}: {error}"))?;
        Ok(transform(file, &bytes, formatter))
    });
    let renamed = renamed.into_iter().collect::<Result<Vec<_>>>()?;
    check_moves(
        files
            .iter()
            .zip(&renamed)
            .map(|(old, new)| (old.as_str(), new.path.as_str())),
    )?;

    let mut written = 0;
    for (file, new) in files.iter().zip(&renamed) {
        if let Some(contents) = &new.contents {
            fs::write(root.join(file), contents)
                .map_err(|error| format!("could not write {file}: {error}"))?;
            written += 1;
        }
    }
    let mut moved = 0;
    let mut left = BTreeSet::new();
    for (file, new) in files.iter().zip(&renamed) {
        if *file == new.path {
            continue;
        }
        let to = root.join(&new.path);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not make {}: {error}", parent.display()))?;
        }
        fs::rename(root.join(file), &to)
            .map_err(|error| format!("could not move {file} to {}: {error}", new.path))?;
        moved += 1;
        let mut parent = Path::new(file).parent();
        while let Some(dir) = parent.filter(|dir| !dir.as_os_str().is_empty()) {
            left.insert(dir.to_path_buf());
            parent = dir.parent();
        }
    }
    // The folders the moves emptied, deepest first. One that still holds a
    // file git ignores stays, and is named (the outermost only).
    let mut stayed: Vec<String> = Vec::new();
    for dir in left.iter().rev() {
        let full = root.join(dir);
        let name = dir.to_string_lossy().replace('\\', "/");
        if full.is_dir() && fs::remove_dir(&full).is_err() && next_old(&name, 0).is_some() {
            stayed.retain(|inner| !inner.starts_with(&format!("{name}/")));
            stayed.push(name);
        }
    }
    stayed.sort();
    Ok(Applied {
        written,
        moved,
        stayed,
    })
}
