//! The layout of the step records and the open items (`docs/steps/` and
//! `docs/outstanding/`, rule 14 of the build plan).
//!
//! Each step writes its records in a folder of its own, and each open item
//! is a file of its own, so that two steps written at the same time never
//! edit the same lines. That only holds while every step puts its files
//! where the others expect them, under the names and headings they expect:
//!
//! - `docs/steps/<step>/answered.md` and `docs/steps/<step>/ci.md`, and
//!   the records of a step's parts, `docs/steps/<step>/<group>/<part>.md`
//!   (since the tenth tooling step), and nothing else in a step's folder.
//!   The folder is the step's number in two digits with its letter if it
//!   has one (`07`, `13a`), or, for work that is not a numbered step, a
//!   name in lower-case words joined by hyphens (`tooling-w5`).
//! - The first line of `answered.md` is `# Answered in step 7, on
//!   2026-11-13`, and of `ci.md` is `# After step 7: the agent loop`, with
//!   the step the folder is named for. A folder with a name has the same
//!   headings with its own words in place of `step 7`. A part's record
//!   begins with a heading of its own, and the step's `answered.md` or
//!   `ci.md` links it, or its group's folder.
//! - `docs/outstanding/<item>.md`, named in lower-case words joined by
//!   hyphens, is the item's sentence, its fields and its table of one row
//!   ([`crate::outstanding`]), and the index `docs/outstanding/README.md`
//!   is what `cargo xtask outstanding` writes from the items.

use std::collections::{BTreeMap, BTreeSet};

use super::Violation;
use crate::outstanding::{self, FOLDER as OUTSTANDING, INDEX as OUTSTANDING_PAGE, is_hyphenated};

const STEPS: &str = "docs/steps/";

/// Where the layout is described, for the messages.
const STEPS_PAGE: &str = "docs/steps/README.md";

/// Whether a file is one this rule looks at: anything under the two
/// directories, whatever its ending.
pub(super) fn is_read(file: &str) -> bool {
    file.starts_with(STEPS) || file.starts_with(OUTSTANDING)
}

/// Checks every file under the two directories. `texts` holds each file's
/// text by its path from the repository's root; `names` are the
/// repository's files, which say what the crates are.
pub(crate) fn check(texts: &BTreeMap<String, String>, names: &BTreeSet<String>) -> Vec<Violation> {
    let crates = outstanding::crates_of(names);
    let mut violations = Vec::new();
    let mut items_read = true;
    for (file, text) in texts {
        let problem = if let Some(rest) = file.strip_prefix(STEPS) {
            step_record(rest, text, texts)
        } else if let Some(rest) = file.strip_prefix(OUTSTANDING) {
            open_item(rest, text, &crates)
        } else {
            None
        };
        if let Some((line, problem)) = problem {
            if file.starts_with(OUTSTANDING) {
                items_read = false;
            }
            violations.push(Violation::new(file, line, problem));
        }
    }
    let any_item = texts.keys().any(|file| file.starts_with(OUTSTANDING));
    match texts.get(OUTSTANDING_PAGE) {
        Some(found) if items_read => violations.extend(stale_index(texts, &crates, found)),
        Some(_) => violations.push(Violation::new(
            OUTSTANDING_PAGE,
            None,
            "the index is checked once every item's file can be read (above).",
        )),
        None if any_item && items_read => violations.push(Violation::new(
            OUTSTANDING_PAGE,
            None,
            "the index of the open items is not there (or git does not track it). Run `cargo \
             xtask outstanding`, which writes it, and `git add` it.",
        )),
        None => {}
    }
    violations
}

/// The index of [`OUTSTANDING_PAGE`], when it is not what `cargo xtask
/// outstanding` writes from the items git tracks. Whatever a merge of its
/// two sides got wrong (a line twice, a line lost, a line of an item that
/// is gone, an order that is not the index's) is found here, since the
/// whole page is compared.
fn stale_index(
    texts: &BTreeMap<String, String>,
    crates: &BTreeSet<String>,
    found: &str,
) -> Option<Violation> {
    let (items, _) = outstanding::read(
        texts
            .iter()
            .map(|(file, text)| (file.as_str(), text.as_str())),
        crates,
    );
    let expected = outstanding::index(&items);
    (found != expected).then(|| {
        Violation::new(
            OUTSTANDING_PAGE,
            None,
            format!(
                "the index is not what the items' files give:{}\n    Run `cargo xtask \
                 outstanding`, which writes it (after a rebase too: git merges this page's two \
                 sides line by line, and the command puts the result right).",
                outstanding::difference(found, &expected)
            ),
        )
    })
}

