//! `cargo xtask mutate`: break the code on purpose, one rule at a time, and
//! see which tests fail (`docs/testing-and-ci.md`, "Breaking the code on
//! purpose").
//!
//! A test that passes says little until it has been seen to fail when the
//! rule it tests is broken. The breaks are data, in `mutations/` (the format
//! is in [`list`]), and this runs them:
//!
//! 1. **Every break is applied to the tree as it is, in memory.** One that
//!    does not apply stops the run before anything is built: its code has
//!    moved, and a break that is not made must never be reported as one no
//!    test caught, or as one a test did. `cargo xtask check` runs this part
//!    alone on every pull request.
//! 2. **The work is done on a copy,** under the target directory, with a
//!    build directory of its own. The working tree is only read.
//! 3. **The tests are run once with nothing broken.** A test that fails by
//!    itself cannot be evidence about a break.
//! 4. **Each break is compiled** before any break's tests are run, and the
//!    ones that do not compile are reported.
//! 5. **Each break's tests are run,** and the tests that failed are read
//!    from nextest's JUnit report. A break no test caught is flagged, and so
//!    is one that the tests named for it did not catch.
//!
//! Every result is written down as it is known, keyed by the code and the
//! break it is for. A run that is stopped, by its own time budget or from
//! outside, carries on from there the next time. The budget is there so that
//! a run through `rx` gives the shared test lock back between slices.

mod budget;
mod changed;
mod drive;
mod fix;
mod jobs;
pub(crate) mod lex;
mod list;
mod merge;
mod names;
mod readers;

pub(crate) use drive::{sync, tree_files};

use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::Result;
use crate::workspace;

use list::Break;

const USAGE: &str = "\
usage: cargo xtask mutate [OPTION]... [BREAK]...

