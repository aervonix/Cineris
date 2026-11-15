//! `cargo xtask regen`: every generated file that can be rebuilt from the
//! tree, rebuilt.
//!
//! After a rebase that met another branch's changes to a generated file, the
//! right answer is not a merge of the two texts but the file the merged
//! sources produce. Step 18 settled such conflicts by hand in the snapshots,
//! the reference screens and the generated schema; this command is the
//! way that is not by hand: resolve the conflicts in the sources, take either
//! side of each generated file, and run `regen`.
//!
//! It runs the writing mode of each generator there is, one step each, so
//! that all of them run even when one fails:
//!
//! | Step | Writes | By |
//! |---|---|---|
//! | `protocol` | `docs/protocol/` | `schema` |
//! | `configuration` | `docs/configuration/` | `cortex-config`'s generated-files test in its writing mode |
//! | `screens` | the reference screens of the look and their text copies | `tui-screens --write` |
//! | `snapshots` | the view, decoder, request, map and configuration snapshots | the tests of the crates that keep them, with `CORTEX_UPDATE_SNAPSHOTS` |
//!
//! It ends with `links`, which writes nothing: what `regen` cannot write
//! ([`NOT_COVERED`]) it can still check, the open items' index and the step
//! records among them. And it lists the files that differ from `HEAD` in
//! the places it writes, so the diff is looked at as a change to the
//! sources' output and not skimmed.
//!
//! Nothing here needs the network or a key. [`NOT_COVERED`] is the list of
//! what does, with the command for each, and a test holds the two lists to
//! every writing mode (`CORTEX_UPDATE_*`) the code has, so a new one is put
//! on one of them.

use std::process::Command;

use crate::steps::{self, Step};
use crate::{Result, checks, screens, workspace};

/// One generator: what it writes, and how.
#[derive(Debug)]
pub(crate) struct Writer {
    /// The name `regen` is asked for it by.
    pub(crate) name: &'static str,
    /// What it writes, in a few words.
    pub(crate) what: &'static str,
    /// Where it writes, as paths from the repository's root: a file, or a
    /// folder when it ends in `/`.
    pub(crate) writes: &'static [&'static str],
    /// The environment variables that put its tests in their writing mode.
    pub(crate) modes: &'static [&'static str],
    run: fn() -> Result<()>,
}

/// The crates that keep snapshots, and write them under
/// `CORTEX_UPDATE_SNAPSHOTS`.
const SNAPSHOT_CRATES: [&str; 4] = ["cortex-tui", "cortex-llm", "cortex-index", "cortex-config"];

/// Every generator `regen` runs, in the order it runs them.
pub(crate) const WRITERS: [Writer; 4] = [
    Writer {
        name: "protocol",
        what: "the protocol's JSON Schema and TypeScript declarations",
        writes: &["docs/protocol/"],
        modes: &[],
        run: protocol,
    },
    Writer {
        name: "configuration",
        what: "the configuration file's JSON Schema and reference",
        writes: &["docs/configuration/"],
        modes: &["CORTEX_UPDATE_GENERATED"],
        run: configuration,
    },
    // Before the snapshots: a test of `cortex-tui` that they run holds the
    // screens' text copies to the screens.
    Writer {
        name: "screens",
        what: "the reference screens of the TUI's look, and their text copies",
        writes: &["docs/architecture/tui-look/"],
        modes: &["CORTEX_UPDATE_SCREENS"],
        run: reference_screens,
    },
    Writer {
        name: "snapshots",
        what: "the snapshots of the views, the decoders, the requests, the maps and the \
               configuration, and the hand-written provider fixtures' requests",
        writes: &[
            "crates/cortex-tui/snapshots/",
            "crates/cortex-llm/tests/it/snapshots/",
            "crates/cortex-llm/fixtures/",
            "crates/cortex-index/tests/it/snapshots/",
            "crates/cortex-config/tests/it/snapshots/",
        ],
        modes: &["CORTEX_UPDATE_SNAPSHOTS"],
        run: snapshots,
    },
];

/// A file or a kind of file that `regen` does not write, and why.
pub(crate) struct NotCovered {
    pub(crate) what: &'static str,
    pub(crate) why: &'static str,
    /// What writes it, or what to do.
    pub(crate) instead: &'static str,
    /// The writing modes that belong to it, which no [`Writer`] sets.
    pub(crate) modes: &'static [&'static str],
}

