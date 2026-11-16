//! `cargo xtask mutate --merge STEP`: the parts of a step's break list,
//! made into the list.
//!
//! A session that hands its work to sub-agents gives each one a list of its
//! own, `mutations/<step>-<part>.breaks`, so that no two of them write one
//! file. At the end this puts them together as `mutations/<step>.breaks`:
//!
//! - The parts are read in name order, and each one's text is kept as it is,
//!   its comments with it, under a comment line that names the part it came
//!   from. A list that is already there is kept, and the parts follow it.
//! - **All or nothing.** Two breaks of one name in the result are refused,
//!   with the files and lines. Then the whole pre-flight is run on what the
//!   lists would be afterwards (the other lists, as they are, and the
//!   merged one): a break that does not apply, one that makes the same change
//!   as another, a `caught-by` that names no test. Only when it passes is the
//!   list written, beside its place and moved over it, and the parts deleted.
//! - **Results kept for the parts' breaks stay.** A result is kept by the
//!   code and the break's own text, never by the name of the list it is in,
//!   so a merged break finds what was learnt for it.
//! - With `--dry-run` it prints what it would do, and does none of it.

use std::fs;
use std::path::Path;

use super::list::{self, DIRECTORY};
use super::{check, names};
use crate::Result;

/// The ending of a list's file.
const SUFFIX: &str = ".breaks";

/// One part: its file's name in `mutations/`, the part's name, its text.
#[derive(Debug)]
struct Part {
    file: String,
    name: String,
    text: String,
}

/// What a merge would do.
#[derive(Debug)]
struct Plan {
    /// `mutations/<step>.breaks`, and its text as it is now when it is there.
    target: String,
    existing: Option<String>,
    parts: Vec<Part>,
    /// The text the list would have.
    merged: String,
}

/// The text of the merged list: what is in the list now, and each part under
/// the comment line that names it. Blank lines between, and one line ending
/// at the end.
fn merged_text(existing: Option<&str>, parts: &[Part]) -> String {
    let mut text = String::new();
    if let Some(existing) = existing {
        text.push_str(existing.trim_end());
        text.push('\n');
    }
    for part in parts {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "# From {DIRECTORY}/{} (part `{}`).\n",
            part.file, part.name
        ));
        text.push_str(part.text.trim_end());
        text.push('\n');
    }
    text
}