Breaks the code on purpose, one break of mutations/*.breaks at a time, on a
copy of the tree, and reports which tests fail. BREAK is a break's name
(`step-12/deny-beats-allow`), a whole list (`step-12`) or the end of a name.
Without one, every break.

  --file PATH    Run the breaks of the list at PATH, which may be anywhere (a
                 review's, kept out of mutations/), and no others. Given more
                 than once, each list. Its name is the file's, and must not be
                 a list's of mutations/.
  --check        Only check that every break still applies, and the pre-flight:
                 no `find` ending in a comma, no `in` naming two items, no two
                 breaks making one change. Runs nothing. Also checks that each
                 `caught-by` names a test that exists, against the lists of
                 tests kept in target/mutate/test-lists.json, which it never
                 builds: with none that is fresh it says so and skips the names.
  --tests        With --check: build the tests of the crates the breaks run
                 (cargo nextest list), keep their lists, and check every
                 `caught-by` against them. BREAK narrows which breaks' names.
                 This is the one that builds.
  --fix-in       With --check: for each break whose `in:` no longer holds its
                 `find`, print the `in:` of the one item that holds it now.
                 Refuses to guess when the code is in several items, in none,
                 or is gone.
  --write        With --fix-in: rewrite those `in:` lines in the lists, and
                 nothing else, then check again.
  --break FILE   Run one break that is in no list: with --find and --replace
                 (and the options below), the same checks, baseline, build and
                 judging as a list's break, and its result kept by its own
                 text, so a second identical run is instant. It is named
                 `one-off/<file>-<hash>`. Nothing in mutations/ is written.
                 Not with break names, a list or --file.
  --find CODE    The code to replace, read by token, there once in FILE (or
                 in --in's item). Needs --break.
  --in-string TEXT
                 In the place of --find: change TEXT inside a string literal,
                 and nothing else of it. Looked for only in the string literals
                 of FILE (or --in's item), as they are written in the source,
                 and there exactly once (or --occurrence). --replace is the
                 text to put in its place. Needs --break.
  --replace CODE What to put there (may be empty). Needs --break.
  --in ITEM      The item the code is in, as a list's `in:` (`Round::spawn`).
  --occurrence N Which match of --find is meant, counted from 1.
  --packages A,B The crates whose tests to run (the crate of FILE if none).
  --test NAME    A test that must fail, as a list's `caught-by` entry; given
                 more than once, each. Without one any failing test catches
                 the break.
  --flagged      Run again only the breaks the last run left without a verdict
                 for this code: COULD NOT BE JUDGED (a stall included) and
                 DOES NOT COMPILE. Says which it chose and why; with none it
                 says so and exits 0. BREAK names narrow the choice. Breaks
                 with a verdict that is flagged (NOT CAUGHT ...) are not
                 chosen: run them with --again.
  --merge STEP PART...
                 Merge mutations/STEP-PART.breaks (the lists of a step's
                 sub-agents), for each PART named, into mutations/STEP.breaks,
                 each part under a comment line naming it, after the whole
                 pre-flight passes on the result, and delete the parts. All
                 or nothing. With no PART it names the lists that look like
                 parts and merges nothing: a list of its own can have such a
                 name (step-12-fixes).
  --dry-run      With --merge: say what it would do, and do none of it.
  --changed REF  Run only the breaks the working tree's diff from REF (a branch,
                 a tag or a commit; new files count) can affect: one whose
                 file the diff touches, or a test its `caught-by` names, or
                 that is new or changed in its list, or the build's settings
                 changed, or a file that is not Rust that a crate it runs (or
                 one they are built on) names in a string of its Rust (the
                 model catalog: an include_str!, a build script's
                 rerun-if-changed, a path). A file outside the crates, docs/,
                 mutations/ and .github/ that no crate names chooses every
                 break. Says how many it left and why, and chooses a break
                 when it cannot tell (a test name no file has). Needs git's
                 history (a `.git`: run it in a git
                 checkout). Narrows names, --file and --flagged; not
                 with --check, --break or --merge. A full run, without it,
                 is what a report rests on.
  --list         Print the breaks chosen, and stop.
  --report       Print what is known so far for the tree as it is. Runs nothing.
  --minutes N    Stop starting new work after N minutes, and carry on from
                 there the next time (5 when not given; 0 for no limit).
  --budget TIME  Run the most relevant breaks first, and start no new one once
                 TIME (90s, 30m, 1h, 1h30m) is spent; those under way finish.
                 With --changed the order is: a break whose file, or whose own
                 text, the diff touched; then one a test it names was touched;
                 then one a file of a crate it runs was (or a data file such a
                 crate reads); then the rest; each in the lists' order.
                 Without --changed every break is of the last. Each class is a
                 run of its own (its tests with nothing broken, then its
                 breaks), and the compile of every break first is left out.
                 Says how many of each class it reached, and names the breaks
                 it left, with the commands that run them. In place of
                 --minutes, not beside it; with --list, prints the order.
  --jobs N       Run N breaks at a time, each in a copy of the tree and a build
                 directory of its own (2 when not given). Each copy's first
                 build is a whole build. The time budget is for all of them
                 together. A run with nothing broken whose timeouts ended
                 together (a machine that stalled) is run again with one.
  --again        Forget the chosen breaks' results first, and run them again.
                 To carry on after the time ran out, leave it off.
  --no-compile-check
                 Do not compile every break before the first is run.

A break with `os: windows` (or `linux`) is run only on that system. On the other
it is checked and not run, and reported as left to a run there, which is neither
a pass nor a failure: `gh workflow run try --ref BRANCH -f os=windows -f
command=\"mutate NAME\"` runs it on a hosted Windows runner.

Exit status: 0 when every chosen break did what its list expects; 1 when one
did not, or something failed; 2 when the time ran out with work left: run
the same command again; 3 when --changed cannot read the diff (no history of
the tree's own, a reference that is no commit), and nothing was run.";

/// The exit status of a run that stopped at its time budget with work left.
const UNFINISHED: u8 = 2;

/// The exit status of `--changed` that could not read the diff: nothing was
/// run, and that is neither a pass nor a break left uncaught.
const NO_DIFF: u8 = 3;

/// What an error of `--changed`'s diff starts with, so that [`conclusion`]
/// gives it [`NO_DIFF`].
const NO_DIFF_MARK: &str = "--changed could not read the diff: ";

/// The minutes a run may start new work for, when it is not told.
const MINUTES: f64 = 5.0;

/// What `cargo xtask mutate` was asked for.
#[derive(Debug, PartialEq)]
struct Request {
    mode: Mode,
    /// The breaks chosen. Empty: all of them.
    only: Vec<String>,
    /// The time budget. `None`: no limit.
    budget: Option<Duration>,
    again: bool,
    compile_check: bool,
    /// How many breaks run at a time.
    jobs: usize,
    /// `--check --tests`: build the lists of tests that `caught-by` names are
    /// checked against.
    tests: bool,
    /// `--check --fix-in`: offer the `in:` of a break whose item moved.
    fix_in: bool,
    /// `--fix-in --write`: rewrite those lines.
    write: bool,
    /// `--file PATH`: lists read from anywhere, whose breaks are the ones
    /// run (a reviewer's, kept out of `mutations/`).
    files: Vec<std::path::PathBuf>,
    /// `--break` and the options that go with it: one break, in no list.
    one_off: list::OneOff,
    /// `--flagged`: only the breaks the last run left without a verdict.
    flagged: bool,
    /// `--changed REF`: only the breaks the diff from `REF` can affect.
    changed: Option<String>,
    /// `--merge STEP`: put the parts of a step's list together.
    merge: Option<String>,
    /// `--dry-run`, with `--merge`.
    dry_run: bool,
    /// `--budget TIME`, as it was given: the breaks run by relevance, and
    /// the time budget is this (`budget`).
    by_relevance: Option<String>,
    /// `--minutes` was given: it and `--budget` are the same limit.
    minutes_given: bool,
}

/// The breaks of the lists `--file` names, each read as a list of
/// `mutations/` is, under its file's name. A name that is already a list's
/// there is refused: two lists of one name would give two breaks one name.
fn load_files(known: &[Break], files: &[std::path::PathBuf]) -> Result<Vec<Break>> {
    let names: std::collections::BTreeSet<&str> = known
        .iter()
        .filter_map(|one| one.id.split('/').next())
        .collect();
    let mut given: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut breaks = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?;
        let list = list::parse(&file.display().to_string(), &text)?;
        if let Some(one) = list.first()
            && let Some(name) = one.id.split('/').next()
            && names.contains(name)
        {
            return Err(format!(
                "{} has the name of a list of {}/: `{name}`. Name it otherwise.",
                file.display(),
                list::DIRECTORY
            ));
        }
        if let Some(name) = list.first().and_then(|one| one.id.split('/').next())
            && !given.insert(name.to_owned())
        {
            return Err(format!(
                "{} has the name of another list given with `--file`: `{name}`. Two lists of \
                 one name would give two breaks one name: name one otherwise.",
                file.display()
            ));
        }
        breaks.extend(list);
    }
    Ok(breaks)
}

/// How many breaks run at a time when the run does not say. Two: in wave 9,
/// four jobs at the desk stalled its disk, and baselines were lost to
/// timeouts that were the machine's.
const JOBS: usize = 2;

/// The share of the last minute, in percent, in which every task waited on
/// the disk (`full avg60` of `/proc/pressure/io`), from which a run with
/// more than one job starts with one. A review's `--jobs 2` on the desk's
/// side slot lost a baseline to a stall at 66%; one job ran clean.
const PRESSURE_FOR_ONE_JOB: f64 = 10.0;

/// How many jobs a run asked for `asked` starts with, given the disk's
/// pressure now (none where it cannot be read), and what to say of it.
fn jobs_for(asked: usize, pressure: Option<&crate::stall::Pressure>) -> (usize, Option<String>) {
    match pressure {
        Some(pressure) if asked > 1 && pressure.full.1 >= PRESSURE_FOR_ONE_JOB => (
            1,
            Some(format!(
                "The disk is busy: every task waited on it {:.1}% of the last minute \
                 (/proc/pressure/io). Running one job, not {asked}: more would stall the \
                 tests with nothing broken.",
                pressure.full.1
            )),
        ),
        _ => (asked, None),
    }
}

/// The most jobs a run may ask for: each is a whole build directory.
const MOST_JOBS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Run,
    Check,
    List,
    Report,
    /// `--merge`.
    Merge,
}

/// Whether `cargo xtask mutate` would take `arguments`, which it reads and
/// runs nothing: how `try` refuses what it cannot pass on before it builds.
pub(crate) fn check_arguments(arguments: &[String]) -> Result<()> {
    Request::parse(arguments).map(|_| ())
}

impl Request {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            mode: Mode::Run,
            only: Vec::new(),
            budget: Some(Duration::from_secs_f64(MINUTES * 60.0)),
            again: false,
            compile_check: true,
            jobs: JOBS,
            tests: false,
            fix_in: false,
            write: false,
            files: Vec::new(),
            one_off: list::OneOff::default(),
            flagged: false,
            changed: None,
            merge: None,
            dry_run: false,
            by_relevance: None,
            minutes_given: false,
        };
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let mut mode = |mode: Mode| {
                if request.mode == Mode::Run {
                    request.mode = mode;
                    Ok(())
                } else {
                    Err(format!(
                        "`{argument}`: say one of --check, --list, --report and --merge"
                    ))
                }
            };
            match argument.as_str() {
                "--check" => mode(Mode::Check)?,
                "--list" => mode(Mode::List)?,
                "--report" => mode(Mode::Report)?,
                "--again" => request.again = true,
                "--no-compile-check" => request.compile_check = false,
                "--tests" => request.tests = true,
                "--fix-in" => request.fix_in = true,
                "--file" => request.files.push(
                    rest.next()
                        .ok_or_else(|| format!("`--file` needs a list's path\n\n{USAGE}"))?
                        .into(),
                ),
                "--write" => request.write = true,
                "--changed" => {
                    let reference = rest.next().ok_or_else(|| {
                        format!("`--changed` needs a branch, a tag or a commit\n\n{USAGE}")
                    })?;
                    if reference.is_empty() || reference.starts_with('-') {
                        return Err(format!(
                            "`--changed {reference}`: a branch, a tag or a commit, not an \
                             option\n\n{USAGE}"
                        ));
                    }
                    request.changed = Some(reference.clone());
                }
                "--flagged" => request.flagged = true,
                "--dry-run" => request.dry_run = true,
                "--merge" => {
                    let step = rest
                        .next()
                        .ok_or_else(|| format!("`--merge` needs a step's name\n\n{USAGE}"))?;
                    mode(Mode::Merge)?;
                    request.merge = Some(step.clone());
                }
                "--break" | "--find" | "--in-string" | "--replace" | "--in" | "--packages"
                | "--test" | "--occurrence" => {
                    let value = rest
                        .next()
                        .ok_or_else(|| format!("`{argument}` needs a value\n\n{USAGE}"))?
                        .clone();
                    let one = &mut request.one_off;
                    match argument.as_str() {
                        "--break" => one.file = Some(value),
                        "--find" => one.find = Some(value),
                        "--in-string" => one.in_string = Some(value),
                        "--replace" => one.replace = Some(value),
                        "--in" => one.anchor = Some(value),
                        "--test" => one.tests.push(value),
                        "--packages" => one.packages.extend(
                            value
                                .split(',')
                                .map(str::trim)
                                .filter(|name| !name.is_empty())
                                .map(str::to_owned),
                        ),
                        _ => {
                            one.occurrence = Some(
                                value
                                    .parse()
                                    .ok()
                                    .filter(|number: &usize| *number > 0)
                                    .ok_or_else(|| {
                                        format!(
                                            "`--occurrence {value}` is not a number counted \
                                             from 1"
                                        )
                                    })?,
                            );
                        }
                    }
                }
                "--jobs" | "-j" => {
                    let text = rest
                        .next()
                        .ok_or_else(|| format!("`{argument}` needs a number\n\n{USAGE}"))?;
                    request.jobs = text
                        .parse()
                        .ok()
                        .filter(|jobs| (1..=MOST_JOBS).contains(jobs))
                        .ok_or_else(|| {
                            format!("`{argument} {text}`: a number from 1 to {MOST_JOBS}")
                        })?;
                }
                "--budget" => {
                    let text = rest.next().ok_or_else(|| {
                        format!("`--budget` needs a length of time (30m, 90s, 1h)\n\n{USAGE}")
                    })?;
                    request.budget = Some(budget::parse_time(text)?);
                    request.by_relevance = Some(text.trim().to_owned());
                }
                "--minutes" => {
                    request.minutes_given = true;
                    let text = rest
                        .next()
                        .ok_or_else(|| format!("`--minutes` needs a number\n\n{USAGE}"))?;
                    let not_minutes = || format!("`--minutes {text}` is not a number of minutes");
                    let minutes: f64 = text
                        .parse()
                        .ok()
                        .filter(|minutes: &f64| minutes.is_finite() && *minutes >= 0.0)
                        .ok_or_else(not_minutes)?;
                    request.budget = if minutes > 0.0 {
                        // A number too large to be a length of time is refused
                        // like one that is no number.
                        Some(
                            Duration::try_from_secs_f64(minutes * 60.0)
                                .map_err(|_| not_minutes())?,
                        )
                    } else {
                        None
                    };
                }
                "--help" | "-h" => return Err(USAGE.to_owned()),
                option if option.starts_with('-') => {
                    return Err(format!("unknown option `{option}`\n\n{USAGE}"));
                }
                name => request.only.push(name.to_owned()),
            }
        }
        for (given, flag, needs) in [
            (request.tests, "--tests", "--check"),
            (request.fix_in, "--fix-in", "--check"),
            (request.write, "--write", "--fix-in"),
        ] {
            let held = if needs == "--check" {
                request.mode == Mode::Check
            } else {
                request.fix_in
            };
            if given && !held {
                return Err(format!("`{flag}` is for use with `{needs}`\n\n{USAGE}"));
            }
        }
        request.refuse_what_does_not_go_together()?;
        Ok(request)
    }

    /// The options that make no sense beside each other.
    fn refuse_what_does_not_go_together(&self) -> Result<()> {
        let one_off = self.one_off.is_given();
        if one_off {
            if self.one_off.file.is_none() {
                return Err(format!(
                    "`--find`, `--in-string`, `--replace`, `--in`, `--occurrence`, `--packages` \
                     and `--test` are for use with `--break FILE`\n\n{USAGE}"
                ));
            }
            if !self.only.is_empty() || !self.files.is_empty() {
                return Err(format!(
                    "`--break` runs one break of its own: it does not go with a break's or a \
                     list's name, or with `--file`\n\n{USAGE}"
                ));
            }
            if self.flagged {
                return Err(format!(
                    "`--break` and `--flagged` do not go together: a one-off break is run by \
                     giving it again\n\n{USAGE}"
                ));
            }
            if matches!(self.mode, Mode::Merge) {
                return Err(format!("`--merge` goes with no other option\n\n{USAGE}"));
            }
            if self.one_off.find.is_some() && self.one_off.in_string.is_some() {
                return Err(format!(
                    "`--find` and `--in-string` do not go together: `--find` replaces code, \
                     `--in-string` changes text inside a string literal\n\n{USAGE}"
                ));
            }
            if (self.one_off.find.is_none() && self.one_off.in_string.is_none())
                || self.one_off.replace.is_none()
            {
                return Err(format!(
                    "`--break FILE` needs `--find CODE` (or `--in-string TEXT`) and `--replace \
                     CODE`\n\n{USAGE}"
                ));
            }
        }
        if self.changed.is_some() {
            if one_off {
                return Err(format!(
                    "`--changed` chooses among the breaks of lists: `--break` is one break of its \
                     own\n\n{USAGE}"
                ));
            }
            if self.mode == Mode::Merge {
                return Err(format!(
                    "`--merge` goes with no other option but `--dry-run`\n\n{USAGE}"
                ));
            }
            if self.mode == Mode::Check {
                return Err(format!(
                    "`--changed` is for a run, `--list` or `--report`: a check reads every list \
                     whole, since a break that does not apply is wrong whether or not it is run \
                     today\n\n{USAGE}"
                ));
            }
        }
        if self.by_relevance.is_some() {
            if self.minutes_given {
                return Err(format!(
                    "`--budget` and `--minutes` do not go together: both are the run's time \
                     limit (`--budget` also runs the most relevant breaks first)\n\n{USAGE}"
                ));
            }
            if matches!(self.mode, Mode::Check | Mode::Merge) {
                return Err(format!(
                    "`--budget` is for a run, or `--list` to see its order: a check and a merge \
                     run no break\n\n{USAGE}"
                ));
            }
        }
        if self.mode == Mode::Merge {
            let others = !self.files.is_empty()
                || self.flagged
                || self.again
                || self.tests
                || self.fix_in
                || self.changed.is_some();
            if others {
                return Err(format!(
                    "`--merge` goes with no other option but `--dry-run`\n\n{USAGE}"
                ));
            }
        } else if self.dry_run {
            return Err(format!("`--dry-run` is for use with `--merge`\n\n{USAGE}"));
        }
        if self.flagged && !matches!(self.mode, Mode::Run | Mode::List | Mode::Report) {
            return Err(format!(
                "`--flagged` is for a run, `--list` or `--report`: a check reads no results\n\n{USAGE}"
            ));
        }
        Ok(())
    }
}

/// `cargo xtask mutate`
pub(crate) fn main(arguments: &[String]) -> ExitCode {
    let again = arguments.iter().any(|argument| argument == "--again");
    let (status, message) = conclusion(&run(arguments), again);
    if let Some(message) = message {
        eprintln!("xtask: {message}");
    }
    ExitCode::from(status)
}

/// The exit status of a run, and what to say last.
fn conclusion(outcome: &Result<drive::Finished>, again: bool) -> (u8, Option<String>) {
    match outcome {
        Ok(drive::Finished::Complete { flagged: 0 }) => (0, None),
        Ok(drive::Finished::Complete { flagged }) => (
            1,
            Some(format!(
                "{flagged} break(s) did not do what the list expects"
            )),
        ),
        Ok(drive::Finished::Unfinished { left }) => {
            let how = if again {
                // With it, the next run would forget what this one did.
                "Run the same command without `--again` to carry on."
            } else {
                "Run the same command again to carry on."
            };
            (
                UNFINISHED,
                Some(format!(
                    "the time budget ran out with {left} piece(s) of work left. {how}"
                )),
            )
        }
        Err(message) => match message.strip_prefix(NO_DIFF_MARK) {
            Some(why) => (
                NO_DIFF,
                Some(format!(
                    "nothing was run: `--changed` could not read the diff. {why}"
                )),
            ),
            None => (1, Some(message.clone())),
        },
    }
}

fn run(arguments: &[String]) -> Result<drive::Finished> {
    let request = Request::parse(arguments)?;
    let root = workspace::root();
    let done = drive::Finished::Complete { flagged: 0 };
    if let Some(step) = &request.merge {
        merge::run(&root, step, &request.only, request.dry_run)?;
        return Ok(done);
    }
    // A break given with `--break` is the only break there is: no list is
    // read, and nothing of `mutations/` is checked or changed.
    let (all, from_files) = if request.one_off.is_given() {
        (vec![request.one_off.to_break()?], Vec::new())
    } else {
        let mut all = list::load_all(&root)?;
        let from_files = load_files(&all, &request.files)?;
        all.extend(from_files.iter().cloned());
        (all, from_files)
    };

    if request.mode == Mode::Check {
        // The whole list, whatever was chosen: a break that does not apply
        // is wrong whether or not it is run today.
        let mut all = all;
        if request.fix_in {
            let fixes = fix::propose_all(&root, &all);
            for fix in &fixes {
                println!("{}\n", fix.describe());
            }
            if request.write {
                let written = fix::write(&root, &fixes)?;
                if written > 0 {
                    println!("Rewrote {written} `in:` line(s) in {}/.\n", list::DIRECTORY);
                    all = list::load_all(&root)?;
                }
            }
        }
        if request.tests {
            // The names of the breaks chosen; the rest are checked as ever.
            let chosen = choose(&all, &request.only)?;
            report_check(&root, &all, &Names::Built(&chosen))?;
        } else {
            report_check(&root, &all, &Names::Kept)?;
        }
        return Ok(done);
    }
    let mut chosen = if request.files.is_empty() {
        choose(&all, &request.only)?
    } else {
        choose(&from_files, &request.only)?
    };
    // Narrowed to what the diff can affect, before anything asks which of
    // the rest were left without a verdict.
    let mut reasons = BTreeMap::new();
    let mut classes = BTreeMap::new();
    if let Some(reference) = &request.changed {
        let known = list::load_all(&root)?;
        let selection = changed::since(&root, reference, &known, &chosen)
            .map_err(|why| format!("{NO_DIFF_MARK}{why}"))?;
        println!("{}", selection.summary(reference));
        reasons = selection.reasons();
        classes = selection.classes.clone();
        chosen = selection.breaks();
        if chosen.is_empty() {
            println!("Nothing to run: no break is affected by the diff.");
            return Ok(done);
        }
    }
    // Read only when it is needed: a list that does not apply is told before
    // anything asks cargo where the target directory is.
    let mut scratch = None;
    if request.flagged {
        let directory = scratch_directory(&root)?;
        let picked = drive::without_a_verdict(&root, &directory, &chosen)?;
        scratch = Some(directory);
        if picked.is_empty() {
            println!(
                "Nothing is flagged: no break of the {} chosen was left without a verdict by \
                 the last run on this code (COULD NOT BE JUDGED, DOES NOT COMPILE).",
                chosen.len()
            );
            return Ok(done);
        }
        println!(
            "Running again the {} break(s) left without a verdict, of the {} chosen:",
            picked.len(),
            chosen.len()
        );
        for (one, why) in &picked {
            println!("  {}  {why}", one.id);
        }
        chosen = picked.into_iter().map(|(one, _)| one).collect();
    }
    // `--budget`: the most relevant first, stable within a class.
    let class_of = |one: &Break| classes.get(&one.id).copied().unwrap_or(budget::Class::Rest);
    if let Some(time) = &request.by_relevance {
        let ordered = budget::ordered(chosen, class_of);
        println!("{}", budget::order_text(&ordered, time));
        chosen = ordered.into_iter().map(|(one, _)| one).collect();
    }
    if request.mode == Mode::List {
        for chosen in &chosen {
            let only = chosen
                .os
                .map_or_else(String::new, |os| format!("  [{} only]", os.word()));
            println!(
                "{}  ({}){only}\n    {}",
                chosen.id,
                chosen.place(),
                chosen.rule
            );
            if let Some(why) = reasons.get(&chosen.id) {
                println!("    chosen: {why}");
            }
            if request.by_relevance.is_some() {
                println!("    class: {}", class_of(chosen).said());
            }
        }
        println!("{} break(s)", chosen.len());
        return Ok(done);
    }
    if chosen.is_empty() {
        println!("There are no breaks in {}/.", list::DIRECTORY);
        return Ok(done);
    }
    report_check(&root, &all, &Names::Not)?;
    let scratch = match scratch {
        Some(scratch) => scratch,
        None => scratch_directory(&root)?,
    };

    // A break of code that only the other system builds is applied above, as
    // on every system, and left to a run there.
    let (chosen, elsewhere): (Vec<Break>, Vec<Break>) =
        chosen.into_iter().partition(Break::runs_here);
    if let Some(said) = left_elsewhere(&elsewhere) {
        println!("{said}");
    }
    if let Some(refusal) = nothing_run_here(&chosen, &elsewhere) {
        return Err(refusal);
    }
    if chosen.is_empty() {
        return Ok(done);
    }
    let mut say = |line: &str| println!("{line}");
    if request.mode == Mode::Report {
        return drive::report_only(&root, &scratch, &chosen, &mut say);
    }
    crate::checks::require_nextest()?;
    let started = Instant::now();
    let budget = request.budget;
    let pressure = crate::stall::read_pressure().ok();
    let (jobs, note) = jobs_for(request.jobs, pressure.as_ref());
    if let Some(note) = note {
        say(&note);
    }
    let mut runners: Vec<drive::CargoRunner> = (0..jobs)
        .map(|job| drive::CargoRunner::new(&scratch, job))
        .collect();
    let mut runners: Vec<&mut dyn drive::Runner> = runners
        .iter_mut()
        .map(|runner| runner as &mut dyn drive::Runner)
        .collect();
    // A known flake that fails with nothing broken is run again once
    // (`crate::flaky`).
    let (flaky, warning) = crate::flaky::listed(&root);
    if let Some(warning) = warning {
        say(&format!("warning: {warning}"));
    }
    let options = drive::Options {
        again: request.again,
        // With `--budget` the breaks are compiled as each is run: compiling
        // every one first would spend the budget on breaks never reached.
        compile_check: request.compile_check && request.by_relevance.is_none(),
        pause: None,
        flaky,
    };
    let out_of_time = || budget.is_some_and(|budget| budget::spent(started.elapsed(), budget));
    let finished = if request.by_relevance.is_some() {
        let ordered: Vec<(Break, budget::Class)> = chosen
            .iter()
            .map(|one| (one.clone(), class_of(one)))
            .collect();
        let finished =
            budget::run_in_order(&ordered, &mut || out_of_time(), &mut |class, breaks| {
                say(&format!(
                    "\n== {} break(s) {} ==",
                    breaks.len(),
                    class.said()
                ));
                drive::drive_jobs(
                    &root,
                    &scratch,
                    breaks,
                    &options,
                    &mut runners,
                    &mut || out_of_time(),
                    &mut say,
                )
            });
        if finished.is_ok() {
            let reached = drive::with_result(&root, &scratch, &chosen)?;
            let command = format!("cargo xtask mutate {}", shown_arguments(arguments));
            let said = budget::reached_text(&ordered, &reached, &command);
            if !said.is_empty() {
                println!("{said}");
            }
        }
        finished
    } else {
        drive::drive_jobs(
            &root,
            &scratch,
            &chosen,
            &options,
            &mut runners,
            &mut || out_of_time(),
            &mut say,
        )
    };
    if !elsewhere.is_empty() && finished.is_ok() {
        println!(
            "\n{} break(s) were left to a run on the other system, as listed above: they are \
             neither a pass nor a failure of this run.",
            elsewhere.len()
        );
    }
    finished
}

/// `arguments` as they would be typed again: a word with a space or a quote
/// in it is quoted.
fn shown_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|word| {
            if word.is_empty() || word.contains([' ', '\'', '"']) {
                format!("'{}'", word.replace('\'', "'\\''"))
            } else {
                word.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The refusal of a run whose every chosen break is of code only the other
/// system builds: it would run nothing, and an exit of 0 would read as every
/// break caught. `None` when a break is run here, or none was chosen.
fn nothing_run_here(chosen: &[Break], elsewhere: &[Break]) -> Option<String> {
    (chosen.is_empty() && !elsewhere.is_empty()).then(|| {
        format!(
            "every break chosen ({}) is of code only the other system builds, so none was run \
             here: run them there, as said above. This is not a pass.",
            elsewhere.len()
        )
    })
}

/// What to say of the breaks this system does not run, and how to run them
/// where their code is built: `os: windows` on Linux, and the other way
/// round. `None` when there are none.
fn left_elsewhere(elsewhere: &[Break]) -> Option<String> {
    // Every one is for the system that is not this one.
    let first = elsewhere.first()?;
    let os = first.os.unwrap_or(list::Os::Windows);
    let mut said = format!(
        "{} break(s) are of code only {} builds, applied here by the pre-flight and left to a \
         {} run, which is neither a pass nor a failure of this one:\n",
        elsewhere.len(),
        os.name(),
        os.name()
    );
    for one in elsewhere {
        said.push_str(&format!("  {}  ({})\n", one.id, one.place()));
    }
    said.push_str(&format!(
        "Run one on a hosted {} runner: `gh workflow run try --ref <branch> -f os={} -f \
         command=\"mutate {}\"`.\n",
        os.name(),
        os.word(),
        first.id
    ));
    Some(said)
}

/// Where the copy of the tree, its build and the results are kept: under
/// the target directory, which belongs to this checkout alone unless
/// `CARGO_TARGET_DIR` sends several checkouts to one. Then they share the
/// copy, one run at a time: each run brings it up to date first, and
/// results are kept by the code they were found for.
fn scratch_directory(root: &Path) -> Result<PathBuf> {
    Ok(workspace::metadata(root)?.target_directory.join("mutate"))
}

/// The breaks the names choose, in list order. A name chooses a break by
/// its whole id, a list by its name, or every break whose id ends with it.
fn choose(all: &[Break], names: &[String]) -> Result<Vec<Break>> {
    if names.is_empty() {
        return Ok(all.to_vec());
    }
    let mut chosen: Vec<&Break> = Vec::new();
    for name in names {
        let matching: Vec<&Break> = all
            .iter()
            .filter(|one| {
                one.id == *name
                    || one.id.split('/').next() == Some(name.as_str())
                    || one.id.ends_with(&format!("/{name}"))
            })
            .collect();
        if matching.is_empty() {
            return Err(format!(
                "no break is called `{name}`. `cargo xtask mutate --list` prints them."
            ));
        }
        for one in matching {
            if !chosen.iter().any(|already| already.id == one.id) {
                chosen.push(one);
            }
        }
    }
    // In the order of the lists, whatever the order they were asked for in.
    let mut ordered: Vec<Break> = all
        .iter()
        .filter(|one| chosen.iter().any(|chosen| chosen.id == one.id))
        .cloned()
        .collect();
    ordered.dedup_by(|a, b| a.id == b.id);
    Ok(ordered)
}

/// The `breaks` step of `cargo xtask check`: every break of every list
/// still applies to the tree, and every test a `caught-by` names is one of
/// the kept lists of tests, where there is a fresh one. It reads the files
/// the breaks name and runs and builds nothing.
pub(crate) fn check_breaks_apply() -> Result<()> {
    let root = workspace::root();
    for warning in rename_warnings(&list_files(&root)) {
        println!("warning: {warning}");
    }
    let all = list::load_all(&root)?;
    report_check(&root, &all, &Names::Kept)
}

/// The break lists' paths, `mutations/<name>.breaks`, in name order; none
/// when the folder cannot be read (loading the lists then says why).
fn list_files(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(root.join("mutations"))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".breaks"))
        .map(|name| format!("mutations/{name}"))
        .collect();
    files.sort();
    files
}

/// A warning for each break list whose name the rename's rules would
/// rewrite: its path, and its name read as text (the old alias and the
/// labels built on it, which a path alone is not checked for), each by
/// `crate::rename`'s own check. The list's name is in every break's name,
/// which the docs and records quote: a name the rename check refuses there
/// is found late otherwise.
pub(crate) fn rename_warnings(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter_map(|file| {
            let name = file.rsplit('/').next().unwrap_or(file);
            let found = crate::rename::violations(file, name.as_bytes());
            let first = found.first()?;
            Some(format!(
                "{file}: the list's name is one the rename rules would rewrite ({}), and every \
                 break's name begins with it: name the list with no old name in it",
                first.message.split(". ").next().unwrap_or(&first.message)
            ))
        })
        .collect()
}

/// How the tests a `caught-by` names are looked for.
enum Names<'a> {
    /// Not at all: a run of the breaks finds a wrong name itself.
    Not,
    /// In the lists of tests that are kept and fresh. Nothing is built.
    Kept,
    /// In lists made now, for the breaks given: `cargo nextest list`, which
    /// builds the tests of the crates they run.
    Built(&'a [Break]),
}

fn report_check(root: &Path, breaks: &[Break], names: &Names<'_>) -> Result<()> {
    let mut problems = check(root, breaks);
    let applying = problems.len();
    let checked = match names {
        Names::Not => None,
        Names::Kept => Some(names::check(root, breaks, None)?),
        Names::Built(chosen) => {
            crate::checks::require_nextest()?;
            let mut list = |packages: &[String]| {
                println!("Listing the tests of {} ...", packages.join(", "));
                names::nextest_list(root, packages)
            };
            Some(names::check(root, chosen, Some(&mut list))?)
        }
    };
    let mut wrong_names = 0;
    if let Some(checked) = &checked {
        wrong_names = checked.problems.len();
        problems.extend(checked.problems.iter().cloned());
    }
    if problems.is_empty() {
        if breaks.iter().all(Break::is_one_off) {
            println!("The break given with `--break` applies.");
        } else {
            println!(
                "Every break applies: {} in {}/.",
                breaks.len(),
                list::DIRECTORY
            );
        }
        if let Some(checked) = &checked {
            println!("{}", names_said(checked));
        }
        return Ok(());
    }
    for problem in &problems {
        println!("{problem}\n");
    }
    if let Some(checked) = checked
        .as_ref()
        .filter(|checked| !checked.unchecked.is_empty())
    {
        println!("{}", names_said(checked));
    }
    Err(format!(
        "{} problem(s) in the break lists: {applying} break(s) that do not apply, {wrong_names} \
         `caught-by` name(s) of tests that do not exist. Nothing was run.",
        problems.len()
    ))
}

/// What a check of the `caught-by` names covered, in a line.
fn names_said(checked: &names::Checked) -> String {
    let said = names_said_of_lists(checked);
    if checked.by_source == 0 {
        said
    } else {
        format!(
            "{said} {} name(s) of breaks of the other system's code were looked for in the words \
             of the source, since its tests are not built here.",
            checked.by_source
        )
    }
}

/// [`names_said`], of the names checked against the lists of tests.
fn names_said_of_lists(checked: &names::Checked) -> String {
    let missing = checked.unchecked.len();
    match (checked.names, missing) {
        (_, 0) => format!(
            "Every `caught-by` names a test that exists: {} name(s) checked against the lists of \
             tests kept in target/mutate/test-lists.json.",
            checked.names
        ),
        (0, _) => format!(
            "The names in `caught-by` were not checked against the tests that exist: no fresh \
             list of tests for {missing} set(s) of crates. `cargo xtask mutate --check --tests` \
             builds them."
        ),
        (names, _) => format!(
            "{names} `caught-by` name(s) checked; those of {missing} set(s) of crates were not: \
             no fresh list of tests ({}). `cargo xtask mutate --check --tests` builds them.",
            checked.unchecked.join("; ")
        ),
    }
}

/// What is wrong with the breaks, against the tree at `root`: one message
/// for each break that cannot be made, with its list and line.
fn check(root: &Path, breaks: &[Break]) -> Vec<String> {
    let mut problems = Vec::new();
    // Each file's text and tokens, read once: a list has many breaks of
    // one file, and this runs on every pull request.
    // With the tokens a search uses, and those a comparison does.
    type Read = (String, Vec<lex::Token>, Vec<lex::Token>);
    let mut files: BTreeMap<&str, Option<Read>> = BTreeMap::new();
    // Each change made, by its file and what it does, with the first break
    // that makes it.
    let mut changes: BTreeMap<(&str, Change), String> = BTreeMap::new();
    for one in breaks {
        let read = files.entry(one.file.as_str()).or_insert_with(|| {
            let source = read_source(root, one).ok()?;
            let code = lex::code_tokens(&source);
            let tokens = lex::without_layout_commas(&source, &code);
            Some((source, tokens, code))
        });
        let Some((source, tokens, code)) = read else {
            // Read again for the message, which names this break.
            if let Err(problem) = read_source(root, one) {
                problems.push(problem);
            }
            continue;
        };
        let (range, items) = match locate(one, source, tokens) {
            Ok(located) => located,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        problems.extend(preflight(one, source, tokens, &items));
        let change = match literal_change(one, source, tokens, &range) {
            Some((literal, text)) => change_of(source, code, &literal, &text),
            None => change_of(source, code, &range, &one.replace),
        };
        if let Some(first) = changes.get(&(one.file.as_str(), change.clone())) {
            problems.push(format!(
                "{}: break `{}` makes the same change as `{first}`: once the code is read by \
                 token, the two are one break, and a second run of it tests nothing more.\n  \
                 Delete one of them, or make it break the rule its `rule:` names.",
                one.place(),
                one.id
            ));
        } else {
            changes.insert((one.file.as_str(), change), one.id.clone());
        }
        match one.test_packages() {
            Err(problem) => problems.push(problem),
            Ok(packages) => {
                for package in packages {
                    let manifest = if package == "xtask" {
                        root.join("xtask").join("Cargo.toml")
                    } else {
                        root.join("crates").join(&package).join("Cargo.toml")
                    };
                    if !manifest.is_file() {
                        problems.push(format!(
                            "{}: break `{}` runs the tests of `{package}`, and there is no such \
                             crate.\n  Correct its `packages:`.",
                            one.place(),
                            one.id
                        ));
                    }
                }
            }
        }
    }
    problems
}

/// For a break whose `find` is an `in-string` text: the whole string literal
/// the range `inside` is in, and the text of that literal with the break's
/// `replace` put over the range. The change of such a break to the file, as
/// a change of one token. `None` for any other break.
fn literal_change(
    one: &Break,
    source: &str,
    tokens: &[lex::Token],
    inside: &Range<usize>,
) -> Option<(Range<usize>, String)> {
    if !one.in_string {
        return None;
    }
    let literal = lex::literal_around(source, tokens, inside)?;
    let mut text = String::with_capacity(literal.len() + one.replace.len());
    text.push_str(&source[literal.start..inside.start]);
    text.push_str(&one.replace);
    text.push_str(&source[inside.end..literal.end]);
    Some((literal, text))
}

/// Whether `text` is exactly one string literal.
fn is_one_string(text: &str) -> bool {
    lex::string_content(text).is_some() && lex::code_texts(text).len() == 1
}

/// What a change does to a file, read by token: how many tokens it keeps
/// at the start and at the end, and the tokens it puts between them. Two
/// breaks with the same change are one break, whatever their `find` and
/// `replace` say: `a && b` made `a`, and `&& b` made nothing.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Change {
    kept_before: usize,
    kept_after: usize,
    put: Vec<String>,
}

/// The change that putting `replace` over the bytes `range` of `source`
/// makes, from the tokens of `source` (`code`, [`lex::code_tokens`]).
///
/// The file with the break made is never lexed: its tokens are those of
/// `source` before the range, `replace`'s, and those of `source` after it.
/// Every break of a list is judged on every pull request, and lexing two
/// whole files for each took the step from 0.2 s to 1.3 s.
fn change_of(source: &str, code: &[lex::Token], range: &Range<usize>, replace: &str) -> Change {
    let before = |index: usize| code[index].text(source);
    let first = code.partition_point(|token| token.start < range.start);
    let after_range = code.partition_point(|token| token.start < range.end);
    let middle = lex::code_texts(replace);
    let length = first + middle.len() + (code.len() - after_range);
    let after = |index: usize| -> &str {
        if index < first {
            before(index)
        } else if index < first + middle.len() {
            middle[index - first]
        } else {
            before(index - first - middle.len() + after_range)
        }
    };
    // The tokens outside the range are the same on both sides by
    // construction: the comparison starts at the range, not at the ends.
    let most = code.len().min(length);
    let kept_before = (first..most)
        .find(|&index| before(index) != after(index))
        .unwrap_or(most);
    let same_after = code.len() - after_range;
    let kept_after = (same_after.min(most - kept_before)..most - kept_before)
        .find(|&back| before(code.len() - 1 - back) != after(length - 1 - back))
        .unwrap_or(most - kept_before);
    Change {
        kept_before,
        kept_after,
        put: (kept_before..length - kept_after)
            .map(|index| after(index).to_owned())
            .collect(),
    }
}

/// What makes a break that applies today stop applying, or apply to code
/// it did not mean, on a later day: the checks of the pre-flight that come
/// after the break is made ([`apply_in`] has the others).
///
/// - Its `find` ends in a comma. `rustfmt` adds and removes the comma
///   before a closing bracket as it wraps and joins lines, and the search
///   passes over such a comma, so the day the code is laid out again the
///   break no longer finds it.
/// - Its `in:` names more than one item. The break is made in the one item
///   whose code it finds today; when the code of another of them changes,
///   it may find that one instead, or both. Not when each of them is under
///   a `#[cfg(…)]` about the system: one function for each system, of
///   which a build has one.
fn preflight(
    one: &Break,
    source: &str,
    tokens: &[lex::Token],
    items: &[Range<usize>],
) -> Vec<String> {
    let mut problems = Vec::new();
    if !one.in_string && ends_in_a_layout_comma(&one.find) {
        problems.push(format!(
            "{}: break `{}` has a `find` that ends in a comma, which rustfmt moves: the break \
             stops applying when the line is wrapped or joined.\n  End `find` before the comma.",
            one.place(),
            one.id
        ));
    }
    if let Some(anchor) = &one.anchor
        && items.len() > 1
        && !items
            .iter()
            .all(|item| lex::under_cfg(source, tokens, item.start))
    {
        let lines: Vec<String> = items
            .iter()
            .map(|item| lex::line_of(source, tokens[item.start].start).to_string())
            .collect();
        problems.push(format!(
            "{}: break `{}` has `in: {anchor}`, which names {} items in `{}`, at lines {}.\n  \
             Name the one meant (`Type::method`, or the `impl`'s trait or type), or leave \
             `in` out if `find` is in the file once.",
            one.place(),
            one.id,
            items.len(),
            one.file,
            lines.join(", ")
        ));
    }
    problems
}

/// Where a `find` that ends in a comma, and is not found, is found without
/// it: a comma before a closing bracket is not a token of the source, and
/// one at the end of `find` is (nothing closes after it), so such a `find`
/// can never match there. The line of the match's end, or `None` when the
/// `find` does not end in a comma or is not there without it either.
fn comma_before_a_bracket(one: &Break, source: &str, tokens: &[lex::Token]) -> Option<usize> {
    if lex::code_texts(&one.find).last() != Some(&",") {
        return None;
    }
    let shorter = one.find.trim_end().strip_suffix(',')?;
    match lex::locate_in(
        source,
        tokens,
        one.anchor.as_deref(),
        shorter,
        one.occurrence,
    ) {
        Ok(range) => Some(lex::line_of(source, range.end)),
        Err(lex::Missing::Ambiguous(lines)) => lines.first().copied(),
        Err(_) => None,
    }
}

/// Whether `find` ends in a comma that `rustfmt` may move: one after an
/// argument, an element or a field. Not the comma after a match arm that is
/// an expression (`Some(1) => one,`), which the formatter always writes.
fn ends_in_a_layout_comma(find: &str) -> bool {
    let texts = lex::code_texts(find);
    let Some((&",", before)) = texts.split_last() else {
        return false;
    };
    // The piece the comma ends: back to the comma before it at the same
    // depth, or to the bracket the piece is inside.
    let mut depth = 0usize;
    let mut start = 0;
    for (index, text) in before.iter().enumerate().rev() {
        match *text {
            ")" | "]" | "}" => depth += 1,
            "(" | "[" | "{" if depth == 0 => {
                start = index + 1;
                break;
            }
            "(" | "[" | "{" => depth -= 1,
            "," if depth == 0 => {
                start = index + 1;
                break;
            }
            _ => {}
        }
    }
    let piece = &before[start..];
    // A match arm has `=>` outside the brackets of its pattern and value.
    let mut depth = 0usize;
    for (index, text) in piece.iter().enumerate() {
        match *text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.saturating_sub(1),
            "=" if depth == 0 && piece.get(index + 1) == Some(&">") => return false,
            _ => {}
        }
    }
    true
}

/// The text of the file a break changes.
fn read_source(root: &Path, one: &Break) -> Result<String> {
    let path = root.join(&one.file);
    fs::read_to_string(&path).map_err(|error| {
        format!(
            "{}: break `{}` no longer applies: `{}` cannot be read ({error}).\n  {}",
            one.place(),
            one.id,
            one.file,
            how_to_fix(one)
        )
    })
}

fn how_to_fix(one: &Break) -> String {
    format!(
        "The code this break changes has moved or been rewritten. If the rule it breaks is \
         still in the code (\"{}\"), change the break's `file`, `in` and `find` to the code as \
         it is now. If the rule is gone, delete the break. Then run `cargo xtask mutate \
         --check`, and `cargo xtask mutate {}` to see that its tests still catch it.",
        one.rule, one.id
    )
}

/// The text of a file with the break made. An error says why the break
/// cannot be made, with the break's list and line and what to do about it.
///
/// A break that changes nothing is an error too: it would be reported as
/// one that no test caught.
fn apply(one: &Break, source: &str) -> Result<String> {
    apply_in(one, source, &lex::tokens(source))
}

/// [`apply`], for a source whose tokens are already known.
fn apply_in(one: &Break, source: &str, tokens: &[lex::Token]) -> Result<String> {
    let (range, _) = locate(one, source, tokens)?;
    let mut broken = String::with_capacity(source.len() + one.replace.len());
    broken.push_str(&source[..range.start]);
    broken.push_str(&one.replace);
    broken.push_str(&source[range.end..]);
    Ok(broken)
}

/// Where in `source` a break is made: the bytes its `replace` goes over.
/// An error says why the break cannot be made, as [`apply`] does.
///
/// With the range, the items its `in` names, for the pre-flight.
fn locate(
    one: &Break,
    source: &str,
    tokens: &[lex::Token],
) -> Result<(Range<usize>, Vec<Range<usize>>)> {
    let inside = one
        .anchor
        .as_ref()
        .map(|anchor| format!(" inside `{anchor}`"))
        .unwrap_or_default();
    let located = one.locate_with_items(source, tokens, one.anchor.as_deref());
    let (range, items) = located.map_err(|missing| {
        let (why, fix) = match missing {
            lex::Missing::NothingToFind if one.in_string => (
                "its `in-string` has no text in it".to_owned(),
                "Give `in-string` the text to change.".to_owned(),
            ),
            lex::Missing::NothingToFind => (
                "its `find` has no code in it".to_owned(),
                "Give `find` the code to replace.".to_owned(),
            ),
            lex::Missing::NoSuchItem { segment, within } => (
                if within.is_empty() {
                    format!("`{}` has no item called `{segment}`", one.file)
                } else {
                    format!(
                        "`{within}` in `{}` has no item called `{segment}`",
                        one.file
                    )
                },
                how_to_fix(one),
            ),
            lex::Missing::NotFound if one.in_string => (
                format!(
                    "its `in-string` text is not in any string literal of `{}`{inside}",
                    one.file
                ),
                format!(
                    "{} The text is looked for as the literal is written in the source, escapes \
                     as they are typed.",
                    how_to_fix(one)
                ),
            ),
            lex::Missing::NotFound => match comma_before_a_bracket(one, source, tokens) {
                Some(line) => (
                    format!(
                        "its `find` ends in a comma, and in `{}` that comma (line {line}) comes \
                         before a closing bracket, where it is the formatter's and not a token; \
                         the `find` has it as a token, so it cannot match",
                        one.file
                    ),
                    "End `find` before the comma, and `replace` too if it ends in one.".to_owned(),
                ),
                None => (
                    format!("its `find` is not in `{}`{inside}", one.file),
                    how_to_fix(one),
                ),
            },
            lex::Missing::Ambiguous(lines) => (
                format!(
                    "its {} is in `{}`{inside} {} times, at lines {}",
                    if one.in_string {
                        "`in-string` text"
                    } else {
                        "`find`"
                    },
                    one.file,
                    lines.len(),
                    lines
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                "Make it longer (with the characters around it), name the item it is in with \
                     `in:`, or say which one is meant with `occurrence:` (counted from 1)."
                    .to_owned(),
            ),
            lex::Missing::NoSuchOccurrence { asked, found } => (
                format!(
                    "it asks for occurrence {asked} of its {}, which is in `{}`{inside} \
                         {found} time(s)",
                    if one.in_string {
                        "`in-string` text"
                    } else {
                        "`find`"
                    },
                    one.file
                ),
                how_to_fix(one),
            ),
        };
        format!(
            "{}: break `{}` no longer applies: {why}.\n  {fix}",
            one.place(),
            one.id
        )
    })?;
    // Inside a string literal, the change is to its text, and the literal
    // must still be one string after it: a `"` in `replace` that no
    // backslash escapes would end it.
    if one.in_string {
        if source[range.clone()] == one.replace {
            return Err(format!(
                "{}: break `{}` changes nothing: its `replace` is the text of its `in-string`.\n  \
                 Give `replace` the broken text.",
                one.place(),
                one.id
            ));
        }
        if let Some((_, text)) = literal_change(one, source, tokens, &range)
            && !is_one_string(&text)
        {
            return Err(format!(
                "{}: break `{}` makes a string literal that is not one string: its `replace` \
                 holds a `\"` that no backslash escapes, or ends in a backslash.\n  Write the \
                 quote as `\\\"`.",
                one.place(),
                one.id
            ));
        }
        return Ok((range, items));
    }
    // Judged on the code that is replaced: the rest of the file is the same
    // text on both sides.
    if lex::same_tokens(&source[range.clone()], &one.replace) {
        return Err(format!(
            "{}: break `{}` changes nothing: its `replace` is the same code as its `find`.\n  \
             Give `replace` the broken code.",
            one.place(),
            one.id
        ));
    }
    Ok((range, items))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::list::{self, Break};
    use super::{Mode, Request, apply, check, choose};

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect()
    }

    pub(super) fn breaks(list: &str, text: &str) -> Vec<Break> {
        list::parse(&format!("mutations/{list}.breaks"), text).unwrap()
    }

    #[test]
    fn a_lists_name_the_rename_would_rewrite_is_warned_of() {
        // The old names, made of pieces, so that this file holds none.
        let alias = ["c", "x"].concat();
        let project = ["cor", "tex"].concat();
        let files = [
            format!("mutations/{alias}-labels.breaks"),
            format!("mutations/{project}-x.breaks"),
            "mutations/old-alias-labels.breaks".to_owned(),
            "mutations/step-12.breaks".to_owned(),
            "mutations/context.breaks".to_owned(),
        ];
        let warnings = super::rename_warnings(&files);
        assert_eq!(warnings.len(), 2, "{warnings:#?}");
        assert!(
            warnings[0].starts_with(&format!("mutations/{alias}-labels.breaks: ")),
            "{}",
            warnings[0]
        );
        assert!(warnings[0].contains("the old alias"), "{}", warnings[0]);
        assert!(
            warnings[1].starts_with(&format!("mutations/{project}-x.breaks: ")),
            "{}",
            warnings[1]
        );
    }

    const SOURCE: &str = "\
pub fn over(seconds: f64, budget: f64) -> bool {
    seconds > budget
}

pub fn far_over(seconds: f64, budget: f64) -> bool {
    seconds > 1.5 * budget
}
";

    fn one(body: &str) -> Break {
        let text = format!("[break] b\nrule: The budget is a limit.\nfile: src/lib.rs\n{body}");
        breaks("demo", &text).remove(0)
    }

    #[test]
    fn jobs_are_a_number_from_one_to_sixteen() {
        let jobs =
            |text: &str| Request::parse(&strings(&["--jobs", text])).map(|request| request.jobs);
        assert_eq!(jobs("4"), Ok(4));
        assert_eq!(jobs("16"), Ok(16));
        assert_eq!(Request::parse(&strings(&["-j", "2"])).unwrap().jobs, 2);
        assert_eq!(Request::parse(&[]).unwrap().jobs, 2);
        for wrong in ["0", "17", "many", "-1"] {
            let error = jobs(wrong).unwrap_err();
            assert!(error.contains("a number from 1 to 16"), "{wrong}: {error}");
        }
        assert!(Request::parse(&strings(&["--jobs"])).is_err());
    }

    #[test]
    fn a_break_is_made_where_its_code_is_and_nowhere_else() {
        let broken = apply(
            &one("in: far_over\nfind: seconds > 1.5 * budget\nreplace: seconds > budget\n"),
            SOURCE,
        )
        .unwrap();

        assert_eq!(
            broken,
            SOURCE.replace("seconds > 1.5 * budget", "seconds > budget")
        );
    }

    #[test]
    fn a_break_is_made_whatever_the_formatter_did_to_the_code() {
        let reformatted = SOURCE.replace(
            "    seconds > 1.5 * budget",
            "    seconds\n        > 1.5 // one and a half\n            * budget",
        );

        let broken = apply(
            &one("find: seconds > 1.5 * budget\nreplace: true\n"),
            &reformatted,
        )
        .unwrap();

        assert!(broken.contains("-> bool {\n    true\n}"), "{broken}");
    }

    #[test]
    fn a_break_whose_code_is_gone_says_where_it_is_listed_and_what_to_do() {
        let error = apply(
            &one("in: far_over\nfind: seconds >= budget\nreplace: true\n"),
            SOURCE,
        )
        .unwrap_err();

        assert!(
            error.starts_with(
                "mutations/demo.breaks:1: break `demo/b` no longer applies: its `find` is not \
                 in `src/lib.rs` inside `far_over`."
            ),
            "{error}"
        );
        assert!(error.contains("The budget is a limit."), "{error}");
        assert!(error.contains("delete the break"), "{error}");
        assert!(error.contains("cargo xtask mutate --check"), "{error}");
    }

    #[test]
    fn a_break_that_could_be_made_in_two_places_is_refused() {
        let error = apply(&one("find: seconds >\nreplace: seconds <\n"), SOURCE).unwrap_err();

        assert!(error.contains("2 times, at lines 2, 6"), "{error}");
        assert!(error.contains("`occurrence:`"), "{error}");

        let second = apply(
            &one("find: seconds >\nreplace: seconds <\noccurrence: 2\n"),
            SOURCE,
        )
        .unwrap();
        assert!(second.contains("seconds < 1.5 * budget"), "{second}");
        assert!(second.contains("seconds > budget"), "{second}");
    }

    #[test]
    fn a_missing_item_is_named() {
        let error = apply(&one("in: total\nfind: seconds\nreplace: 0.0\n"), SOURCE).unwrap_err();

        assert!(
            error.contains("`src/lib.rs` has no item called `total`"),
            "{error}"
        );
    }

    #[test]
    fn a_break_that_changes_nothing_is_refused() {
        // The same code, laid out another way.
        let error = apply(
            &one("in: over\nfind: seconds > budget\nreplace:\n    seconds // still\n        > budget\n"),
            SOURCE,
        )
        .unwrap_err();

        assert!(error.contains("changes nothing"), "{error}");
    }

    #[test]
    fn the_check_reads_the_tree_and_names_every_break_that_cannot_be_made() {
        let root = super::drive::tests::tree(
            "check",
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/lib.rs", SOURCE),
            ],
        );
        let list = "\
[break] fine
rule: r
file: crates/demo/src/lib.rs
in: over
find: seconds > budget
replace: true

[break] gone
rule: r
file: crates/demo/src/lib.rs
find: seconds == budget
replace: true

[break] no-file
rule: r
file: crates/demo/src/other.rs
find: x
replace: y

[break] no-crate
rule: r
file: crates/demo/src/lib.rs
in: over
find: seconds > budget
replace: false
packages: cineris-nowhere
";

        let problems = check(&root, &breaks("demo", list));

        assert_eq!(problems.len(), 3, "{problems:#?}");
        assert!(problems[0].starts_with("mutations/demo.breaks:8: break `demo/gone`"));
        assert!(problems[1].starts_with("mutations/demo.breaks:14: break `demo/no-file`"));
        assert!(problems[1].contains("cannot be read"), "{}", problems[1]);
        assert!(problems[2].starts_with("mutations/demo.breaks:20: break `demo/no-crate`"));
        assert!(problems[2].contains("no such crate"), "{}", problems[2]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The source the pre-flight's lists are made against: a call with
    /// arguments, a match, one function for each system, and two items of
    /// one name.
    const PREFLIGHT: &str = "\
pub fn limit(seconds: u64) -> u64 {
    clamp(seconds, 1, 600)
}

pub fn kind(code: u16) -> &'static str {
    match code {
        413 => \"too large\",
        503 => \"overloaded\",
        _ => \"other\",
    }
}

#[cfg(unix)]
fn owner() -> bool {
    uid() == 0
}

#[cfg(windows)]
fn owner() -> bool {
    sid().is_admin()
}

pub struct Budget;

impl Budget {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Budget {
    fn default() -> Self {
        Budget::new()
    }
}

impl fmt::Debug for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(\"budget\")
    }
}
";

    /// The pre-flight's problems with `list`, against `PREFLIGHT`, in a tree
    /// named for `test`: two tests of one name would share one folder, and
    /// under `cargo test`, where they are threads, one would remove the
    /// other's.
    fn problems_with(test: &str, list: &str) -> Vec<String> {
        let root = super::drive::tests::tree(
            test,
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/lib.rs", PREFLIGHT),
            ],
        );
        let problems = check(&root, &breaks("demo", list));
        let _ = std::fs::remove_dir_all(&root);
        problems
    }

    #[test]
    fn a_list_the_preflight_passes() {
        let list = "\
[break] limit-too-high
rule: r
file: crates/demo/src/lib.rs
find: clamp(seconds, 1, 600)
replace: clamp(seconds, 1, 6000)

[break] an-arm-removed
rule: r
file: crates/demo/src/lib.rs
in: kind
find: 413 => \"too large\",
replace:

[break] owner-anyone
rule: r
file: crates/demo/src/lib.rs
in: owner
find: uid() == 0
replace: true

[break] new-in-default
rule: r
file: crates/demo/src/lib.rs
in: Default::default
find: Budget::new()
replace: Budget

[break] debug-says-other
rule: r
file: crates/demo/src/lib.rs
in: Budget::fmt
find: f.write_str(\"budget\")
replace: f.write_str(\"other\")
";
        assert_eq!(problems_with("preflight-passes", list), [] as [String; 0]);
    }

    #[test]
    fn a_find_whose_comma_comes_before_a_bracket_is_told_why_it_cannot_match() {
        let root = super::drive::tests::tree(
            "preflight-comma",
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                (
                    "crates/demo/src/lib.rs",
                    "pub fn call() {\n    run(\n        first,\n        second,\n    )\n}\n",
                ),
            ],
        );
        let list = "\
[break] comma-before-bracket
rule: r
file: crates/demo/src/lib.rs
find: second,
replace: third,

[break] comma-elsewhere
rule: r
file: crates/demo/src/lib.rs
find: fourth,
replace: third,
";
        let problems = check(&root, &breaks("demo", list));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(
            problems[0].contains("`demo/comma-before-bracket`")
                && problems[0].contains("that comma (line 4) comes before a closing bracket")
                && problems[0].contains("End `find` before the comma"),
            "{problems:#?}"
        );
        // A `find` that is not there without its comma either is just not
        // there.
        assert!(
            problems[1].contains("`demo/comma-elsewhere`")
                && problems[1].contains("its `find` is not in")
                && !problems[1].contains("closing bracket"),
            "{problems:#?}"
        );
    }

    #[test]
    fn the_preflight_names_each_kind_of_break_that_will_go_wrong() {
        let list = "\
[break] same-code
rule: r
file: crates/demo/src/lib.rs
find: clamp(seconds, 1, 600)
replace: clamp( seconds, 1, /* the same */ 600 )