/// A step's folder name as its headings write the step: `07` is `step 7`,
/// `13a` is `step 13a`. `None` for a folder that is not a numbered step.
fn step_of(folder: &str) -> Option<String> {
    let digits = folder.get(..2)?;
    let letter = folder.get(2..)?;
    let numbered = digits.chars().all(|c| c.is_ascii_digit())
        && letter.len() <= 1
        && letter.chars().all(|c| c.is_ascii_lowercase());
    let number: u32 = digits.parse().ok()?;
    numbered.then(|| format!("step {number}{letter}"))
}

/// Whether `text` is a date as the records write one: `2026-11-13`.
fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    matches!(parts.as_slice(), [year, month, day]
        if year.len() == 4 && month.len() == 2 && day.len() == 2
            && parts.iter().all(|part| part.chars().all(|c| c.is_ascii_digit())))
}

/// What is wrong with a file under `docs/steps/`, given its path from
/// there: the line it is at, if it is at one, and what to do.
fn step_record(
    path: &str,
    text: &str,
    texts: &BTreeMap<String, String>,
) -> Option<(Option<usize>, String)> {
    if path == "README.md" {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    let (folder, file) = match parts.as_slice() {
        [folder, file] => (*folder, *file),
        [folder, _, _] => (*folder, ""),
        _ => {
            return Some((
                None,
                format!(
                    "a step's records are `{STEPS}<step>/answered.md`, `{STEPS}<step>/ci.md` and \
                     a part's `{STEPS}<step>/<group>/<part>.md`, and this is none of them.\n    \
                     {STEPS_PAGE} has the layout."
                ),
            ));
        }
    };
    let step = step_of(folder);
    // A name must not look like a number that was written wrongly: `7`,
    // `007`, `13A`.
    let named = is_hyphenated(folder) && !folder.starts_with(|c: char| c.is_ascii_digit());
    if step.is_none() && !named {
        return Some((
            None,
            format!(
                "`{folder}` is not a step's folder.\n    A folder is the step's number in two \
                 digits with its letter if it has one (`07`, `13a`), so that the folders sort \
                 in step order; or, for work that is not a numbered step, a name in lower-case \
                 words joined by hyphens. {STEPS_PAGE} has the layout."
            ),
        ));
    }
    if let [_, group, part] = parts.as_slice() {
        return part_record(folder, group, part, text, texts);
    }
    let (begins, then) = match file {
        "answered.md" => ("# Answered in ", ", on "),
        "ci.md" => ("# After ", ": "),
        _ => {
            return Some((
                None,
                format!(
                    "a step's folder holds `answered.md` (what the owner answered) and `ci.md` \
                     (the step's CI times), and nothing else.\n    An item left open is a file \
                     in `{OUTSTANDING}`. {STEPS_PAGE} has the layout."
                ),
            ));
        }
    };

    let heading = text.lines().next().unwrap_or("");
    let expected = match (&step, file) {
        (Some(step), "answered.md") => format!("# Answered in {step}, on <date>"),
        (Some(step), _) => format!("# After {step}: <what it built>"),
        (None, "answered.md") => "# Answered in <the work's name>, on <date>".to_owned(),
        (None, _) => "# After <the work's name>: <what it built>".to_owned(),
    };
    let well_formed = heading
        .strip_prefix(begins)
        .and_then(|rest| rest.split_once(then))
        .is_some_and(|(who, rest)| {
            let who_fits = match &step {
                Some(step) => who == step,
                None => !who.trim().is_empty(),
            };
            let rest_fits = if file == "answered.md" {
                is_date(rest)
            } else {
                !rest.trim().is_empty()
            };
            who_fits && rest_fits
        });
    if well_formed {
        return None;
    }
    Some((
        Some(1),
        format!(
            "the first line is `{heading}`, and for this file it is `{expected}`.\n    The \
             heading says which step the record is of, in the form every other record has, so \
             that the records read in order as one page. {STEPS_PAGE} has the layout."
        ),
    ))
}

/// What is wrong with a part's record, `<step>/<group>/<part>.md`: a step
/// done in several pull requests gives each part a file of its own, so that
/// two parts never append to one record, and the step's `answered.md` (or
/// `ci.md`) links each one, so that reading the step's records in order
/// still reaches every part.
fn part_record(
    folder: &str,
    group: &str,
    part: &str,
    text: &str,
    texts: &BTreeMap<String, String>,
) -> Option<(Option<usize>, String)> {
    let name = part.strip_suffix(".md");
    if !is_hyphenated(group) || !name.is_some_and(is_hyphenated) {
        return Some((
            None,
            format!(
                "a part's record is `{STEPS}<step>/<group>/<part>.md`, the group and the part \
                 each named in lower-case words joined by hyphens (`after-review/pr2-trust.md`).\n    \
                 {STEPS_PAGE} has the layout."
            ),
        ));
    }
    let heading = text.lines().next().unwrap_or("");
    if heading
        .strip_prefix("# ")
        .is_none_or(|rest| rest.trim().is_empty())
    {
        return Some((
            Some(1),
            format!(
                "the first line is `{heading}`, and a part's record begins with a heading of \
                 its own: `# <the step>, <the part>`.\n    {STEPS_PAGE} has the layout."
            ),
        ));
    }
    // The part itself, or its group's folder: a step's own session links
    // the folder once, and its parts, written at the same time, then add
    // nothing to a shared file.
    let group_path = format!("{STEPS}{folder}/{group}");
    let part_path = format!("{group_path}/{part}");
    let linked = ["answered.md", "ci.md"].iter().any(|record| {
        let record = format!("{STEPS}{folder}/{record}");
        texts.get(&record).is_some_and(|text| {
            let targets = super::links::targets(&record, text);
            targets.contains(&part_path) || targets.contains(&group_path)
        })
    });
    if linked {
        return None;
    }
    Some((
        None,
        format!(
            "no record of the step links this part: add a link to its group, `[…]({group}/)`, \
             to `{STEPS}{folder}/answered.md` (or `ci.md`) once, or to the part itself, so that \
             the step's records lead to every part.\n    {STEPS_PAGE} has the layout."
        ),
    ))
}

/// What is wrong with a file under `docs/outstanding/`.
fn open_item(path: &str, text: &str, crates: &BTreeSet<String>) -> Option<(Option<usize>, String)> {
    if path == "README.md" {
        return None;
    }
    let name = path.strip_suffix(".md");
    let Some(name) = name.filter(|name| !path.contains('/') && is_hyphenated(name)) else {
        return Some((
            None,
            format!(
                "an open item is one file in `{OUTSTANDING}`, named for the item in lower-case \
                 words joined by hyphens, ending `.md`.\n    {OUTSTANDING_PAGE} has the layout."
            ),
        ));
    };
    outstanding::parse(name, text, crates)
        .err()
        .map(|(line, message)| (Some(line), message))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::check;
    use crate::outstanding;

    fn names() -> BTreeSet<String> {
        [
            "crates/cineris-tui/Cargo.toml",
            "crates/cineris-cli/Cargo.toml",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect()
    }

    fn found(files: &[(&str, &str)]) -> Vec<String> {
        let texts: BTreeMap<String, String> = files
            .iter()
            .map(|(file, text)| ((*file).to_owned(), (*text).to_owned()))
            .collect();
        check(&texts, &names())
            .into_iter()
            .map(|violation| violation.to_string())
            .collect()
    }

    fn assert_one(found: &[String], needles: &[&str]) {
        assert_eq!(found.len(), 1, "{found:#?}");
        for needle in needles {
            assert!(
                found[0].contains(needle),
                "`{needle}` is not in:\n{}",
                found[0]
            );
        }
    }

    fn item(summary: &str, area: &str) -> String {
        format!(
            "{summary}\n\n- Area: {area}\n- Owner: step 13\n\n| Item | Who | When |\n\
             |---|---|---|\n| **A flag.** It has a recommendation. | Step 13 | In that step |\n"
        )
    }

    /// The index `cargo xtask outstanding` writes for these items.
    fn index_of(items: &[(&str, &str)]) -> String {
        let crates = outstanding::crates_of(&names());
        let (items, problems) = outstanding::read(items.iter().copied(), &crates);
        assert!(problems.is_empty(), "{problems:?}");
        outstanding::index(&items)
    }

    #[test]
    fn an_index_that_is_not_what_the_items_give_is_found() {
        let flag = item("A flag.", "cineris-tui");
        let quick = item("The engine.", "other");
        let items = [
            ("docs/outstanding/a-flag.md", flag.as_str()),
            ("docs/outstanding/quickjs.md", quick.as_str()),
        ];
        let right = index_of(&items);
        let with = |index: &str| {
            let mut files = items.to_vec();
            files.push(("docs/outstanding/README.md", index));
            found(&files)
        };
        assert_eq!(with(&right), [] as [&str; 0]);

        let flag_line = "- [a-flag](a-flag.md): A flag. Owner: step 13.\n";
        assert!(right.contains(flag_line), "{right}");
        // What a merge of two sides line by line can leave: a line lost,
        // a line twice, the line of an item whose file is gone, an order
        // that is not the index's.
        let lost = right.replace(flag_line, "");
        let twice = right.replace(flag_line, &format!("{flag_line}{flag_line}"));
        let gone = right.replace(
            flag_line,
            &format!("{flag_line}- [gone](gone.md): Gone. Owner: x.\n"),
        );
        let quick_line = "- [quickjs](quickjs.md): The engine. Owner: step 13.\n";
        let moved = right
            .replace(quick_line, "")
            .replace(flag_line, &format!("{quick_line}{flag_line}"));
        for (index, needle) in [
            (lost, "It lacks:\n      - [a-flag](a-flag.md)"),
            (twice, "It has twice:\n      - [a-flag](a-flag.md)"),
            (gone, "should not"),
            (moved, "another order"),
            (String::new(), "It lacks"),
        ] {
            let found = with(&index);
            assert_one(
                &found,
                &[
                    "docs/outstanding/README.md: the index is not what",
                    needle,
                    "Run `cargo xtask outstanding`",
                ],
            );
        }
    }

    #[test]
    fn the_index_waits_for_items_that_cannot_be_read() {
        let flag = item("A flag.", "cineris-tui");
        let right = index_of(&[("docs/outstanding/a-flag.md", flag.as_str())]);
        let found = found(&[
            ("docs/outstanding/a-flag.md", flag.as_str()),
            ("docs/outstanding/broken.md", "| Item | Who | When |\n"),
            ("docs/outstanding/README.md", right.as_str()),
        ]);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found[1].starts_with("docs/outstanding/README.md: the index is checked once"));
        assert!(
            found[0].starts_with("docs/outstanding/broken.md:1:"),
            "{found:#?}"
        );
    }

    #[test]
    fn records_and_items_in_their_places_pass() {
        let flag = item("A flag.", "cineris-cli");
        let engine = item("The engine.", "other");
        let index = index_of(&[
            (
                "docs/outstanding/speed-flag-for-headless-mode.md",
                flag.as_str(),
            ),
            ("docs/outstanding/quickjs-2.md", engine.as_str()),
        ]);
        assert_eq!(
            found(&[
                ("docs/steps/README.md", "# Step records\n"),
                (
                    "docs/steps/07/answered.md",
                    "# Answered in step 7, on 2026-11-13\n\nText, and [the trust \
                     part](after-review/pr2-trust.md#what-changed).\n"
                ),
                ("docs/steps/07/ci.md", "# After step 7: the agent loop\n"),
                (
                    "docs/steps/07/after-review/pr2-trust.md",
                    "# Step 7, after the review: trust\n"
                ),
                (
                    "docs/steps/13a/answered.md",
                    "# Answered in step 13a, on 2026-11-14\n"
                ),
                (
                    "docs/steps/13a/ci.md",
                    "# After step 13a: core assembly\n\n[Its times](parts/two.md)\n"
                ),
                ("docs/steps/13a/parts/two.md", "# Step 13a's second part\n"),
                (
                    "docs/steps/tooling-w5/answered.md",
                    "# Answered in the wave 5 tooling step, on 2026-11-13\n"
                ),
                (
                    "docs/steps/tooling-w5/ci.md",
                    "# After the wave 5 tooling step: developer tooling\n"
                ),
                ("docs/outstanding/README.md", &index),
                ("docs/outstanding/speed-flag-for-headless-mode.md", &flag),
                ("docs/outstanding/quickjs-2.md", &format!("{engine}\n\n")),
                // Nothing here is this rule's.
                ("docs/build-plan.md", "no heading at all\n"),
            ]),
            [] as [&str; 0]
        );
    }

    #[test]
    fn a_folder_that_is_not_a_steps_is_refused() {
        for folder in ["7", "007", "13A", "Step-07", "step_07", "07-loop", "1a"] {
            let file = format!("docs/steps/{folder}/ci.md");
            let found = found(&[(&file, "# After step 7: the agent loop\n")]);
            assert_one(&found, &[&format!("`{folder}` is not a step's folder")]);
        }
        let found = found(&[("docs/steps/7/parts/two.md", "# Two\n")]);
        assert_one(&found, &["`7` is not a step's folder"]);
    }

    #[test]
    fn a_steps_folder_holds_its_two_records_and_its_parts_and_nothing_else() {
        assert_one(
            &found(&[("docs/steps/07/notes.md", "# Notes\n")]),
            &["holds `answered.md`", "`docs/outstanding/`"],
        );
        assert_one(
            &found(&[("docs/steps/07/ci/runs/run-1.md", "# After step 7: x\n")]),
            &["none of them"],
        );
        assert_one(
            &found(&[("docs/steps/notes.md", "# Notes\n")]),
            &["none of them"],
        );
    }

    #[test]
    fn a_parts_record_is_named_headed_and_linked() {
        let answered = "# Answered in step 7, on 2026-11-13\n\n[Two](parts/two.md)\n";
        for (part, text, needle) in [
            ("parts/Two.md", "# Two\n", "lower-case words"),
            ("Parts/two.md", "# Two\n", "lower-case words"),
            ("parts/two.txt", "# Two\n", "lower-case words"),
            ("parts/two.md", "Two\n", "a heading of its own"),
            ("parts/two.md", "# \n", "a heading of its own"),
            ("parts/two.md", "## Two\n", "a heading of its own"),
        ] {
            let file = format!("docs/steps/07/{part}");
            let found = found(&[(&file, text), ("docs/steps/07/answered.md", answered)]);
            assert_one(&found, &[&file, needle]);
        }
        // Linked from nowhere, from another step, or only in code.
        for (record, text) in [
            (
                "docs/steps/07/answered.md",
                "# Answered in step 7, on 2026-11-13\n",
            ),
            (
                "docs/steps/08/answered.md",
                "# Answered in step 8, on 2026-11-13\n\n[Two](../07/parts/two.md)\n",
            ),
            (
                "docs/steps/07/answered.md",
                "# Answered in step 7, on 2026-11-13\n\n```text\n[Two](parts/two.md)\n```\n",
            ),
            (
                "docs/steps/07/answered.md",
                "# Answered in step 7, on 2026-11-13\n\n[Three](parts/two.md.old)\n",
            ),
        ] {
            let found = found(&[("docs/steps/07/parts/two.md", "# Two\n"), (record, text)]);
            assert_one(
                &found,
                &[
                    "docs/steps/07/parts/two.md",
                    "no record of the step links this part",
                ],
            );
        }
    }

    #[test]
    fn a_link_to_a_parts_group_covers_every_part_in_it() {
        let heading = "# Answered in step 7, on 2026-11-13\n\n";
        for link in [
            "[After the review](after-review/)",
            "[After the review](after-review)",
            "[After the review](./after-review/)",
            "[One](./after-review/one.md)",
            "[one]: after-review/one.md",
        ] {
            let answered = format!("{heading}{link}\n");
            let found = found(&[
                ("docs/steps/07/answered.md", &answered),
                ("docs/steps/07/after-review/one.md", "# One\n"),
                ("docs/steps/07/after-review/two.md", "# Two\n"),
            ]);
            let wanted: &[&str] = if link.contains("one.md") {
                // A link to one part covers that part alone.
                &["docs/steps/07/after-review/two.md"]
            } else {
                &[]
            };
            assert_eq!(found.len(), wanted.len(), "{link}: {found:#?}");
            for (found, file) in found.iter().zip(wanted) {
                assert!(found.starts_with(file), "{link}: {found}");
            }
        }
        // In a code span, or to another group, it covers nothing.
        for link in ["`[After](after-review/)`", "[Other](other-group/)"] {
            let answered = format!("{heading}{link}\n");
            let found = found(&[
                ("docs/steps/07/answered.md", &answered),
                ("docs/steps/07/after-review/one.md", "# One\n"),
            ]);
            assert_one(&found, &["no record of the step links this part"]);
        }
    }

    #[test]
    fn items_with_no_index_are_found() {
        let flag = item("A flag.", "cineris-tui");
        assert_one(
            &found(&[("docs/outstanding/a-flag.md", &flag)]),
            &[
                "docs/outstanding/README.md",
                "is not there",
                "cargo xtask outstanding",
            ],
        );
    }

    #[test]
    fn a_records_heading_names_its_own_step_in_the_form_the_others_have() {
        for (file, heading, expected) in [
            // Another step's heading.
            (
                "docs/steps/07/answered.md",
                "# Answered in step 8, on 2026-11-13",
                "`# Answered in step 7, on <date>`",
            ),
            // The folder's spelling, not the step's.
            (
                "docs/steps/07/answered.md",
                "# Answered in step 07, on 2026-11-13",
                "`# Answered in step 7, on <date>`",
            ),
            (
                "docs/steps/07/answered.md",
                "# Answered in step 7, on 13 November",
                "`# Answered in step 7, on <date>`",
            ),
            (
                "docs/steps/07/answered.md",
                "## Answered in step 7, on 2026-11-13",
                "`# Answered in step 7, on <date>`",
            ),
            (
                "docs/steps/13a/ci.md",
                "# After step 13: core assembly",
                "`# After step 13a: <what it built>`",
            ),
            (
                "docs/steps/07/ci.md",
                "# After step 7",
                "`# After step 7: <what it built>`",
            ),
            (
                "docs/steps/07/ci.md",
                "# Answered in step 7, on 2026-11-13",
                "`# After step 7: <what it built>`",
            ),
            (
                "docs/steps/tooling-w5/answered.md",
                "# Tooling",
                "`# Answered in <the work's name>, on <date>`",
            ),
            (
                "docs/steps/07/ci.md",
                "",
                "`# After step 7: <what it built>`",
            ),
        ] {
            let found = found(&[(file, &format!("{heading}\n\nText.\n"))]);
            assert_one(&found, &[&format!("{file}:1:"), expected]);
        }
    }

    #[test]
    fn an_open_item_is_named_in_hyphenated_words() {
        let flag = item("A flag.", "cineris-tui");
        for file in [
            "docs/outstanding/Speed-flag.md",
            "docs/outstanding/speed_flag.md",
            "docs/outstanding/speed flag.md",
            "docs/outstanding/speed-flag.txt",
            "docs/outstanding/step-13/speed-flag.md",
            "docs/outstanding/-speed.md",
        ] {
            assert_one(
                &found(&[(file, &flag)]),
                &["lower-case words joined by hyphens"],
            );
        }
    }

    #[test]
    fn an_open_item_out_of_form_is_found_at_its_line() {
        let row = "| **A flag.** | Step 13 | In that step |";
        let fields = "- Area: cineris-tui\n- Owner: step 13\n";
        for (text, line) in [
            // Two items in one file.
            (
                format!(
                    "A flag.\n\n{fields}\n| Item | Who | When |\n|---|---|---|\n{row}\n{row}\n"
                ),
                9,
            ),
            // The form before the tenth tooling step.
            (format!("| Item | Who | When |\n|---|---|---|\n{row}\n"), 1),
            (
                "A flag.\n\n- Owner: step 13\n\n| Item | Who | When |\n".to_owned(),
                4,
            ),
            (String::new(), 1),
        ] {
            let found = found(&[("docs/outstanding/speed-flag.md", &text)]);
            assert_one(
                &found,
                &[
                    &format!("docs/outstanding/speed-flag.md:{line}:"),
                    "docs/outstanding/README.md has the layout",
                ],
            );
        }
    }
}
