//! `cargo xtask outstanding`: the index of `docs/outstanding/README.md`,
//! written from the items themselves (since the tenth tooling step).
//!
//! Every open item is a file of its own, so that two steps that each leave
//! an item open touch different files. The index listed them by hand, so
//! two pull requests that each added an item changed the same lines, and
//! every rebase of one onto the other stopped there. Now each item's file
//! holds what its line in the index says:
//!
//! ```text
//! `interrupt` on an idle session does not stop a background summary request.
//!
//! - Area: cortex-agent
//! - Owner: step 32
//!
//! | Item | Who | When |
//! |---|---|---|
//! | **…** | Step 32 (the hardening pass) | Before the first release |
//! ```
//!
//! - **The first line** is the item in a sentence: the text of its line in
//!   the index.
//! - **`Area`** is the crate the item is about (`cortex-tui`), or one of
//!   [`AREAS`]: the heading it is listed under.
//! - **`Owner`** is who settles it, in the few words the index gives; the
//!   table's `Who` says it in full.
//! - **`Flaky`**, any number of times, names a test the item is about that
//!   fails now and then for a reason not yet fixed:
//!   `` - Flaky: `cortex-core::it trust::sys_…` ``, the test's binary and its
//!   name as nextest gives them. `stress`, `ci` and `mutate` read these.
//!
//! The index is sorted (the crates by name, then [`AREAS`] in their order;
//! the items of an area by name), and `cargo xtask rules` fails when it is
//! not what this command writes. Git merges the index's two sides line by
//! line (`merge=union` in `.gitattributes`), so a rebase never stops on it;
//! whatever such a merge gets wrong (a line twice, a line of an item that is
//! gone, two lines out of order, a line lost) the rule finds, and running
//! this command again puts right.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::{Result, workspace};

/// The folder of the open items, from the repository's root.
pub(crate) const FOLDER: &str = "docs/outstanding/";

/// The index, from the repository's root.
pub(crate) const INDEX: &str = "docs/outstanding/README.md";

/// The header of an item's table.
pub(crate) const ITEM_HEADER: [&str; 2] = ["| Item | Who | When |", "|---|---|---|"];

/// The areas that are not a crate, in the order the index lists them after
/// the crates: the value of `Area`, and the heading it is listed under.
pub(crate) const AREAS: [(&str, &str); 3] = [
    ("tooling", "The tooling and CI"),
    ("windows", "Windows"),
    ("other", "Other"),
];

const USAGE: &str = "\
usage: cargo xtask outstanding [--check]

Writes the index of docs/outstanding/README.md from the items' files: each
item's first line, its `- Area:` and its `- Owner:`. --check writes nothing
and fails when the index is not what it would write.

After a rebase: `cargo xtask outstanding`, and commit the index it wrote.";

/// What the index says above the items. It is written with them, so that
/// the whole page is the command's.
const PREAMBLE: &str = "\
# Still outstanding

What the owner or a later step still has to settle: one file per item. Together they are the list that \"Still outstanding\" in [build-plan.md](../build-plan.md) points to.

**This page is written by `cargo xtask outstanding`** from the items' files (since the tenth tooling step): edit an item's file, never this page. `cargo xtask rules` fails when the page is not what the command writes.

- **Leaving something open:** add a file, named for the item in lowercase words joined by hyphens, and run `cargo xtask outstanding`. The file is the item in a sentence on its first line, which is its line in the index below; a blank line; the fields, a list: `- Area:` the crate it is about (`cortex-tui`) or `tooling`, `windows` or `other`, `- Owner:` who settles it, in a few words, and `- Flaky:` for each test the item is about that fails now and then, its binary and name as nextest gives them (`` - Flaky: `cortex-core::it trust::sys_…` ``), which `stress`, `ci` and `mutate` read; a blank line; and a table of one row, under the header `| Item | Who | When |`: the item in full, who settles it, and when.
- **Settling an item:** delete its file, run `cargo xtask outstanding`, and record the answer in the settling step's `steps/<step>/answered.md`.
- **Changing an item:** edit its file, and run the command if its first line or a field changed.
- **An item several steps share** stays one file, and each step that does its part leaves the file alone and records that part in its own `steps/<step>/answered.md`. Only the step that finishes the item deletes its file, so parallel steps never edit it.
- **After a rebase:** `cargo xtask outstanding`. Git merges this page line by line, keeping both sides' lines (`merge=union` in `.gitattributes`), so a rebase does not stop here; what that merge gets wrong (a line twice, the line of an item that is gone, a line lost, lines out of order) `rules` finds and the command puts right.