[break] gone
rule: r
file: crates/demo/src/lib.rs
find: clamp(seconds, 2, 600)
replace: 0

[break] twice
rule: r
file: crates/demo/src/lib.rs
find: Self
replace: Budget

[break] comma
rule: r
file: crates/demo/src/lib.rs
find: seconds, 1,
replace: seconds, 0,

[break] two-items
rule: r
file: crates/demo/src/lib.rs
in: Budget
find: Self
occurrence: 1
replace: Budget

[break] first
rule: r
file: crates/demo/src/lib.rs
find: 600)
replace: 6000)

[break] same-change
rule: r
file: crates/demo/src/lib.rs
in: limit
find: clamp(seconds, 1, 600)
replace: clamp(seconds, 1, 6000)
";
        let problems = problems_with("preflight-kinds", list);
        let said = |name: &str, what: &str| {
            problems.iter().any(|problem| {
                problem.contains(&format!("`demo/{name}`")) && problem.contains(what)
            })
        };
        assert_eq!(problems.len(), 6, "{problems:#?}");
        assert!(said("same-code", "changes nothing"), "{problems:#?}");
        assert!(said("gone", "is not in"), "{problems:#?}");
        assert!(said("twice", "times, at lines"), "{problems:#?}");
        assert!(
            said("comma", "ends in a comma, which rustfmt moves"),
            "{problems:#?}"
        );
        assert!(
            said("two-items", "names 4 items in `crates/demo/src/lib.rs`"),
            "{problems:#?}"
        );
        assert!(
            said("same-change", "makes the same change as `demo/first`"),
            "{problems:#?}"
        );
    }

    /// Two functions of one name, one under `cfg(test)` and one under a
    /// feature, can both be built: an `in` naming them is refused. One for
    /// each system is not.
    #[test]
    fn only_a_cfg_of_the_system_makes_two_items_of_a_name_one() {
        let source = "#[cfg(test)]\nfn twin() -> u8 { 1 }\n#[cfg(feature = \"x\")]\nfn twin() -> u8 { 2 }\n\
                      #[cfg(target_os = \"linux\")]\nfn os() -> u8 { 3 }\n#[cfg(not(target_os = \"linux\"))]\nfn os() -> u8 { 4 }\n";
        let root = super::drive::tests::tree(
            "preflight_cfg",
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/lib.rs", source),
            ],
        );
        let list = "\
[break] twin
rule: r
file: crates/demo/src/lib.rs
in: twin
find: 2
replace: 5