/// What `regen` leaves to others.
pub(crate) const NOT_COVERED: [NotCovered; 6] = [
    NotCovered {
        what: "the tables of docs/steps/<step>/ci.md",
        why: "they are measurements of a CI run: a run id, read through `gh`, and a network",
        instead: "cargo xtask ci-times RUN --against main --pr N --write docs/steps/<step>/ci.md",
        modes: &[],
    },
    NotCovered {
        what: "the index of docs/outstanding/README.md",
        why: "each line is prose that the item's file does not hold (the area it is about, what \
              it covers in a sentence, who settles it), so there is nothing to generate it \
              from; `links`, which `regen` ends with, fails on an item left out, listed twice \
              or named for a file that is gone",
        instead: "edit it by hand: after a rebase keep every line of both sides, and delete the \
                  line of an item whose file is gone",
        modes: &[],
    },
    NotCovered {
        what: "the toolbox fixtures and the tools kept beside them",
        why: "the kept tools are what a recording was made with; written alone they would \
              disagree with it, and a recording is a call to a paid API",
        instead: "cargo xtask record --toolbox [--provider PROVIDER], by hand, with a key file",
        modes: &["CORTEX_UPDATE_TOOLBOX"],
    },
    NotCovered {
        what: "the recorded provider fixtures (crates/cortex-llm/fixtures/)",
        why: "they are the provider's own answers, recorded from the real API; the \
              hand-written ones are regenerated by `snapshots`, the recorded ones are never \
              edited",
        instead: "cargo xtask record [--provider PROVIDER] [SCENARIO...], by hand",
        modes: &[],
    },
    NotCovered {
        what: "the golden transcript (crates/cortex-protocol/tests/golden/)",
        why: "it is the contract of a released protocol version, and is not edited to make a \
              test pass",
        instead: "an addition to a released version gets a new golden file",
        modes: &[],
    },
    NotCovered {
        what: "Cargo.lock, and the break lists in mutations/",
        why: "cargo keeps the first, and only a dependency change moves it; the second are \
              written by hand with the code they break",
        instead: "cargo build, and `cargo xtask check breaks` for the lists",
        modes: &[],
    },
];

fn protocol() -> Result<()> {
    crate::schema::write()
}

fn configuration() -> Result<()> {
    crate::generate_configuration()
}

fn snapshots() -> Result<()> {
    checks::require_nextest()?;
    let mut arguments = vec![
        "nextest".to_owned(),
        "run".to_owned(),
        "--locked".to_owned(),
        // Tiers 1 and 2: the snapshot tests are among them, and a test that
        // starts a process has nothing to write.
        "--profile".to_owned(),
        "fast".to_owned(),
        "--no-fail-fast".to_owned(),
    ];
    for package in SNAPSHOT_CRATES {
        arguments.push("-p".to_owned());
        arguments.push(package.to_owned());
    }
    steps::cargo_with(&arguments, &[("CORTEX_UPDATE_SNAPSHOTS", "1")])
}

fn reference_screens() -> Result<()> {
    screens::run(&["--write".to_owned()])
}

/// The writers `names` ask for, in the order they run in: all of them when
/// there are no names.
pub(crate) fn select(names: &[String]) -> Result<Vec<&'static Writer>> {
    if names.is_empty() {
        return Ok(WRITERS.iter().collect());
    }
    for name in names {
        if !WRITERS.iter().any(|writer| writer.name == name) {
            return Err(format!(
                "no generator is called `{name}`. They are {}.\n\n{USAGE}",
                WRITERS.map(|writer| writer.name).join(", ")
            ));
        }
    }
    Ok(WRITERS
        .iter()
        .filter(|writer| names.iter().any(|name| name == writer.name))
        .collect())
}

const USAGE: &str = "\
usage: cargo xtask regen [--list] [GENERATOR...]

Rebuilds every generated file that can be rebuilt from the tree, with no
network and no key: the protocol's schema and TypeScript declarations, the
configuration's schema and reference, every snapshot, and the reference screens.
Use it after a rebase that met changes to a generated file: settle the
conflict in the sources, take either side of the generated file, run this.
Every generator runs even when one fails. With names, only those, in their
order. --list says what each writes and what `regen` leaves to other commands.
It ends by listing the files that differ from HEAD in the places it writes,
and by checking the links, the open items' index and the step records, which
it cannot write.";

/// What `--list` prints.
fn listing() -> String {
    let mut text = String::from("Written by `cargo xtask regen`:\n");
    for writer in &WRITERS {
        text.push_str(&format!("  {}: {}\n", writer.name, writer.what));
        for path in writer.writes {
            text.push_str(&format!("      {path}\n"));
        }
        if !writer.modes.is_empty() {
            text.push_str(&format!(
                "      writing mode: {}\n",
                writer.modes.join(", ")
            ));
        }
    }
    text.push_str("\nNot written by it:\n");
    for left in &NOT_COVERED {
        text.push_str(&format!(
            "  {}\n      why: {}\n      instead: {}\n",
            left.what, left.why, left.instead
        ));
        if !left.modes.is_empty() {
            text.push_str(&format!("      writing mode: {}\n", left.modes.join(", ")));
        }
    }
    text
}