One file per item, and not one per step, because a step settles items that other steps left open: two steps that settle different items then touch different files.

## The items, by crate or area

One line an item: what it covers and who settles it. It is the place to look first when a review asks whether a finding is already known.
";

/// A test an item names as flaky: the binary nextest runs it in and its
/// name there, as nextest's report gives them (`cortex-core::it` and
/// `trust::sys_…`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Flaky {
    pub(crate) binary: String,
    pub(crate) test: String,
}

/// One open item, as its file says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    /// The file's name without `.md`.
    pub(crate) name: String,
    /// The first line.
    pub(crate) summary: String,
    /// `Area`: a crate's name, or a key of [`AREAS`].
    pub(crate) area: String,
    /// `Owner`.
    pub(crate) owner: String,
    /// Every `Flaky`, in the order the file gives them.
    pub(crate) flaky: Vec<Flaky>,
}

/// What is wrong with an item's file: the line, from 1, and what to do.
pub(crate) type Problem = (usize, String);

/// Whether `name` is lower-case words of letters and digits, joined by
/// hyphens.
pub(crate) fn is_hyphenated(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// A test's name as a filter can name it exactly, and a report gives it.
fn is_test_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        && !name.starts_with(':')
        && !name.ends_with(':')
}

/// Whether `binary` is a test binary of the workspace as nextest names it:
/// a crate's name for its unit tests (`cortex-core`), or the crate's and a
/// target's (`cortex-core::it`, `cortex-cli::bin/cortex`).
fn is_binary(binary: &str, crates: &BTreeSet<String>) -> bool {
    let (package, target) = match binary.split_once("::") {
        Some((package, target)) => (package, Some(target)),
        None => (binary, None),
    };
    let target_fits = target.is_none_or(|target| {
        !target.is_empty()
            && target
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/'))
    });
    (package == "xtask" || crates.contains(package)) && target_fits
}

/// The form of an item's file, for the messages.
const FORM: &str = "an open item's file is its sentence on the first line, a blank line, its \
                    fields (`- Area: <crate, tooling, windows or other>`, `- Owner: <who \
                    settles it>`, and a `- Flaky: <binary> <test>` for each flaky test it names), \
                    a blank line, and the \
                    table's header `| Item | Who | When |`, the line `|---|---|---|` and one \
                    row.\n    docs/outstanding/README.md has the layout";

fn problem(line: usize, what: impl AsRef<str>) -> Problem {
    (line, format!("{}.\n    {FORM}.", what.as_ref()))
}