[break] os
rule: r
file: crates/demo/src/lib.rs
in: os
find: 3
replace: 5
";
        let problems = check(&root, &breaks("demo", list));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(
            problems[0].contains("`demo/twin` has `in: twin`, which names 2 items"),
            "{problems:#?}"
        );
    }

    #[test]
    fn a_comma_is_a_layout_comma_unless_it_ends_a_match_arm() {
        use super::ends_in_a_layout_comma as layout;
        assert!(layout("f(a, b,"));
        assert!(layout("x: 1,"));
        assert!(layout("seconds,"));
        assert!(!layout("f(a, b)"));
        assert!(!layout("Some(413) => Signal::ContextLength,"));
        assert!(!layout("x => f(a, b),"));
        assert!(!layout("'\\u{80}'..='\\u{9f}' => Some(C1),"));
        // An arm's comma ends the arm, not an argument inside its value.
        assert!(layout("x => f(a,"));
    }

    #[test]
    fn breaks_are_chosen_by_name_by_list_or_by_the_end_of_a_name() {
        let body = "rule: r\nfile: f\nfind: x\nreplace: y\n";
        let mut all = breaks("loop", &format!("[break] a\n{body}[break] b\n{body}"));
        all.extend(breaks(
            "policy",
            &format!("[break] a\n{body}[break] c\n{body}"),
        ));
        let ids = |names: &[&str]| -> Vec<String> {
            choose(&all, &strings(names))
                .unwrap()
                .into_iter()
                .map(|one| one.id)
                .collect()
        };

        assert_eq!(ids(&[]), ["loop/a", "loop/b", "policy/a", "policy/c"]);
        assert_eq!(ids(&["policy/c"]), ["policy/c"]);
        assert_eq!(ids(&["loop"]), ["loop/a", "loop/b"]);
        assert_eq!(ids(&["a"]), ["loop/a", "policy/a"]);
        // In list order, and once, however they were asked for.
        assert_eq!(
            ids(&["c", "loop/a", "policy"]),
            ["loop/a", "policy/a", "policy/c"]
        );
        let error = choose(&all, &strings(&["d"])).unwrap_err();
        assert!(error.contains("no break is called `d`"), "{error}");
    }

    #[test]
    fn the_exit_status_says_whether_the_list_held_and_whether_there_is_more_to_do() {
        use super::drive::Finished;

        assert_eq!(
            super::conclusion(&Ok(Finished::Complete { flagged: 0 }), false),
            (0, None)
        );
        let (status, message) = super::conclusion(&Ok(Finished::Complete { flagged: 2 }), false);
        assert_eq!(status, 1);
        assert!(message.unwrap().contains("2 break(s) did not do"));
        let (status, message) = super::conclusion(&Ok(Finished::Unfinished { left: 5 }), false);
        assert_eq!(status, 2);
        assert!(message.unwrap().contains("Run the same command again"));
        let (status, message) = super::conclusion(&Err("no cargo".to_owned()), false);
        assert_eq!((status, message.as_deref()), (1, Some("no cargo")));
    }

    #[test]
    fn a_run_that_forgot_first_is_carried_on_without_forgetting_again() {
        use super::drive::Finished;

        let (status, message) = super::conclusion(&Ok(Finished::Unfinished { left: 5 }), true);

        assert_eq!(status, 2);
        assert!(
            message
                .unwrap()
                .contains("Run the same command without `--again` to carry on."),
        );
    }

    #[test]
    fn a_list_from_anywhere_is_read_and_may_not_take_a_lists_name() {
        let dir = std::env::temp_dir().join(format!("cin-mutate-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let review = dir.join("review-16.breaks");
        std::fs::write(
            &review,
            "[break] a\nrule: R.\nfile: x.rs\nfind: a\nreplace: b\n",
        )
        .unwrap();
        let known = breaks(
            "step-16",
            "[break] b\nrule: R.\nfile: x.rs\nfind: c\nreplace: d\n",
        );
        let found = super::load_files(&known, std::slice::from_ref(&review)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "review-16/a");
        let clash = dir.join("step-16.breaks");
        std::fs::write(
            &clash,
            "[break] a\nrule: R.\nfile: x.rs\nfind: a\nreplace: b\n",
        )
        .unwrap();
        let error = super::load_files(&known, &[clash]).unwrap_err();
        assert!(
            error.contains("has the name of a list of mutations/: `step-16`"),
            "{error}"
        );
        assert!(super::load_files(&known, &[dir.join("missing.breaks")]).is_err());
        // Two lists of one name, from two folders.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("review-16.breaks"),
            "[break] c\nrule: R.\nfile: x.rs\nfind: e\nreplace: f\n",
        )
        .unwrap();
        let error = super::load_files(&known, &[review.clone(), other.join("review-16.breaks")])
            .unwrap_err();
        assert!(
            error.contains("another list given with `--file`: `review-16`"),
            "{error}"
        );
        assert_eq!(
            Request::parse(&strings(&["--file", "x.breaks"]))
                .unwrap()
                .files,
            [std::path::PathBuf::from("x.breaks")]
        );
        assert!(Request::parse(&strings(&["--file"])).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_busy_disk_makes_a_run_of_several_jobs_one() {
        use crate::stall::Pressure;
        let at = |full_avg60: f64| Pressure {
            some: (0.0, full_avg60),
            full: (0.0, full_avg60),
            some_total: 0,
            full_total: 0,
        };
        let (jobs, note) = super::jobs_for(2, Some(&at(66.0)));
        assert_eq!(jobs, 1);
        assert!(note.unwrap().contains("66.0% of the last minute"));
        assert_eq!(super::jobs_for(4, Some(&at(10.0))).0, 1);
        // A quiet disk, one job asked, or no pressure to read: as asked.
        assert_eq!(super::jobs_for(2, Some(&at(9.9))), (2, None));
        assert_eq!(super::jobs_for(1, Some(&at(66.0))), (1, None));
        assert_eq!(super::jobs_for(3, None), (3, None));
    }

    #[test]
    fn a_request_is_read() {
        let request = Request::parse(&strings(&[
            "--minutes",
            "0.5",
            "--again",
            "step-12",
            "--no-compile-check",
        ]))
        .unwrap();

        assert_eq!(
            request,
            Request {
                mode: Mode::Run,
                only: strings(&["step-12"]),
                budget: Some(Duration::from_secs(30)),
                again: true,
                compile_check: false,
                jobs: 2,
                tests: false,
                fix_in: false,
                write: false,
                files: Vec::new(),
                one_off: list::OneOff::default(),
                flagged: false,
                changed: None,
                merge: None,
                dry_run: false,
                by_relevance: None,
                minutes_given: true,
            }
        );
        assert_eq!(
            Request::parse(&[]).unwrap().budget,
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            Request::parse(&strings(&["--minutes", "0"]))
                .unwrap()
                .budget,
            None
        );
        assert_eq!(
            Request::parse(&strings(&["--check"])).unwrap().mode,
            Mode::Check
        );
    }

    #[test]
    fn a_budget_is_the_time_limit_and_orders_the_breaks_and_is_not_given_twice() {
        let request =
            Request::parse(&strings(&["--changed", "main", "--budget", "1h30m"])).unwrap();
        assert_eq!(request.budget, Some(Duration::from_secs(5400)));
        assert_eq!(request.by_relevance.as_deref(), Some("1h30m"));
        // `--list` shows its order.
        assert!(Request::parse(&strings(&["--budget", "90s", "--list"])).is_ok());
        for (arguments, said) in [
            (
                &["--budget", "30m", "--minutes", "5"][..],
                "do not go together",
            ),
            (
                &["--minutes", "5", "--budget", "30m"][..],
                "do not go together",
            ),
            (&["--budget", "30m", "--check"][..], "run no break"),
            (&["--budget", "30"][..], "with its unit"),
            (&["--budget"][..], "needs a length of time"),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(said), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn the_check_takes_the_flags_that_list_the_tests_and_correct_the_in_lines() {
        let request = Request::parse(&strings(&[
            "--check", "--tests", "--fix-in", "--write", "step-12",
        ]))
        .unwrap();

        assert_eq!(request.mode, Mode::Check);
        assert!(request.tests && request.fix_in && request.write);
        assert_eq!(request.only, strings(&["step-12"]));
        // In any order.
        assert!(
            Request::parse(&strings(&["--write", "--fix-in", "--check"]))
                .unwrap()
                .write
        );
    }

    #[test]
    fn a_break_given_on_the_command_line_is_read_with_all_its_options() {
        let request = Request::parse(&strings(&[
            "--break",
            "crates/demo/src/lib.rs",
            "--find",
            "seconds > budget",
            "--replace",
            "",
            "--in",
            "over",
            "--occurrence",
            "2",
            "--packages",
            "demo, other",
            "--test",
            "tests::a",
            "--test",
            "demo::it tests::b*",
        ]))
        .unwrap();

        assert_eq!(
            request.one_off,
            list::OneOff {
                file: Some("crates/demo/src/lib.rs".to_owned()),
                anchor: Some("over".to_owned()),
                find: Some("seconds > budget".to_owned()),
                in_string: None,
                replace: Some(String::new()),
                occurrence: Some(2),
                packages: strings(&["demo", "other"]),
                tests: strings(&["tests::a", "demo::it tests::b*"]),
            }
        );
        assert_eq!(request.mode, Mode::Run);
        // A value that begins with a dash is a value, not an option.
        let request = Request::parse(&strings(&[
            "--break",
            "f.rs",
            "--find",
            "a - b",
            "--replace",
            "-1",
        ]))
        .unwrap();
        assert_eq!(request.one_off.replace.as_deref(), Some("-1"));
    }

    #[test]
    fn flagged_and_merge_are_read() {
        let request = Request::parse(&strings(&["--flagged", "step-12"])).unwrap();
        assert!(request.flagged);
        assert_eq!(request.only, strings(&["step-12"]));
        assert_eq!(
            Request::parse(&strings(&["--flagged", "--report"]))
                .unwrap()
                .mode,
            Mode::Report
        );

        let request = Request::parse(&strings(&["--merge", "tooling-7", "--dry-run"])).unwrap();
        assert_eq!(request.mode, Mode::Merge);
        assert_eq!(request.merge.as_deref(), Some("tooling-7"));
        assert!(request.dry_run);
    }

    #[test]
    fn the_breaks_are_split_by_the_system_that_builds_their_code() {
        let other = if cfg!(windows) { "linux" } else { "windows" };
        let body = "rule: r\nfile: f\nfind: x\nreplace: y\n";
        let all = breaks(
            "loop",
            &format!(
                "[break] anywhere\n{body}[break] elsewhere\n{body}os: {other}\n\
                 [break] here\n{body}os: {}\n",
                if cfg!(windows) { "windows" } else { "linux" }
            ),
        );

        let (here, elsewhere): (Vec<Break>, Vec<Break>) =
            all.into_iter().partition(Break::runs_here);

        let ids = |breaks: &[Break]| breaks.iter().map(|one| one.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&here), ["loop/anywhere", "loop/here"]);
        assert_eq!(ids(&elsewhere), ["loop/elsewhere"]);
    }

    #[test]
    fn what_is_left_to_the_other_system_is_said_with_the_command_that_runs_it_there() {
        let other = if cfg!(windows) { "linux" } else { "windows" };
        let all = breaks(
            "loop",
            &format!("[break] a\nrule: r\nfile: f\nfind: x\nreplace: y\nos: {other}\n"),
        );

        let said = super::left_elsewhere(&all).unwrap();

        assert!(said.contains("1 break(s) are of code only"), "{said}");
        assert!(said.contains("neither a pass nor a failure"), "{said}");
        assert!(said.contains("loop/a  (mutations/loop.breaks:1)"), "{said}");
        assert!(
            said.contains(&format!(
                "gh workflow run try --ref <branch> -f os={other} -f command=\"mutate loop/a\""
            )),
            "{said}"
        );
        assert_eq!(super::left_elsewhere(&[]), None);

        // A run of nothing but such breaks is refused: it ran nothing.
        let refused = super::nothing_run_here(&[], &all).unwrap();
        assert!(refused.contains("none was run here"), "{refused}");
        assert!(refused.contains("This is not a pass"), "{refused}");
        let here = breaks("loop", "[break] b\nrule: r\nfile: f\nfind: x\nreplace: y\n");
        assert_eq!(super::nothing_run_here(&here, &all), None);
        assert_eq!(super::nothing_run_here(&[], &[]), None);
    }

    #[test]
    fn a_break_of_the_other_systems_code_is_applied_by_the_preflight_here() {
        let other = if cfg!(windows) { "linux" } else { "windows" };
        let list = format!(
            "[break] fine\nrule: r\nfile: crates/demo/src/lib.rs\nin: limit\n\
             find: clamp(seconds, 1, 600)\nreplace: clamp(seconds, 1, 6000)\nos: {other}\n\n\
             [break] gone\nrule: r\nfile: crates/demo/src/lib.rs\nfind: clamp(seconds, 2, 600)\n\
             replace: 0\nos: {other}\n"
        );

        let problems = problems_with("preflight-other-system", &list);

        // Held, whichever system builds it: the one that cannot be made is
        // told, the other is not.
        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(problems[0].contains("`demo/gone`"), "{problems:#?}");
    }

    #[test]
    fn a_request_that_makes_no_sense_is_refused() {
        for (arguments, expected) in [
            (&["--minutes"][..], "needs a number"),
            (&["--minutes", "soon"], "is not a number of minutes"),
            (&["--minutes", "-1"], "is not a number of minutes"),
            (&["--minutes", "1e300"], "is not a number of minutes"),
            (&["--check", "--report"], "say one of"),
            (&["--fast"], "unknown option `--fast`"),
            (&["--tests"], "`--tests` is for use with `--check`"),
            (
                &["--list", "--fix-in"],
                "`--fix-in` is for use with `--check`",
            ),
            (
                &["--check", "--write"],
                "`--write` is for use with `--fix-in`",
            ),
            (&["--find", "x"], "are for use with `--break FILE`"),
            (&["--test", "a::b"], "are for use with `--break FILE`"),
            (
                &["--break", "f.rs"],
                "needs `--find CODE` (or `--in-string TEXT`) and `--replace CODE`",
            ),
            (
                &["--break", "f.rs", "--find", "a"],
                "needs `--find CODE` (or `--in-string TEXT`) and `--replace CODE`",
            ),
            (
                &[
                    "--break",
                    "f.rs",
                    "--find",
                    "a",
                    "--replace",
                    "b",
                    "step-12",
                ],
                "does not go with a break's or a list's name",
            ),
            (
                &[
                    "--break",
                    "f.rs",
                    "--find",
                    "a",
                    "--replace",
                    "b",
                    "--file",
                    "x.breaks",
                ],
                "does not go with a break's or a list's name",
            ),
            (
                &[
                    "--break",
                    "f.rs",
                    "--find",
                    "a",
                    "--replace",
                    "b",
                    "--flagged",
                ],
                "do not go together",
            ),
            (
                &[
                    "--break",
                    "f.rs",
                    "--find",
                    "a",
                    "--replace",
                    "b",
                    "--occurrence",
                    "0",
                ],
                "is not a number counted from 1",
            ),
            (&["--break"], "needs a value"),
            (&["--merge"], "needs a step's name"),
            (
                &["--merge", "step-9", "--flagged"],
                "goes with no other option",
            ),
            (&["--merge", "step-9", "--check"], "say one of"),
            (&["--dry-run"], "`--dry-run` is for use with `--merge`"),
            (&["--check", "--flagged"], "`--flagged` is for a run"),
            (&["--changed"], "needs a branch, a tag or a commit"),
            (
                &["--changed", "--list"],
                "a branch, a tag or a commit, not an option",
            ),
            (
                &["--changed", "main", "--check"],
                "`--changed` is for a run, `--list` or `--report`",
            ),
            (
                &[
                    "--changed",
                    "main",
                    "--break",
                    "f.rs",
                    "--find",
                    "a",
                    "--replace",
                    "b",
                ],
                "`--changed` chooses among the breaks of lists",
            ),
            (
                &["--changed", "main", "--merge", "step-9"],
                "`--merge` goes with no other option",
            ),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    /// A source with strings: the same words in code and in literals.
    const STRINGS: &str = "\
pub fn kind(code: u16) -> &'static str {
    let overloaded = code == 503;
    match code {
        413 => \"too large\",
        503 => \"overloaded, retry\",
        _ => \"other\",
    }
}

pub fn raw() -> &'static str {
    r#\"a \"raw\" overloaded one\"#
}

pub fn budget() -> &'static str {
    \"budget\"
}
";

    /// The problems of `list` against `STRINGS`, in a tree named for `test`.
    fn string_problems(test: &str, list: &str) -> Vec<String> {
        let root = super::drive::tests::tree(
            test,
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/lib.rs", STRINGS),
            ],
        );
        let problems = check(&root, &breaks("demo", list));
        let _ = std::fs::remove_dir_all(&root);
        problems
    }

    fn in_string(body: &str) -> Break {
        one(&format!("{body}\n"))
    }

    #[test]
    fn an_in_string_changes_a_word_in_a_literal_and_nothing_else_of_it() {
        let broken = apply(
            &in_string("in: kind\nin-string: retry\nreplace: give up"),
            STRINGS,
        )
        .unwrap();

        assert_eq!(
            broken,
            STRINGS.replace("overloaded, retry", "overloaded, give up")
        );
        // The same text laid out in another way in a raw string, by its
        // content as written.
        let broken = apply(
            &in_string("in-string: \"raw\" overloaded\nreplace: \"raw\" busy"),
            STRINGS,
        )
        .unwrap();
        assert!(broken.contains("a \"raw\" busy one"), "{broken}");
        // An empty `replace` takes the text out.
        let broken = apply(&in_string("in-string: , retry\nreplace:"), STRINGS).unwrap();
        assert!(broken.contains("\"overloaded\","), "{broken}");
    }

    #[test]
    fn an_in_string_is_looked_for_in_literals_only_and_must_be_there_once() {
        // `overloaded` is a name in the code, and in two literals.
        let error = apply(&in_string("in-string: overloaded\nreplace: busy"), STRINGS).unwrap_err();
        assert!(
            error.contains("its `in-string` text is in `src/lib.rs` 2 times, at lines 5, 11"),
            "{error}"
        );
        assert!(error.contains("`occurrence:`"), "{error}");
        let second = apply(
            &in_string("in-string: overloaded\nreplace: busy\noccurrence: 2"),
            STRINGS,
        )
        .unwrap();
        assert!(second.contains("a \"raw\" busy one"), "{second}");
        assert!(second.contains("\"overloaded, retry\""), "{second}");
        // Code is not a string: a name that is no literal's text is not found.
        let error = apply(&in_string("in-string: code\nreplace: x"), STRINGS).unwrap_err();
        assert!(
            error.contains("its `in-string` text is not in any string literal of `src/lib.rs`"),
            "{error}"
        );
        // The item narrows it, and a text outside the item is not found there.
        let error =
            apply(&in_string("in: raw\nin-string: retry\nreplace: x"), STRINGS).unwrap_err();
        assert!(error.contains("inside `raw`"), "{error}");
        let error = apply(
            &in_string("in-string: too large\nreplace: x\noccurrence: 3"),
            STRINGS,
        )
        .unwrap_err();
        assert!(error.contains("asks for occurrence 3"), "{error}");
    }

    #[test]
    fn an_in_string_that_would_not_leave_one_string_or_change_nothing_is_refused() {
        let same = apply(&in_string("in-string: retry\nreplace: retry"), STRINGS).unwrap_err();
        assert!(same.contains("changes nothing"), "{same}");
        for replace in ["re\"try", "retry\\", "\"; drop(); \""] {
            let error = apply(
                &in_string(&format!("in-string: retry\nreplace: {replace}")),
                STRINGS,
            )
            .unwrap_err();
            assert!(error.contains("not one string"), "{replace}: {error}");
        }
        // A quote with its backslash is part of the text.
        let broken = apply(
            &in_string("in-string: retry\nreplace: say \\\"x\\\""),
            STRINGS,
        )
        .unwrap();
        assert!(broken.contains("\"overloaded, say \\\"x\\\"\""), "{broken}");
    }

    #[test]
    fn an_in_string_that_makes_the_change_of_another_break_is_refused_as_a_copy() {
        let list = "\
[break] whole
rule: r
file: crates/demo/src/lib.rs
in: budget
find: \"budget\"
replace: \"other\"

[break] word
rule: r
file: crates/demo/src/lib.rs
in: budget
in-string: budget
replace: other

[break] elsewhere
rule: r
file: crates/demo/src/lib.rs
in: kind
in-string: retry
replace: give up
";
        let problems = string_problems("string-copy", list);

        assert_eq!(problems.len(), 1, "{problems:#?}");
        assert!(
            problems[0].starts_with(
                "mutations/demo.breaks:8: break `demo/word` makes the same change as `demo/whole`"
            ),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn a_comma_at_the_end_of_an_in_string_is_text_and_not_a_layout_comma() {
        let list = "\
[break] comma
rule: r
file: crates/demo/src/lib.rs
in: kind
in-string: overloaded,
replace: overloaded;
";
        assert_eq!(string_problems("string-comma", list), [] as [String; 0]);
    }

    #[test]
    fn a_break_given_with_an_in_string_is_checked_like_one_of_a_list() {
        let given = |arguments: &[&str]| {
            Request::parse(&strings(arguments)).and_then(|request| request.one_off.to_break())
        };

        let one = given(&[
            "--break",
            "crates/demo/src/lib.rs",
            "--in-string",
            "retry",
            "--replace",
            "give up",
        ])
        .unwrap();
        assert!(one.in_string);
        assert_eq!(one.find, "retry");
        assert!(one.id.starts_with("one-off/lib-"), "{}", one.id);
        let root = super::drive::tests::tree(
            "one-off-in-string",
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/lib.rs", STRINGS),
            ],
        );
        assert_eq!(check(&root, std::slice::from_ref(&one)), [] as [String; 0]);
        let _ = std::fs::remove_dir_all(&root);
        // The same words with `--find` are another break.
        let plain = given(&[
            "--break",
            "crates/demo/src/lib.rs",
            "--find",
            "retry",
            "--replace",
            "give up",
        ])
        .unwrap();
        assert_ne!(plain.id, one.id);
        for (arguments, expected) in [
            (
                &[
                    "--break",
                    "f.rs",
                    "--find",
                    "a",
                    "--in-string",
                    "b",
                    "--replace",
                    "c",
                ][..],
                "`--find` and `--in-string` do not go together",
            ),
            (
                &["--break", "f.rs", "--replace", "c"][..],
                "needs `--find CODE` (or `--in-string TEXT`)",
            ),
            (
                &["--in-string", "b", "--replace", "c"][..],
                "are for use with `--break FILE`",
            ),
        ] {
            let error = Request::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn changed_narrows_a_run_and_goes_with_what_narrows_or_lists() {
        let request = Request::parse(&strings(&[
            "--changed",
            "origin/main",
            "--file",
            "review.breaks",
            "--flagged",
            "--again",
            "--jobs",
            "2",
            "--minutes",
            "0",
            "step-12",
        ]))
        .unwrap();
        assert_eq!(request.changed.as_deref(), Some("origin/main"));
        assert_eq!(request.only, strings(&["step-12"]));
        for mode in ["--list", "--report"] {
            let request = Request::parse(&strings(&[mode, "--changed", "HEAD~3"])).unwrap();
            assert_eq!(request.changed.as_deref(), Some("HEAD~3"));
        }
        assert_eq!(Request::parse(&[]).unwrap().changed, None);
    }
}