/// `git status --short` of the places the writers in `selected` write, as
/// text. `None` where git does not answer (a copy of the tree with no
/// `.git`).
fn changed(selected: &[&Writer]) -> Option<String> {
    let mut command = Command::new("git");
    command
        .args(["status", "--short", "--"])
        .current_dir(workspace::root());
    for writer in selected {
        command.args(writer.writes.iter().map(|path| path.trim_end_matches('/')));
    }
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn show_changed(selected: &[&Writer]) -> Result<()> {
    match changed(selected) {
        Some(text) if text.trim().is_empty() => {
            println!("Nothing differs from HEAD in the places regen writes.");
        }
        Some(text) => {
            println!("Differs from HEAD (review it like code; `git diff` shows how):\n{text}");
        }
        None => println!("git did not answer here: look at what changed by hand."),
    }
    Ok(())
}

/// `cargo xtask regen [--list] [GENERATOR...]`.
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    if arguments.iter().any(|argument| argument == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if arguments.iter().any(|argument| argument == "--list") {
        if arguments.len() > 1 {
            return Err(format!("`--list` takes nothing else\n\n{USAGE}"));
        }
        print!("{}", listing());
        return Ok(());
    }
    if let Some(flag) = arguments.iter().find(|argument| argument.starts_with('-')) {
        return Err(format!("unknown option `{flag}`\n\n{USAGE}"));
    }
    steps::run("regen", plan(select(arguments)?))
}

/// The steps of a run: each generator, then the list of what differs from
/// `HEAD`, then `links`, which checks what no generator writes.
fn plan(selected: Vec<&'static Writer>) -> Vec<Step> {
    let mut list: Vec<Step> = selected
        .iter()
        .map(|writer| Step::function(writer.name, writer.run))
        .collect();
    list.push(Step::closure("changed", move || show_changed(&selected)));
    list.push(Step::function("links", checks::links));
    list
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{NOT_COVERED, WRITERS, listing, plan, select};
    use crate::workspace;

    fn names(selected: &[&super::Writer]) -> Vec<&'static str> {
        selected.iter().map(|writer| writer.name).collect()
    }

    #[test]
    fn no_name_is_every_generator_and_names_keep_their_order() {
        assert_eq!(
            names(&select(&[]).unwrap()),
            ["protocol", "configuration", "screens", "snapshots"]
        );
        assert_eq!(
            names(&select(&["screens".to_owned(), "protocol".to_owned()]).unwrap()),
            ["protocol", "screens"]
        );
        let error = select(&["schema".to_owned()]).unwrap_err();
        assert!(
            error.contains("`schema`") && error.contains("protocol"),
            "{error}"
        );
    }

    #[test]
    fn a_run_is_the_generators_then_what_changed_then_the_check_of_what_it_cannot_write() {
        let steps = |names: &[&str]| -> Vec<&'static str> {
            let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
            plan(select(&names).unwrap())
                .iter()
                .map(|step| step.name)
                .collect()
        };
        assert_eq!(
            steps(&[]),
            [
                "protocol",
                "configuration",
                "screens",
                "snapshots",
                "changed",
                "links"
            ]
        );
        assert_eq!(steps(&["snapshots"]), ["snapshots", "changed", "links"]);
    }

    #[test]
    fn every_place_regen_writes_is_in_the_repository() {
        let files = workspace::repository_files(&workspace::root()).unwrap();
        for writer in &WRITERS {
            for path in writer.writes {
                assert!(
                    files.iter().any(|file| file.starts_with(path)),
                    "{}: nothing is kept under `{path}`: the generator or the list is out of date",
                    writer.name
                );
            }
        }
    }

    /// Every `CORTEX_UPDATE_*` variable in the code: a way to write a file
    /// from a test, which `regen` runs or says why it does not.
    #[test]
    fn every_writing_mode_in_the_code_is_run_by_regen_or_left_to_another_command() {
        let root = workspace::root();
        let mut found = BTreeSet::new();
        for file in workspace::repository_files(&root).unwrap() {
            if !std::path::Path::new(&file)
                .extension()
                .is_some_and(|extension| extension == "rs")
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(root.join(&file)) else {
                continue;
            };
            let mut rest = text.as_str();
            while let Some(at) = rest.find("CORTEX_UPDATE_") {
                let tail = &rest[at..];
                let end = tail
                    .find(|c: char| !(c.is_ascii_uppercase() || c == '_'))
                    .unwrap_or(tail.len());
                // The bare prefix is how this file and the docs speak of them
                // all.
                if tail[..end].len() > "CORTEX_UPDATE_".len() {
                    found.insert(tail[..end].to_owned());
                }
                rest = &tail[end..];
            }
        }
        let known: BTreeSet<String> = WRITERS
            .iter()
            .flat_map(|writer| writer.modes)
            .chain(NOT_COVERED.iter().flat_map(|left| left.modes))
            .map(|mode| (*mode).to_owned())
            .collect();
        assert_eq!(
            found, known,
            "left: the `CORTEX_UPDATE_*` variables the code reads; right: those `regen` runs \
             (WRITERS) or lists as left to another command (NOT_COVERED). A new writing mode \
             goes on one of the two lists; one that is gone comes off."
        );
    }

    #[test]
    fn the_listing_names_every_generator_and_everything_left_out() {
        let text = listing();
        for writer in &WRITERS {
            assert!(text.contains(writer.name), "{text}");
            for path in writer.writes {
                assert!(text.contains(path), "{text}");
            }
        }
        for left in &NOT_COVERED {
            assert!(
                text.contains(left.what) && text.contains(left.instead),
                "{text}"
            );
        }
    }
}