/// Reads an item's file. `name` is its name without `.md`; `crates` are
/// the names an `Area` that is a crate may have, and `Flaky`'s binary
/// begins with one of them or with `xtask`.
pub(crate) fn parse(
    name: &str,
    text: &str,
    crates: &BTreeSet<String>,
) -> std::result::Result<Item, Problem> {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |last| last + 1);
    let lines = &lines[..end];

    let summary = lines.first().copied().unwrap_or("").trim_end();
    if summary.trim().is_empty()
        || summary.starts_with(['|', '#', '-', '>', ' ', '*'])
        || summary.starts_with("<!--")
    {
        return Err(problem(
            1,
            format!(
                "the first line is `{summary}`, and it should be the item in a sentence: its \
                 line in the index"
            ),
        ));
    }
    if lines.get(1).is_none_or(|line| !line.trim().is_empty()) {
        return Err(problem(
            2,
            "the item's sentence is one line, followed by a blank line",
        ));
    }

    let mut area: Option<String> = None;
    let mut owner: Option<String> = None;
    let mut flaky = Vec::new();
    let mut at = 2;
    while let Some(line) = lines.get(at).copied() {
        if line.trim().is_empty() {
            break;
        }
        let number = at + 1;
        let Some((key, value)) = line
            .strip_prefix("- ")
            .and_then(|field| field.split_once(':'))
        else {
            return Err(problem(
                number,
                format!("`{line}` is not a field: a field is `- Key: value`"),
            ));
        };
        let value = value.trim();
        match key {
            "Area" => {
                if area.is_some() {
                    return Err(problem(number, "`Area` is given twice"));
                }
                let known = AREAS.iter().any(|(key, _)| *key == value)
                    || (value.starts_with("cortex-") && crates.contains(value));
                if !known {
                    return Err(problem(
                        number,
                        format!(
                            "`{value}` is no area: an area is a crate of `crates/` (`cortex-tui`), \
                             or {}",
                            AREAS.map(|(key, _)| format!("`{key}`")).join(", ")
                        ),
                    ));
                }
                area = Some(value.to_owned());
            }
            "Owner" => {
                if owner.is_some() {
                    return Err(problem(number, "`Owner` is given twice"));
                }
                if value.is_empty() {
                    return Err(problem(number, "`Owner` is empty"));
                }
                owner = Some(value.trim_end_matches('.').to_owned());
            }
            "Flaky" => {
                let test = value
                    .strip_prefix('`')
                    .and_then(|value| value.strip_suffix('`'))
                    .unwrap_or(value);
                let words: Vec<&str> = test.split_whitespace().collect();
                let [binary, test] = words.as_slice() else {
                    return Err(problem(
                        number,
                        format!(
                            "`Flaky` is `{value}`, and it should be a test's binary and its name, \
                             as nextest's report gives them: `cortex-core::it trust::sys_x`"
                        ),
                    ));
                };
                if !is_binary(binary, crates) {
                    return Err(problem(
                        number,
                        format!(
                            "`{binary}` is no test binary of the workspace: a crate's unit tests \
                             are its name (`cortex-core`), its integration tests its name and \
                             the target's (`cortex-core::it`)"
                        ),
                    ));
                }
                if !is_test_name(test) {
                    return Err(problem(
                        number,
                        format!(
                            "`{test}` is not a test's name as nextest gives it (letters, digits, \
                             `_` and `::`): a filter must name it exactly"
                        ),
                    ));
                }
                let flake = Flaky {
                    binary: (*binary).to_owned(),
                    test: (*test).to_owned(),
                };
                if flaky.contains(&flake) {
                    return Err(problem(number, format!("`{test}` is named twice")));
                }
                flaky.push(flake);
            }
            other => {
                return Err(problem(
                    number,
                    format!("`{other}` is no field: they are `Area`, `Owner` and `Flaky`"),
                ));
            }
        }
        at += 1;
    }
    let Some(area) = area else {
        return Err(problem(at + 1, "the item has no `- Area:`"));
    };
    let Some(owner) = owner else {
        return Err(problem(at + 1, "the item has no `- Owner:`"));
    };

    // The blank line, the header, one row and nothing after.
    let table = at + 1;
    if lines.get(at).is_none() {
        return Err(problem(table, "the table is missing"));
    }
    for (offset, expected) in ITEM_HEADER.iter().enumerate() {
        if lines.get(table + offset).copied() != Some(*expected) {
            return Err(problem(
                table + offset + 1,
                "the table's header is not there",
            ));
        }
    }
    let row = table + 2;
    match lines.get(row) {
        Some(found) if found.starts_with("| ") && found.trim_end().ends_with('|') => {}
        _ => return Err(problem(row + 1, "the table's row is not there")),
    }
    if lines.len() > row + 1 {
        return Err(problem(
            row + 2,
            "an item is one row: a second item is a second file",
        ));
    }

    Ok(Item {
        name: name.to_owned(),
        summary: summary.to_owned(),
        area,
        owner,
        flaky,
    })
}

/// The heading an area is listed under, and where it comes: the crates by
/// name, then [`AREAS`] in their order.
fn heading(area: &str) -> (usize, &str) {
    AREAS
        .iter()
        .position(|(key, _)| *key == area)
        .map_or((0, area), |at| (at + 1, AREAS[at].1))
}

/// An item's line in the index.
fn line(item: &Item) -> String {
    let summary = item.summary.trim_end();
    let stop = if summary.ends_with(['.', '?', '!']) {
        ""
    } else {
        "."
    };
    format!(
        "- [{name}]({name}.md): {summary}{stop} Owner: {owner}.",
        name = item.name,
        owner = item.owner
    )
}