/// What to merge for `step`, read from `root`'s `mutations/`.
fn plan(root: &Path, step: &str, named: &[String]) -> Result<Plan> {
    let well_formed = !step.is_empty()
        && step
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !well_formed {
        return Err(format!(
            "`--merge {step}`: a step is named by lower-case letters, digits and `-`, as the \
             list is: `step-12` for `{DIRECTORY}/step-12.breaks`"
        ));
    }
    let directory = root.join(DIRECTORY);
    let unreadable =
        |error: std::io::Error| format!("could not read {}: {error}", directory.display());
    let mut files = Vec::new();
    if directory.is_dir() {
        for entry in fs::read_dir(&directory).map_err(unreadable)? {
            let name = entry
                .map_err(unreadable)?
                .file_name()
                .to_string_lossy()
                .into_owned();
            files.push(name);
        }
    }
    files.sort();
    let mut parts = Vec::new();
    for file in files {
        let Some(name) = file
            .strip_prefix(&format!("{step}-"))
            .and_then(|rest| rest.strip_suffix(SUFFIX))
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let path = directory.join(&file);
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        parts.push(Part {
            name: name.to_owned(),
            file,
            text,
        });
    }
    if parts.is_empty() {
        return Err(format!(
            "no part to merge: there is no {DIRECTORY}/{step}-<part>{SUFFIX}. Each sub-agent's \
             list is named for its step and its part."
        ));
    }
    // Only the parts named are taken. A list of its own can have a name of
    // the same form (`step-12-fixes`, `tooling-2` beside `tooling`), and
    // merging it would rename its breaks and delete it.
    let found: Vec<String> = parts.iter().map(|part| part.name.clone()).collect();
    if named.is_empty() {
        return Err(format!(
            "`--merge {step}` takes the parts it is given by name: `cargo xtask mutate --merge \
             {step} {}`. These lists are named like parts of it: {}. Name only the parts: a list \
             of its own (a step's fixes, another step) named like one is not a part.",
            found.join(" "),
            found
                .iter()
                .map(|name| format!("{DIRECTORY}/{step}-{name}{SUFFIX}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(missing) = named.iter().find(|name| !found.contains(name)) {
        return Err(format!(
            "`--merge {step} {missing}`: there is no {DIRECTORY}/{step}-{missing}{SUFFIX}. The \
             parts there are: {}",
            found.join(", ")
        ));
    }
    parts.retain(|part| named.contains(&part.name));
    let target = format!("{DIRECTORY}/{step}{SUFFIX}");
    let existing = match fs::read_to_string(root.join(&target)) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("could not read {target}: {error}")),
    };
    let merged = merged_text(existing.as_deref(), &parts);
    Ok(Plan {
        target,
        existing,
        parts,
        merged,
    })
}

/// What is wrong with the parts and the list as they would be put together,
/// before the pre-flight: a part that is not a list, and one break name in
/// two places.
fn name_problems(plan: &Plan) -> Vec<String> {
    let mut problems = Vec::new();
    // Each break's name, with where it is: the file and the line.
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut sources: Vec<(String, &str)> = Vec::new();
    if let Some(existing) = &plan.existing {
        sources.push((plan.target.clone(), existing));
    }
    for part in &plan.parts {
        sources.push((format!("{DIRECTORY}/{}", part.file), &part.text));
    }
    for (place, text) in sources {
        match list::parse(&place, text) {
            Err(error) => problems.push(error),
            Ok(breaks) => {
                for one in breaks {
                    let name = one.id.rsplit('/').next().unwrap_or_default().to_owned();
                    let here = format!("{place}:{}", one.line);
                    match seen.iter().find(|(seen, _)| *seen == name) {
                        Some((_, first)) => problems.push(format!(
                            "{here}: break `{name}` is also at {first}: two breaks of one name \
                             cannot be one list. Rename one of them."
                        )),
                        None => seen.push((name, here)),
                    }
                }
            }
        }
    }
    problems
}

/// What the whole pre-flight says of the lists as they would be: every list
/// but the target and the parts, as they are, and the merged text in the
/// target's place.
fn preflight(root: &Path, plan: &Plan) -> Result<Vec<String>> {
    let mut all: Vec<_> = list::load_all(root)?
        .into_iter()
        .filter(|one| {
            one.list != plan.target
                && !plan
                    .parts
                    .iter()
                    .any(|part| one.list == format!("{DIRECTORY}/{}", part.file))
        })
        .collect();
    all.extend(list::parse(&plan.target, &plan.merged)?);
    let mut problems = check(root, &all);
    problems.extend(names::check(root, &all, None)?.problems);
    Ok(problems)
}

/// `--merge STEP`, and with `dry_run` only what it would do.
pub(crate) fn run(root: &Path, step: &str, named: &[String], dry_run: bool) -> Result<()> {
    let plan = plan(root, step, named)?;
    let breaks_of =
        |text: &str, place: &str| list::parse(place, text).map_or(0, |breaks| breaks.len());
    println!(
        "{} {} part(s) into {}{}:",
        if dry_run { "Would merge" } else { "Merging" },
        plan.parts.len(),
        plan.target,
        if plan.existing.is_some() {
            " (which is there, and keeps what it has)"
        } else {
            ""
        }
    );
    for part in &plan.parts {
        println!(
            "  {DIRECTORY}/{}: {} break(s)",
            part.file,
            breaks_of(&part.text, &part.file)
        );
    }
    let mut problems = name_problems(&plan);
    if problems.is_empty() {
        problems = preflight(root, &plan)?;
        if !problems.is_empty() {
            println!(
                "\nIn the merged list, which was not written (its lines are those of the list as it would be):"
            );
        }
    }
    if !problems.is_empty() {
        for problem in &problems {
            println!("{problem}\n");
        }
        return Err(format!(
            "{} problem(s) in the lists to merge. Nothing was written and no part deleted.",
            problems.len()
        ));
    }
    println!(
        "The pre-flight passes on the merged list: {} break(s).",
        breaks_of(&plan.merged, &plan.target)
    );
    if dry_run {
        println!("--dry-run: nothing was written and no part deleted.");
        return Ok(());
    }
    let failed = |what: &str, error: std::io::Error| format!("could not {what}: {error}");
    let target = root.join(&plan.target);
    let beside = target.with_extension("breaks.new");
    fs::write(&beside, &plan.merged)
        .map_err(|error| failed(&format!("write {}", beside.display()), error))?;
    fs::rename(&beside, &target)
        .map_err(|error| failed(&format!("move {} into place", target.display()), error))?;
    let mut left = Vec::new();
    for part in &plan.parts {
        let path = root.join(DIRECTORY).join(&part.file);
        if let Err(error) = fs::remove_file(&path) {
            left.push(format!("{DIRECTORY}/{}: {error}", part.file));
        }
    }
    if !left.is_empty() {
        return Err(format!(
            "{} is written, and these parts could not be deleted, which leaves their breaks \
             twice: delete them by hand.\n  {}",
            plan.target,
            left.join("\n  ")
        ));
    }
    println!(
        "Wrote {} and deleted the {} part(s). Results kept for these breaks are kept: they are \
         found by the code and the break, not the list.",
        plan.target,
        plan.parts.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::run;
    use crate::mutate::drive::tests::tree;

    const LIB: &str = "pub fn over(a: u8, b: u8) -> bool {\n    a > b\n}\n\npub fn under(a: u8, b: u8) -> bool {\n    a < b\n}\n";

    fn one(name: &str, find: &str, replace: &str) -> String {
        format!(
            "[break] {name}\nrule: r\nfile: crates/demo/src/lib.rs\nfind: {find}\nreplace: {replace}\n"
        )
    }

    fn setup(test: &str, lists: &[(&str, String)]) -> PathBuf {
        let mut files = vec![
            (
                "crates/demo/Cargo.toml".to_owned(),
                "[package]\n".to_owned(),
            ),
            ("crates/demo/src/lib.rs".to_owned(), LIB.to_owned()),
        ];
        for (name, text) in lists {
            files.push((format!("mutations/{name}.breaks"), text.clone()));
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        tree(test, &refs)
    }

    fn lists_in(root: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root.join("mutations"))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn the_parts_become_the_list_with_their_comments_and_each_part_named() {
        let root = setup(
            "merge-made",
            &[
                (
                    "step-9-a",
                    format!(
                        "# The first part's note.\n\n{}",
                        one("over-is-strict", "a > b", "a >= b")
                    ),
                ),
                ("step-9-b", one("under-is-strict", "a < b", "a <= b")),
                ("step-10", one("unrelated", "a > b", "a != b")),
            ],
        );
        // Not a list: a file that is not named for a part is left alone.
        fs::write(root.join("mutations/step-9-notes.txt"), "not a list").unwrap();

        run(&root, "step-9", &["a".to_owned(), "b".to_owned()], false).unwrap();

        let merged = fs::read_to_string(root.join("mutations/step-9.breaks")).unwrap();
        assert!(
            merged.starts_with(
                "# From mutations/step-9-a.breaks (part `a`).\n# The first part's note.\n"
            ),
            "{merged}"
        );
        assert!(
            merged.contains(
                "\n# From mutations/step-9-b.breaks (part `b`).\n[break] under-is-strict\n"
            )
        );
        assert!(merged.contains("[break] over-is-strict\nrule: r\n"));
        // The parts are gone, and nothing else was touched.
        assert_eq!(
            lists_in(&root),
            ["step-10.breaks", "step-9-notes.txt", "step-9.breaks"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_list_that_is_there_is_kept_and_the_parts_follow_it() {
        let root = setup(
            "merge-append",
            &[
                ("step-9", one("first", "a > b", "a >= b")),
                ("step-9-fixes", one("second", "a < b", "a <= b")),
            ],
        );

        run(&root, "step-9", &["fixes".to_owned()], false).unwrap();

        let merged = fs::read_to_string(root.join("mutations/step-9.breaks")).unwrap();
        let first = merged.find("[break] first").unwrap();
        let second = merged.find("[break] second").unwrap();
        assert!(first < second, "{merged}");
        assert_eq!(merged.matches("[break]").count(), 2);
        assert_eq!(lists_in(&root), ["step-9.breaks"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn two_breaks_of_one_name_are_refused_with_both_files_and_nothing_is_written() {
        let root = setup(
            "merge-names",
            &[
                ("step-9-a", one("same-name", "a > b", "a >= b")),
                ("step-9-b", one("same-name", "a < b", "a <= b")),
            ],
        );

        let error = run(&root, "step-9", &["a".to_owned(), "b".to_owned()], false).unwrap_err();

        assert!(
            error.contains("Nothing was written and no part deleted"),
            "{error}"
        );
        assert_eq!(lists_in(&root), ["step-9-a.breaks", "step-9-b.breaks"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn two_breaks_that_make_one_change_are_refused_by_the_preflight() {
        let root = setup(
            "merge-same-change",
            &[
                ("step-9-a", one("one", "a > b", "a >= b")),
                ("step-9-b", one("two", "a > b", "a  >=  b")),
            ],
        );

        let error = run(&root, "step-9", &["a".to_owned(), "b".to_owned()], false).unwrap_err();

        assert!(error.contains("1 problem(s)"), "{error}");
        assert_eq!(lists_in(&root), ["step-9-a.breaks", "step-9-b.breaks"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_break_that_does_not_apply_stops_the_merge_and_a_dry_run_changes_nothing() {
        let root = setup(
            "merge-dry",
            &[
                ("step-9-a", one("fine", "a > b", "a >= b")),
                ("step-9-b", one("gone", "a == b", "true")),
            ],
        );
        assert!(run(&root, "step-9", &["a".to_owned(), "b".to_owned()], true).is_err());
        assert!(run(&root, "step-9", &["a".to_owned(), "b".to_owned()], false).is_err());
        assert_eq!(lists_in(&root), ["step-9-a.breaks", "step-9-b.breaks"]);
        let _ = fs::remove_dir_all(&root);

        let root = setup(
            "merge-dry-ok",
            &[
                ("step-9-a", one("fine", "a > b", "a >= b")),
                ("step-9-b", one("also", "a < b", "a <= b")),
            ],
        );
        run(&root, "step-9", &["a".to_owned(), "b".to_owned()], true).unwrap();
        assert_eq!(lists_in(&root), ["step-9-a.breaks", "step-9-b.breaks"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_parts_named_are_merged_and_a_list_of_its_own_is_left_alone() {
        let root = setup(
            "merge-named",
            &[
                ("step-9", one("first", "a > b", "a >= b")),
                // A list of its own, named like a part.
                ("step-9-fixes", one("fixed", "a < b", "a <= b")),
                ("step-9-api", one("api", "a < b", "a > b")),
            ],
        );
        // With no part named, nothing is merged: the candidates are said.
        let error = run(&root, "step-9", &[], false).unwrap_err();
        assert!(
            error.contains("takes the parts it is given by name"),
            "{error}"
        );
        assert!(error.contains("mutations/step-9-fixes.breaks"), "{error}");
        let error = run(&root, "step-9", &["nope".to_owned()], false).unwrap_err();
        assert!(
            error.contains("there is no mutations/step-9-nope.breaks"),
            "{error}"
        );
        assert_eq!(
            lists_in(&root),
            ["step-9-api.breaks", "step-9-fixes.breaks", "step-9.breaks"]
        );

        run(&root, "step-9", &["api".to_owned()], false).unwrap();
        let merged = fs::read_to_string(root.join("mutations/step-9.breaks")).unwrap();
        assert!(merged.contains("[break] api"), "{merged}");
        assert!(!merged.contains("[break] fixed"), "{merged}");
        assert_eq!(lists_in(&root), ["step-9-fixes.breaks", "step-9.breaks"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_step_with_no_parts_and_a_step_badly_named_are_refused() {
        let root = setup("merge-none", &[("step-9", one("only", "a > b", "a >= b"))]);

        let error = run(&root, "step-9", &["a".to_owned()], false).unwrap_err();
        assert!(error.contains("no part to merge"), "{error}");
        let error = run(&root, "Step 9", &["a".to_owned()], false).unwrap_err();
        assert!(
            error.contains("lower-case letters, digits and `-`"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