/// The whole index, from the items in any order.
pub(crate) fn index(items: &[Item]) -> String {
    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by(|a, b| {
        heading(&a.area)
            .cmp(&heading(&b.area))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut text = PREAMBLE.to_owned();
    let mut current: Option<&str> = None;
    for item in sorted {
        let (_, title) = heading(&item.area);
        if current != Some(title) {
            text.push_str(&format!("\n### {title}\n\n"));
            current = Some(title);
        }
        text.push_str(&line(item));
        text.push('\n');
    }
    text
}

/// The items of `texts`, keyed by their paths from the repository's root,
/// and the problems of those that cannot be read. Files that are not items
/// (the index, anything not directly in the folder or not `.md`) are left
/// out.
pub(crate) fn read<'a>(
    texts: impl IntoIterator<Item = (&'a str, &'a str)>,
    crates: &BTreeSet<String>,
) -> (Vec<Item>, Vec<(String, Problem)>) {
    let mut items = Vec::new();
    let mut problems = Vec::new();
    for (file, text) in texts {
        let Some(path) = file.strip_prefix(FOLDER) else {
            continue;
        };
        let Some(name) = path.strip_suffix(".md") else {
            continue;
        };
        if path == "README.md" || name.contains('/') || !is_hyphenated(name) {
            continue;
        }
        match parse(name, text, crates) {
            Ok(item) => items.push(item),
            Err(problem) => problems.push((file.to_owned(), problem)),
        }
    }
    (items, problems)
}

/// What differs between the index as it is and as the items make it, for a
/// message: lines it lacks, lines it has that it should not, or the order.
pub(crate) fn difference(found: &str, expected: &str) -> String {
    let have: Vec<&str> = found.lines().collect();
    let want: Vec<&str> = expected.lines().collect();
    let lacks: Vec<&str> = want.iter().filter(|l| !have.contains(l)).copied().collect();
    let extra: Vec<&str> = have.iter().filter(|l| !want.contains(l)).copied().collect();
    let mut message = String::new();
    let shown = |lines: &[&str]| {
        let mut text = String::new();
        for line in lines.iter().take(5) {
            let cut: String = line.chars().take(120).collect();
            text.push_str(&format!("\n      {cut}"));
        }
        if lines.len() > 5 {
            text.push_str(&format!("\n      … and {} more", lines.len() - 5));
        }
        text
    };
    if !lacks.is_empty() {
        message.push_str(&format!("\n    It lacks:{}", shown(&lacks)));
    }
    if !extra.is_empty() {
        message.push_str(&format!(
            "\n    It has, and should not (an item whose file is gone, or one git does not \
             track yet: `git add` it):{}",
            shown(&extra)
        ));
    }
    let twice: BTreeSet<&str> = have
        .iter()
        .filter(|l| l.starts_with("- [") && have.iter().filter(|o| o == l).count() > 1)
        .copied()
        .collect();
    if !twice.is_empty() {
        message.push_str(&format!(
            "\n    It has twice:{}",
            shown(&twice.into_iter().collect::<Vec<_>>())
        ));
    }
    if message.is_empty() {
        message.push_str("\n    Its lines are the right ones, in another order or spacing.");
    }
    message
}

/// The crates of the workspace, by the folders of `crates/` that hold a
/// manifest, from a list of the repository's files.
pub(crate) fn crates_of<'a>(files: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
    files
        .into_iter()
        .filter_map(|file| {
            let rest = file.strip_prefix("crates/")?;
            let (name, manifest) = rest.split_once('/')?;
            (manifest == "Cargo.toml").then(|| name.to_owned())
        })
        .collect()
}

/// `cargo xtask outstanding [--check]`.
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let mut check = false;
    for argument in arguments {
        match argument.as_str() {
            "--check" => check = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    let root = workspace::root();
    let (expected, problems) = expected_index(&root)?;
    if !problems.is_empty() {
        let mut message = String::from("these items cannot be read, so the index was not written:");
        for (file, (line, what)) in &problems {
            message.push_str(&format!("\n{file}:{line}: {what}"));
        }
        return Err(message);
    }
    let path = root.join(INDEX);
    let found = fs::read_to_string(&path).unwrap_or_default();
    if found == expected {
        println!("{INDEX} is up to date.");
        return Ok(());
    }
    if check {
        return Err(format!(
            "{INDEX} is not what the items' files give:{}\n    Run `cargo xtask outstanding`.",
            difference(&found, &expected)
        ));
    }
    fs::write(&path, &expected).map_err(|error| format!("could not write {INDEX}: {error}"))?;
    println!("wrote {INDEX}:{}", difference(&found, &expected));
    Ok(())
}

/// The index the items on disk give (every `.md` of the folder, whether
/// git tracks it or not), and the problems of the items that cannot be
/// read.
fn expected_index(root: &Path) -> Result<(String, Vec<(String, Problem)>)> {
    let folder = root.join(FOLDER);
    let mut texts = Vec::new();
    let entries =
        fs::read_dir(&folder).map_err(|error| format!("could not read {FOLDER}: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read {FOLDER}: {error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().is_file() {
            continue;
        }
        let text = fs::read_to_string(entry.path())
            .map_err(|error| format!("could not read {FOLDER}{name}: {error}"))?;
        texts.push((format!("{FOLDER}{name}"), text));
    }
    let files = workspace::repository_files(root)?;
    let crates = crates_of(&files);
    let (items, problems) = read(
        texts
            .iter()
            .map(|(file, text)| (file.as_str(), text.as_str())),
        &crates,
    );
    Ok((index(&items), problems))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Flaky, difference, index, parse, read};

    fn crates() -> BTreeSet<String> {
        ["cortex-core", "cortex-tui", "cortex-index"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    }

    const ROW: &str = "| Item | Who | When |\n|---|---|---|\n| **A flag.** | Step 13 | Then |\n";

    fn item(summary: &str, fields: &str) -> String {
        format!("{summary}\n\n{fields}\n{ROW}")
    }

    #[test]
    fn an_item_is_its_sentence_its_fields_and_its_row() {
        let text = item(
            "A flag that is not read.",
            "- Area: cortex-tui\n- Owner: step 13\n- Flaky: `cortex-core::it trust::sys_x`\n\
             - Flaky: cortex-tui signals::y\n",
        );
        let parsed = parse("a-flag", &text, &crates()).unwrap();
        assert_eq!(parsed.summary, "A flag that is not read.");
        assert_eq!(parsed.area, "cortex-tui");
        assert_eq!(parsed.owner, "step 13");
        assert_eq!(
            parsed.flaky,
            [
                Flaky {
                    binary: "cortex-core::it".to_owned(),
                    test: "trust::sys_x".to_owned()
                },
                Flaky {
                    binary: "cortex-tui".to_owned(),
                    test: "signals::y".to_owned()
                }
            ]
        );
        // Blank lines at the end are allowed; an area that is no crate is
        // one of the named ones.
        let text = format!(
            "{}\n\n",
            item("x", "- Area: tooling\n- Owner: the owner.\n")
        );
        let parsed = parse("x", &text, &crates()).unwrap();
        assert_eq!(
            (parsed.area.as_str(), parsed.owner.as_str()),
            ("tooling", "the owner")
        );
    }

    #[test]
    fn a_file_out_of_form_is_refused_at_its_line() {
        let fields = "- Area: cortex-tui\n- Owner: step 13\n";
        for (text, line, needle) in [
            (ROW.to_owned(), 1, "the first line"),
            (String::new(), 1, "the first line"),
            (format!("# A flag\n\n{fields}\n{ROW}"), 1, "the first line"),
            (
                format!("A flag\nand more\n\n{fields}\n{ROW}"),
                2,
                "one line",
            ),
            (item("x", "- Owner: step 13\n"), 4, "no `- Area:`"),
            (item("x", "- Area: cortex-tui\n"), 4, "no `- Owner:`"),
            (
                item("x", "- Area: cortex-nope\n- Owner: x\n"),
                3,
                "is no area",
            ),
            (item("x", "- Area: Tooling\n- Owner: x\n"), 3, "is no area"),
            (
                item("x", "- Area: cortex-tui\n- Area: cortex-tui\n- Owner: x\n"),
                4,
                "twice",
            ),
            (item("x", "- Area: cortex-tui\n- Owner:\n"), 4, "empty"),
            (
                item("x", "- Area: cortex-tui\n- Owner: x\n- Who: y\n"),
                5,
                "no field",
            ),
            (
                item("x", "Area: cortex-tui\n- Owner: x\n"),
                3,
                "not a field",
            ),
            (
                item("x", &format!("{fields}- Flaky: cortex-core\n")),
                5,
                "binary and its name",
            ),
            (
                item("x", &format!("{fields}- Flaky: cortex-nope::it a::b\n")),
                5,
                "no test binary",
            ),
            (
                item("x", &format!("{fields}- Flaky: cortex-core::it a b\n")),
                5,
                "binary and its name",
            ),
            (
                item("x", &format!("{fields}- Flaky: cortex-core::it a(b)\n")),
                5,
                "exactly",
            ),
            (
                item(
                    "x",
                    &format!("{fields}- Flaky: xtask a::b\n- Flaky: xtask a::b\n"),
                ),
                6,
                "twice",
            ),
            (format!("x\n\n{fields}"), 5, "table is missing"),
            (
                format!("x\n\n{fields}\n| Item | Owner | When |\n"),
                6,
                "header",
            ),
            (
                format!("x\n\n{fields}\n| Item | Who | When |\n|---|---|---|\n"),
                8,
                "row",
            ),
            (
                format!("x\n\n{fields}\n{ROW}{}", ROW.lines().last().unwrap()),
                9,
                "a second file",
            ),
        ] {
            let (found, what) = parse("x", &text, &crates()).unwrap_err();
            assert_eq!(found, line, "{text:?}: {what}");
            assert!(what.contains(needle), "{text:?}: `{needle}` not in {what}");
            assert!(
                what.contains("docs/outstanding/README.md has the layout"),
                "{what}"
            );
        }
    }

    #[test]
    fn the_index_lists_every_item_once_by_area_then_name() {
        let texts = [
            ("docs/outstanding/README.md", "anything"),
            (
                "docs/outstanding/b-two.md",
                &*item("Two", "- Area: cortex-tui\n- Owner: step 25\n"),
            ),
            (
                "docs/outstanding/a-one.md",
                &*item("One.", "- Area: cortex-tui\n- Owner: x\n"),
            ),
            (
                "docs/outstanding/odd.md",
                &*item("Odd?", "- Area: other\n- Owner: the owner\n"),
            ),
            (
                "docs/outstanding/tool.md",
                &*item("Tool.", "- Area: tooling\n- Owner: y\n"),
            ),
            (
                "docs/outstanding/core.md",
                &*item("Core.", "- Area: cortex-core\n- Owner: z\n"),
            ),
            ("docs/outstanding/bad.md", "not an item"),
            ("docs/steps/07/answered.md", "elsewhere"),
            ("docs/outstanding/sub/x.md", "not an item's place"),
        ];
        let (items, problems) = read(texts.iter().copied(), &crates());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].0, "docs/outstanding/bad.md");
        let text = index(&items);
        assert!(text.starts_with("# Still outstanding\n"), "{text}");
        let tail = &text[text.find("\n### ").unwrap()..];
        assert_eq!(
            tail,
            "\n### cortex-core\n\n\
             - [core](core.md): Core. Owner: z.\n\
             \n### cortex-tui\n\n\
             - [a-one](a-one.md): One. Owner: x.\n\
             - [b-two](b-two.md): Two. Owner: step 25.\n\
             \n### The tooling and CI\n\n\
             - [tool](tool.md): Tool. Owner: y.\n\
             \n### Other\n\n\
             - [odd](odd.md): Odd? Owner: the owner.\n"
        );
        // The order of the files does not matter.
        let mut reversed = texts;
        reversed.reverse();
        let (again, _) = read(reversed.iter().copied(), &crates());
        assert_eq!(index(&again), text);
    }

    #[test]
    fn a_difference_names_lines_lacking_extra_and_twice() {
        let want = "a\n- [x](x.md): x.\n- [y](y.md): y.\n";
        let lost = difference("a\n- [x](x.md): x.\n", want);
        assert!(lost.contains("It lacks:\n      - [y](y.md): y."), "{lost}");
        let gone = difference(
            "a\n- [x](x.md): x.\n- [y](y.md): y.\n- [z](z.md): z.\n",
            want,
        );
        assert!(
            gone.contains("should not") && gone.contains("[z](z.md)"),
            "{gone}"
        );
        let twice = difference(
            "a\n- [x](x.md): x.\n- [x](x.md): x.\n- [y](y.md): y.\n",
            want,
        );
        assert!(
            twice.contains("It has twice:\n      - [x](x.md): x."),
            "{twice}"
        );
        let order = difference("a\n- [y](y.md): y.\n- [x](x.md): x.\n", want);
        assert!(order.contains("another order"), "{order}");
    }

    /// A rebase that meets two sides' new items goes through: git merges
    /// the index line by line, and the rule finds what that gets wrong.
    #[test]
    fn the_index_is_merged_line_by_line() {
        let attributes =
            std::fs::read_to_string(crate::workspace::root().join(".gitattributes")).unwrap();
        assert!(
            attributes
                .lines()
                .any(|line| line.trim() == format!("{} merge=union", super::INDEX)),
            "{attributes}"
        );
    }
}
