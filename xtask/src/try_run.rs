//! `cargo xtask try`: what the `try` workflow runs on a hosted runner, for
//! a branch with no pull request (`docs/testing-and-ci.md`, "A run on the
//! runners without a pull request").
//!
//! The workflow hands over its inputs as they were typed, each as one
//! argument, and this turns them into one of a few commands. Doing it here
//! and not in the workflow's script keeps the rules in one place, on both
//! OSes, under unit tests; and a typed input is only ever split into words
//! and checked, never handed to a shell.

use std::process::ExitCode;

use crate::Result;
use crate::workspace::Package;

const USAGE: &str = "\
usage: cargo xtask try --command 'COMMAND [ARGUMENTS]' [--profile ci|release|dist]
                       [--count N] [--load N] [--cores K] [--filter 'EXPR; EXPR']
                       [--against REF] [--label NAME] [--commit SHA] [--dry-run]
                       [--os OS] [--testkit none|profile|starts] [--capture yes|no]
       cargo xtask try --final [--against REF] [--count N] [--commit SHA]
                       [--dry-run | --dispatch BRANCH]
                       the checks before a pull request, in one run: the
                       Windows A/B against REF (main when not given), the
                       tests on Linux 22.04, and the binary's size against
                       main's; `try-result RUN` reads the three jobs
       cargo xtask try --dispatch BRANCH --command '...' [the same inputs]
                       [--queued-warn MIN]
                       checks the inputs as a dry run does, resolves --against
                       REF to its commit as GitHub has it, dispatches the
                       workflow on BRANCH with them (`gh workflow run try`),
                       prints the run's id, and names any run of the workflow
                       on BRANCH that has sat queued with no jobs for longer
                       than MIN minutes (10; --queued-limit is the same)

COMMAND is one of:
  test [-p CRATE]... [FILTER]...   the tests, as the blocking job builds and runs
                                   them, with the tier report; with --count,
                                   `stress` with that many repeats
  test --job [--setup-base FILE] [--setup-head FILE] [--keep-build] [-p CRATE]...
                                   with --against: `ab --job`, the whole job
                                   (build, then the tests) of both trees,
                                   --count rounds (2 when not given; an even
                                   number, each side first as often), each
                                   side with its own setup script, and the
                                   median of each phase
  final                            what --final runs (each job is told its
                                   part with --part ab|tests|size)
  stress -p CRATE... [ARGS]...     the tests many times over (--count repeats)
  startup                          start-up of `cineris` built in the profile
                                   (release or dist), --count runs
  perf [ARGS]...                   the performance budgets (step 20)
  perf -p CRATE... [NAME]...       the crates' ignored `perf_` tests, in a release
                                   build (dist with --profile dist), one at a
                                   time, output shown, and nothing else; a NAME
                                   narrows them
  size [OPTION]...                 `cargo xtask size`: the binary's size and its
                                   crates (--target, --check, --against-main,
                                   --breakdown, --top N pass; the `profile` input
                                   chooses the build, release or dist, and ci
                                   is the command's own, dist; Linux builds
                                   the musl target unless --target says
                                   another; --at is refused: use `commit`)
  ci [ARGS]...                     any form of `cargo xtask ci`
  mutate BREAK... [OPTION]...      `cargo xtask mutate` of the breaks (a break,
                                   a list or the end of a name) on the runner,
                                   run to its end in one job (--minutes 0);
                                   the results and logs are kept; --jobs N,
                                   --budget TIME (the nearest breaks first),
                                   --again, --no-compile-check and --file PATH
                                   pass; --count, --load, --cores, --filter
                                   and --against are refused with it
  partial-landlock                 the nightly's job: cineris-sandbox's tests on
                                   a kernel with the partial Landlock (on
                                   os=linux-22.04, the one runner that has it)
  advisories                       the nightly's advisory check
  unused                           the nightly's unused-dependency check

The nightly's jobs take no words, no --filter, --count, --load, --cores or
--against, and run in the ci profile.

A word of `test` or `stress` that is not `-p CRATE` goes to nextest: a test's
name, or an option such as `--run-ignored only`. --filter takes nextest filter
expressions, separated by `;`, each passed as one `-E` argument, spaces and
all: `test(/shell::/) & !test(=slow); package(cineris-agent)`. nextest runs a
test that any of them matches.

--against REF makes `test` an A/B run: the workflow checks REF out beside the
branch, and `cargo xtask ab` builds both once and runs them in turn, --count
rounds (5 when not given), on one runner, then compares them by tier, crate
and test. --dispatch resolves REF to its full commit hash first, as GitHub has
it at the dispatch, and passes the hash, with REF as typed as the `label`
input: a run that waits for a runner still compares with the commit REF named
when it was dispatched, and its report still calls the base REF. A full hash
is passed as it is; a REF GitHub does not know is refused, and nothing is
dispatched. --label NAME (the `label` input) is what the report calls the
base, --against's value when not given. It runs with --keep-going: a round in which a test fails is recorded
and left out, the run goes on, and it ends as a failure after the comparison of
the rounds that finished.

`test --job` with --against REF is `ab --job`: each round builds each tree's
tests from its own clean build directory and runs them, and each phase (the
setup, the build, each run of the tests, and each tier's summed test time) is
timed; the medians are compared. A round that fails is recorded and the run
goes on. `try` gives it --minutes, so that it starts no round it would not
finish within the job's limit.

--final (or --command final) is one dispatch of three jobs, each told its part
by the workflow: `ab` on Windows (`test` against REF, main when not given,
--count rounds), `tests` on Linux 22.04 (`test`, on the partial Landlock's
kernel) and `size` on Linux (`size --check --against-main`). It takes
--against, --label, --count and --commit, and refuses the other inputs.

--against REF with `perf -p CRATE...` makes it a performance A/B run: `cargo xtask
perf-ab` builds the crates' ignored `perf_` tests of both trees once, runs them
in turn, --count rounds (3 when not given), on one runner, and compares the
measurements they print (`perf: NAME VALUE UNIT`). A NAME narrows the tests;
other options are refused.

--dependents beside -p, in `test`, `stress` or `perf -p`, is resolved here, as
`cargo xtask test --dependents -p CRATE` resolves it: the crates it added are
named, and the job runs every one of them.

--commit SHA (40 hex digits, lower case) makes the branch's side of the run the
tree of that commit, whose own `xtask` then runs the command: it must contain
this `try`. --ref stays the workflow file, and --against the other side.

A word that `try` cannot pass on to what it runs is refused before anything is
built, with the word and the reason. --dry-run does only that: it prints the
command the job would run, and fails as the job would; --os and --testkit are
the workflow's other inputs, which it checks too.

An empty value is the same as leaving the flag out.";

/// The commands a `try` run may run.
const COMMANDS: [&str; 11] = [
    "test",
    "final",
    "stress",
    "startup",
    "size",
    "perf",
    "ci",
    "mutate",
    "partial-landlock",
    "advisories",
    "unused",
];

/// The nightly workflow's jobs a `try` run may run, as they are: the job's
/// own `cargo xtask` command, with nothing added.
const NIGHTLY_JOBS: [&str; 3] = ["partial-landlock", "advisories", "unused"];

/// The filter of the ignored performance tests: a test whose name, below its
/// binary, is `perf_…` or ends a module path with one.
pub(crate) const PERF_TESTS: &str = "test(/(^|::)perf_/)";

/// The workflow's inputs.
#[derive(Clone, Debug, Default, PartialEq)]
struct Inputs {
    words: Vec<String>,
    profile: String,
    count: Option<u32>,
    load: Option<usize>,
    cores: Option<usize>,
    /// nextest filter expressions, each one `-E` argument.
    filters: Vec<String>,
    /// The ref an A/B run compares the branch with.
    against: Option<String>,
    /// The commit the branch's side of the run is, in place of the branch's
    /// head.
    commit: Option<String>,
    /// Print the command the job would run, and run nothing.
    dry_run: bool,
    /// Dispatch the workflow on this branch with the inputs, and print the
    /// run's id: at a desk, in place of `gh workflow run try`.
    dispatch: Option<String>,
    /// The workflow's own inputs, which a dry run checks and a run is not
    /// given.
    os: Option<String>,
    testkit: Option<String>,
    /// `capture no`: a passing test's output is shown (nextest's
    /// `--no-capture`, which runs the tests one at a time).
    show_output: bool,
    /// With `--dispatch`: the minutes a run of the workflow on the branch
    /// may sit queued with no jobs before it is named.
    queued_warn: Option<u64>,
    /// What an A/B's report calls the base: the ref as typed, when
    /// `against` is the hash a dispatch resolved it to.
    label: Option<String>,
    /// Which part of a final run this job is ([`FINAL_PARTS`]): the
    /// workflow gives it from the job's matrix entry.
    part: Option<String>,
}

/// One job of a final run (`try --final`): its part, which the workflow
/// passes as `--part`, the name of its matrix entry in `try.yml` (the job is
/// `try-<job>`, as any `try` job is, so that `try-result` reads it as it
/// reads the others), the runner it runs on, and what it is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FinalPart {
    part: &'static str,
    job: &'static str,
    runs_on: &'static str,
    what: &'static str,
}

/// The jobs of a final run. `try.yml`'s matrix for `command=final` must be
/// these, in this order: a test reads the workflow and holds it to them.
const FINAL_PARTS: [FinalPart; 3] = [
    FinalPart {
        part: "ab",
        job: "windows",
        runs_on: "windows-latest",
        what: "the Windows A/B, both trees' tests on one runner",
    },
    FinalPart {
        part: "tests",
        job: "linux-22.04",
        runs_on: "ubuntu-22.04",
        what: "the tests on Linux 22.04, the kernel with Landlock ABI 4",
    },
    FinalPart {
        part: "size",
        job: "linux",
        runs_on: "ubuntu-latest",
        what: "the binary's size, against its budget and main's",
    },
];

/// The ref a final run's A/B compares with when `--against` does not say.
const FINAL_AGAINST: &str = "main";

/// What a final run with no part to run is told.
const NO_PART: &str = "a final run is three jobs, each given its part by the workflow (`--part \
                       ab|tests|size`): `--dry-run` shows what each runs, and `--dispatch` \
                       starts them";

/// The `os` inputs of the workflow.
const OSES: [&str; 4] = ["both", "linux", "windows", "linux-22.04"];

/// The `testkit` inputs of the workflow.
const TESTKITS: [&str; 3] = ["none", "profile", "starts"];

/// Whether `text` is a full commit hash: 40 hexadecimal digits, in lower
/// case. A short one, or a name, is not: the workflow checks the commit out
/// by what it says, and a name is the ref's.
fn commit_hash(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where the workflow puts the ref of an A/B run: beside the branch's
/// checkout, not in it, so that the branch's `.cargo/config.toml` does not
/// apply to its build.
pub(crate) const AB_BASE: &str = "../ab-base";

/// Whether `expression` may be passed to nextest as a filter: what
/// [`plain`] allows, and the spaces, brackets and marks of a filter or a
/// regular expression. No quote, `$`, `;`, back-tick or control character.
fn filter_text(expression: &str) -> bool {
    !expression.trim().is_empty()
        && expression.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b" _-:./=,()|&!~*+@#^[]{}?\\".contains(&byte)
        })
}

/// Whether `word` may reach a command line: letters, digits and the marks
/// a crate's name, a test's name, a path or a nextest filter needs. No
/// quote, no `$`, no `;`, no space: a word is passed as one argument and
/// never to a shell, and this keeps it that way should either change.
fn plain(word: &str) -> bool {
    !word.is_empty()
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-:./=,()|&!~*+@#".contains(&byte))
}

fn number<T>(flag: &str, text: &str, most: T) -> Result<Option<T>>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display + Default,
{
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<T>() {
        Ok(value) if value > T::default() && value <= most => Ok(Some(value)),
        _ => Err(format!("`{flag} {text}`: a number from 1 to {most}")),
    }
}

impl Inputs {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut inputs = Inputs {
            profile: "ci".to_owned(),
            ..Inputs::default()
        };
        let mut rest = arguments.iter();
        while let Some(flag) = rest.next() {
            if flag == "--help" || flag == "-h" {
                return Err(USAGE.to_owned());
            }
            if flag == "--dry-run" {
                inputs.dry_run = true;
                continue;
            }
            if flag == "--final" {
                inputs.words = vec!["final".to_owned()];
                continue;
            }
            let value = rest
                .next()
                .ok_or_else(|| format!("`{flag}` needs a value\n\n{USAGE}"))?;
            match flag.as_str() {
                "--command" => {
                    inputs.words = value.split_whitespace().map(str::to_owned).collect();
                }
                "--profile" => {
                    let profile = value.trim();
                    if !profile.is_empty() {
                        inputs.profile = profile.to_owned();
                    }
                }
                "--count" => inputs.count = number("--count", value, 10_000)?,
                "--load" => inputs.load = number("--load", value, 64)?,
                "--cores" => inputs.cores = number("--cores", value, 64)?,
                "--filter" => {
                    for expression in value.split(';').map(str::trim) {
                        if expression.is_empty() {
                            continue;
                        }
                        if !filter_text(expression) {
                            return Err(format!(
                                "`{expression}` has a character a filter does not pass on: \
                                 letters, digits, spaces and _-:./=,()|&!~*+@#^[]{{}}?\\ only"
                            ));
                        }
                        inputs.filters.push(expression.to_owned());
                    }
                }
                "--against" => {
                    let against = value.trim();
                    if !against.is_empty() {
                        if !plain(against) {
                            return Err(format!("`--against {against}` is not a ref"));
                        }
                        inputs.against = Some(against.to_owned());
                    }
                }
                "--commit" => {
                    let commit = value.trim();
                    if !commit.is_empty() {
                        if !commit_hash(commit) {
                            return Err(format!(
                                "`--commit {commit}` is not a commit: the full 40 hexadecimal \
                                 digits of its hash, in lower case (a branch or a short hash is \
                                 not one: `git rev-parse <name>` gives it)"
                            ));
                        }
                        inputs.commit = Some(commit.to_owned());
                    }
                }
                "--os" => {
                    let os = value.trim();
                    if !os.is_empty() {
                        if !OSES.contains(&os) {
                            return Err(format!("`--os {os}`: {}", OSES.join(", ")));
                        }
                        inputs.os = Some(os.to_owned());
                    }
                }
                "--testkit" => {
                    let testkit = value.trim();
                    if !testkit.is_empty() {
                        if !TESTKITS.contains(&testkit) {
                            return Err(format!("`--testkit {testkit}`: none, profile or starts"));
                        }
                        inputs.testkit = Some(testkit.to_owned());
                    }
                }
                "--dispatch" => {
                    let branch = value.trim();
                    if !plain(branch) {
                        return Err(format!("`--dispatch {branch}` is not a branch's name"));
                    }
                    inputs.dispatch = Some(branch.to_owned());
                    // What is dispatched is checked as a dry run checks it.
                    inputs.dry_run = true;
                }
                "--queued-warn" | "--queued-limit" => {
                    inputs.queued_warn = Some(crate::runs::queued_warn_minutes(flag, value)?);
                }
                "--label" => {
                    let label = value.trim();
                    if !label.is_empty() {
                        if !plain(label) {
                            return Err(format!("`--label {label}` is not a ref's name"));
                        }
                        inputs.label = Some(label.to_owned());
                    }
                }
                "--part" => {
                    let part = value.trim();
                    if !FINAL_PARTS.iter().any(|each| each.part == part) {
                        return Err(format!(
                            "`--part {part}`: {}, the part of a final run a job is",
                            FINAL_PARTS.map(|each| each.part).join(", ")
                        ));
                    }
                    inputs.part = Some(part.to_owned());
                }
                "--capture" => match value.trim() {
                    "" | "yes" => inputs.show_output = false,
                    "no" => inputs.show_output = true,
                    other => {
                        return Err(format!(
                            "`--capture {other}`: yes (nextest keeps a passing test's output) \
                             or no (it is shown, the tests one at a time)"
                        ));
                    }
                },
                other => return Err(format!("unknown flag `{other}`\n\n{USAGE}")),
            }
        }
        if inputs.is_final() {
            inputs.check_final()?;
            // A dispatch or a dry run compares with main when not told. A
            // job, told its part, is given `against` by the workflow, which
            // checks out only what it names: a job that defaulted it would
            // look for a base that is not there.
            if inputs.part.is_none() {
                inputs
                    .against
                    .get_or_insert_with(|| FINAL_AGAINST.to_owned());
            }
        } else if inputs.part.is_some() {
            return Err(
                "`--part` is the part of a final run a job is: it goes with `--final`".to_owned(),
            );
        }
        if inputs.label.is_some() && inputs.against.is_none() {
            return Err(
                "`--label` goes with `--against`: it is what an A/B's report calls the base"
                    .to_owned(),
            );
        }
        if inputs.queued_warn.is_some() && inputs.dispatch.is_none() {
            return Err(
                "`--queued-warn` goes with `--dispatch`: it is how long a run it finds may sit \
                 queued"
                    .to_owned(),
            );
        }
        if !["ci", "release", "dist"].contains(&inputs.profile.as_str()) {
            return Err(format!(
                "`--profile {}`: ci, release or dist",
                inputs.profile
            ));
        }
        if let Some(word) = inputs.words.iter().find(|word| !plain(word)) {
            return Err(format!(
                "`{word}` has a character a try run does not pass on: letters, digits and \
                 _-:./=,()|&!~*+@# only, with words separated by spaces"
            ));
        }
        Ok(inputs)
    }

    /// The `cargo xtask` arguments the inputs come to, checked: a word that
    /// the command cannot take is refused here, before anything is built.
    fn command(&self) -> Result<Vec<String>> {
        let command = self.build()?;
        self.refuse_words()?;
        validate(&command)?;
        Ok(command)
    }

    /// The `cargo xtask` arguments the inputs come to.
    fn build(&self) -> Result<Vec<String>> {
        let Some((verb, rest)) = self.words.split_first() else {
            return Err(format!("say what to run\n\n{USAGE}"));
        };
        if !COMMANDS.contains(&verb.as_str()) {
            return Err(format!(
                "`{verb}` is not a command a try run runs\n\n{USAGE}"
            ));
        }
        if verb == "final" {
            let Some(part) = &self.part else {
                return Err(NO_PART.to_owned());
            };
            return self.for_part(part)?.command();
        }
        if !self.filters.is_empty() && !matches!(verb.as_str(), "test" | "stress") {
            return Err("`--filter` chooses tests: it goes with `test` or `stress`".to_owned());
        }
        if self.show_output {
            if !matches!(verb.as_str(), "test" | "stress") {
                return Err(
                    "`--capture no` shows a passing test's output: it goes with `test` or \
                     `stress` (`perf -p` shows it already)"
                        .to_owned(),
                );
            }
            if self.against.is_some() {
                return Err(
                    "`--capture no` runs the tests one at a time, which an A/B's comparison of \
                     the two trees' runs is not made of: leave it out of an `against` run"
                        .to_owned(),
                );
            }
        }
        if verb == "mutate" {
            return self.mutate(rest);
        }
        if NIGHTLY_JOBS.contains(&verb.as_str()) {
            return self.nightly_job(verb, rest);
        }
        if verb == "size" {
            return self.size(rest);
        }
        if let Some(against) = &self.against {
            if verb == "perf" {
                return self.perf_ab(rest, against);
            }
            return self.ab(verb, rest, against);
        }
        let squeezed = self.load.is_some() || self.cores.is_some();
        if squeezed && !(verb == "stress" || verb == "test" && self.count.is_some()) {
            return Err(
                "`--load` and `--cores` go with a stress run: `stress`, or `test` \
                        with a count"
                    .to_owned(),
            );
        }
        let owned = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        if verb == "perf" {
            let (packages, others) = packages_and_others(rest)?;
            if !packages.is_empty() {
                return self.perf_tests(packages, others);
            }
        }
        let mut command = Vec::new();
        match verb.as_str() {
            "test" | "stress" if verb == "stress" || self.count.is_some() => {
                command.push("stress".to_owned());
                if !rest.iter().any(|word| word == "--count") {
                    command.extend(owned(&["--count", &self.count.unwrap_or(100).to_string()]));
                }
                command.extend(owned(&["--cargo-profile", &self.profile]));
                command.extend(self.with_filters(rest.to_vec()));
            }
            "test" if self.profile == "ci" => {
                // The blocking job's build and run, and the tier report:
                // packages before `--`, the rest for nextest after it.
                command.extend(owned(&["ci", "--tiers", "--top", "20"]));
                let (packages, others) = packages_and_others(rest)?;
                for package in packages {
                    command.extend(["-p".to_owned(), package]);
                }
                let others = self.with_filters(others);
                if !others.is_empty() {
                    command.push("--".to_owned());
                    command.extend(others);
                }
            }
            "test" => {
                command.extend(owned(&[
                    "test",
                    "--cargo-profile",
                    &self.profile,
                    "--profile",
                    "ci",
                ]));
                command.extend(self.with_filters(rest.to_vec()));
            }
            "startup" => {
                if self.profile == "ci" {
                    return Err(
                        "start-up is measured on a release build: choose the release or \
                                the dist profile"
                            .to_owned(),
                    );
                }
                command.extend(owned(&["startup", "--profile", &self.profile]));
                if let Some(count) = self.count {
                    command.extend(owned(&["--runs", &count.to_string()]));
                }
                command.extend(rest.iter().cloned());
            }
            _ => {
                // `perf` and `ci` take their words as they are.
                command.push(verb.clone());
                command.extend(rest.iter().cloned());
            }
        }
        if let Some(load) = self.load {
            command.extend(owned(&["--load", &load.to_string()]));
        }
        if let Some(cores) = self.cores {
            command.extend(owned(&["--cores", &cores.to_string()]));
        }
        Ok(command)
    }
}

/// What nextest does not take, as a word of a `test`, `stress` or `perf -p`
/// command, and why: they are options of another command, or something a
/// `try` input says. Passed on, each would reach nextest after the build and
/// fail the job with no test run.
const NOT_FOR_NEXTEST: [(&str, &str); 27] = [
    (
        "--job",
        "it makes an A/B a job A/B (`ab --job`): it goes with the `against` input",
    ),
    (
        "--setup-base",
        "it is an option of a job A/B (`ab --job`): it goes with `--job` and the `against` input",
    ),
    (
        "--setup-head",
        "it is an option of a job A/B (`ab --job`): it goes with `--job` and the `against` input",
    ),
    (
        "--keep-build",
        "it is an option of a job A/B (`ab --job`): it goes with `--job` and the `against` input",
    ),
    (
        "--tiers",
        "it is an option of `cargo xtask ci`, and a `test` run already ends with the tier \
         report (use the `ci` command)",
    ),
    (
        "--top",
        "it is an option of `cargo xtask ci --tiers`: a `test` run lists 20 tests a tier",
    ),
    (
        "--like-runner",
        "it is an option of `cargo xtask ci --tiers` (use the `ci` command)",
    ),
    (
        "--from",
        "it is an option of `cargo xtask ci --tiers` (use the `ci` command)",
    ),
    (
        "--compare",
        "it is an option of `cargo xtask ci --tiers` (use the `ci` command)",
    ),
    (
        "--factors-from",
        "it is an option of `cargo xtask ci --tiers` (use the `ci` command)",
    ),
    (
        "--cargo-profile",
        "the `profile` input chooses cargo's profile",
    ),
    (
        "--profile",
        "`try` chooses nextest's profile, and the `profile` input cargo's",
    ),
    (
        "-P",
        "`try` chooses nextest's profile, and the `profile` input cargo's",
    ),
    (
        "--count",
        "it is the `count` input, and makes the run a stress run",
    ),
    (
        "--first-seed",
        "it is an option of `cargo xtask stress`: use the `stress` command",
    ),
    ("--load", "it is the `load` input, for a stress run"),
    ("--cores", "it is the `cores` input, for a stress run"),
    (
        "--keep-going",
        "it is an option of `cargo xtask ab`, which `try` gives an `against` run itself",
    ),
    (
        "--base",
        "it is an option of `cargo xtask ab`, which `try` runs for the `against` input",
    ),
    (
        "--label",
        "it is an option of `cargo xtask ab`, which `try` runs for the `against` input",
    ),
    ("--against", "it is the `against` input"),
    (
        "--filter",
        "it is the `filter` input (nextest's own is `-E`)",
    ),
    ("--commit", "it is the `commit` input"),
    (
        "--workspace",
        "the run is of the whole workspace when no crate is named, and nextest refuses it \
         given twice",
    ),
    (
        "--locked",
        "every run `try` makes is `--locked` already, and nextest refuses it given twice",
    ),
    (
        "--no-tests",
        "`try`'s runs set what a run that finds no test does, and nextest refuses it given \
         twice",
    ),
    (
        "--dry-run",
        "it is an option of `cargo xtask try`, not of the command",
    ),
];

/// The options of a stress run that are the command's own when it is one.
const STRESS_OWN: [&str; 4] = ["--count", "--first-seed", "--load", "--cores"];

/// Refuses a word of `words` that nextest would not take, before the part
/// of them that follows a `--` of their own (which is the test binaries').
/// `into_stress`: the command is a stress run, which takes the options of
/// [`STRESS_OWN`] itself.
fn refuse_foreign_words(words: &[String], into_stress: bool) -> Result<()> {
    let words = words.iter().take_while(|word| *word != "--");
    for word in words {
        let flag = word.split_once('=').map_or(word.as_str(), |(flag, _)| flag);
        // A short option with its value joined to it: `-Pfast`.
        let flag = if flag.starts_with("-P") && !flag.starts_with("--") {
            "-P"
        } else {
            flag
        };
        let Some((_, why)) = NOT_FOR_NEXTEST.iter().find(|(name, _)| *name == flag) else {
            continue;
        };
        if into_stress && STRESS_OWN.contains(&flag) {
            continue;
        }
        return Err(format!(
            "`{word}` cannot be passed on to the tests: {why}. Nothing was built."
        ));
    }
    Ok(())
}

/// Refuses what the command `command` (after `try` has made it) would
/// refuse itself, by reading its arguments as it does, and running nothing.
fn validate(command: &[String]) -> Result<()> {
    let Some((verb, arguments)) = command.split_first() else {
        return Ok(());
    };
    let refused = |error: String| {
        let first = error.lines().next().unwrap_or_default();
        format!("`cargo xtask {verb}` would refuse this: {first}. Nothing was built.")
    };
    match verb.as_str() {
        "ci" => crate::checks::check_ci_arguments(arguments).map_err(refused),
        "stress" => crate::stress::check_arguments(arguments).map_err(refused),
        "ab" => crate::ab::check_arguments(arguments).map_err(refused),
        "perf" => crate::perf::check_arguments(arguments).map_err(refused),
        "perf-ab" => crate::perf_ab::check_arguments(arguments).map_err(refused),
        "size" => crate::nonblocking::check_arguments(arguments, cfg!(target_os = "linux"))
            .map_err(refused),
        "mutate" => crate::mutate::check_arguments(arguments).map_err(refused),
        "startup" => check_startup(arguments).map_err(refused),
        _ => Ok(()),
    }
}

/// Whether `cargo xtask startup` takes `arguments`: the profile's build
/// (the runner has no binary to name), the runs, the budget column and the
/// check. The profile is set by the `profile` input.
fn check_startup(arguments: &[String]) -> Result<()> {
    let mut seen_profile = false;
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        let mut value = |what: &str| words.next().ok_or_else(|| format!("`{word}` needs {what}"));
        match word.as_str() {
            "--runs" => {
                let text = value("a number")?;
                if !text.parse::<u32>().is_ok_and(|runs| runs > 0) {
                    return Err(format!("`--runs {text}` is not a number above zero"));
                }
            }
            "--budget" => {
                let text = value("a column")?;
                if !["desktop", "linux", "windows"].contains(&text.as_str()) {
                    return Err(format!("`--budget {text}`: desktop, linux or windows"));
                }
            }
            "--check" => {}
            "--workspace" => {
                value("a directory")?;
            }
            "--profile" if !seen_profile => {
                value("a cargo profile")?;
                seen_profile = true;
            }
            "--profile" => {
                return Err(
                    "a second `--profile`: the `profile` input chooses the build".to_owned(),
                );
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            path => {
                return Err(format!(
                    "`{path}`: a try run measures a build of its own, and names no binary"
                ));
            }
        }
    }
    Ok(())
}

impl Inputs {
    /// Whether this is a final run (`--final`, or `--command final`).
    fn is_final(&self) -> bool {
        self.words.first().is_some_and(|verb| verb == "final")
    }

    /// Refuses what a final run does not take: its parts are fixed, so the
    /// inputs that would change one are an error and not something it
    /// quietly drops.
    fn check_final(&self) -> Result<()> {
        if self.words.len() > 1 {
            return Err(format!(
                "`{}`: a final run takes no words; its three parts are fixed",
                self.words.join(" ")
            ));
        }
        for (given, input) in [
            (!self.filters.is_empty(), "--filter"),
            (self.load.is_some(), "--load"),
            (self.cores.is_some(), "--cores"),
            (self.show_output, "--capture no"),
            (
                self.testkit.as_deref().is_some_and(|kit| kit != "none"),
                "--testkit",
            ),
            (
                self.os.is_some(),
                "--os (each part has its own runner: Windows, Linux 22.04 and Linux)",
            ),
            (
                self.profile != "ci",
                "--profile (the tests run in the ci profile, and the size is of the dist build)",
            ),
        ] {
            if given {
                return Err(format!(
                    "`{input}` does not go with a final run: it takes --against, --label, \
                     --count and --commit"
                ));
            }
        }
        Ok(())
    }

    /// The inputs of the part `part` of a final run: the A/B against
    /// `against` with the count, the tests, or the size, each with the
    /// commit, and nothing else.
    fn for_part(&self, part: &str) -> Result<Inputs> {
        let mut inputs = Inputs {
            profile: "ci".to_owned(),
            commit: self.commit.clone(),
            part: None,
            ..Inputs::default()
        };
        match part {
            "ab" => {
                inputs.words = vec!["test".to_owned()];
                inputs.against = Some(self.against.clone().ok_or(
                    "a final run's A/B job was given no `against`: `cargo xtask try \
                             --final --dispatch BRANCH` sends main, resolved to its commit; a \
                             dispatch by hand needs `-f against=main`. Nothing was built",
                )?);
                inputs.label = self.label.clone();
                inputs.count = self.count;
            }
            "tests" => inputs.words = vec!["test".to_owned()],
            "size" => {
                inputs.words = ["size", "--check", "--against-main"]
                    .map(str::to_owned)
                    .to_vec();
            }
            other => return Err(format!("`{other}` is not a part of a final run")),
        }
        Ok(inputs)
    }

    /// What each job of a final run runs, a line each: the job, what it
    /// is, and its command.
    fn final_lines(&self) -> Result<Vec<String>> {
        FINAL_PARTS
            .iter()
            .map(|part| {
                let command = self.for_part(part.part)?.command()?;
                Ok(format!(
                    "try-{} ({}, part {}): $ cargo xtask {}",
                    part.job,
                    part.what,
                    part.part,
                    command.join(" ")
                ))
            })
            .collect()
    }

    /// Refuses a word the command's words hold that nextest would not take
    /// ([`refuse_foreign_words`]).
    fn refuse_words(&self) -> Result<()> {
        let Some((verb, rest)) = self.words.split_first() else {
            return Ok(());
        };
        let tests = match verb.as_str() {
            "test" | "stress" => true,
            "perf" => !packages_and_others(rest)?.0.is_empty(),
            _ => false,
        };
        if !tests {
            return Ok(());
        }
        let into_stress =
            verb == "stress" || verb == "test" && self.count.is_some() && self.against.is_none();
        // A job A/B's own options are `ab --job`'s, not nextest's.
        let rest = if verb == "test" && self.against.is_some() {
            job_words(rest)?.1
        } else {
            rest.to_vec()
        };
        refuse_foreign_words(&rest, into_stress)
    }

    /// `--dependents` in the command's words resolved into `-p` for each
    /// crate that is named or can see a change to one, as `cargo xtask test
    /// --dependents -p CRATE` resolves it. `packages` reads the workspace's,
    /// and is asked only when the flag is there.
    fn resolve_dependents(
        &mut self,
        packages: impl FnOnce() -> Result<Vec<Package>>,
    ) -> Result<Option<crate::dependents::Expanded>> {
        let Some((verb, rest)) = self.words.split_first() else {
            return Ok(None);
        };
        let mut before = rest.iter().take_while(|word| *word != "--");
        if !before.any(|word| word == "--dependents") {
            return Ok(None);
        }
        if !matches!(verb.as_str(), "test" | "stress" | "perf") {
            return Err(format!(
                "`--dependents` adds the crates whose tests can see a change to the ones named with `-p`: it goes with `test`, `stress` or `perf -p`, not `{verb}`"
            ));
        }
        let expanded = crate::dependents::expand_in(rest, &packages()?)?;
        let Some(expanded) = expanded else {
            return Ok(None);
        };
        let verb = verb.clone();
        self.words = std::iter::once(verb)
            .chain(expanded.arguments.iter().cloned())
            .collect();
        Ok(Some(expanded))
    }

    /// `mutate`: `cargo xtask mutate` of the breaks named, run to its end in
    /// one job (`--minutes 0`: its time budget is for a desk's shared test
    /// lock, which a runner has no one to share). The results and the logs
    /// are in `target/mutate`, which the workflow keeps.
    fn mutate(&self, rest: &[String]) -> Result<Vec<String>> {
        for (given, flag) in [
            (self.against.is_some(), "--against"),
            (self.count.is_some(), "--count"),
            (self.load.is_some(), "--load"),
            (self.cores.is_some(), "--cores"),
        ] {
            if given {
                return Err(format!(
                    "`{flag}` does not go with `mutate`, which runs the breaks of the lists named once, to the end"
                ));
            }
        }
        if self.profile != "ci" {
            return Err(
                "`mutate` builds its own copy of the tree, as it does locally: the ci profile"
                    .to_owned(),
            );
        }
        let mut command: Vec<String> = ["mutate", "--minutes", "0"].map(str::to_owned).to_vec();
        let mut named = false;
        let mut words = rest.iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--check" | "--list" | "--report" | "--tests" | "--fix-in" | "--write" => {
                    return Err(format!(
                        "`{word}` runs no break: it is for a local run, which reads the lists and the tree. A try run runs the breaks"
                    ));
                }
                "--minutes" => {
                    return Err(
                        "`--minutes`: a try run gives `--minutes 0` itself, so that the run goes to its end in one job"
                            .to_owned(),
                    );
                }
                "--budget" => {
                    let text = words
                        .next()
                        .ok_or_else(|| format!("`{word}` needs a value"))?;
                    check_mutate_budget(text)?;
                    command.extend([word.clone(), text.clone()]);
                }
                "--jobs" | "-j" | "--file" => {
                    command.push(word.clone());
                    command.push(
                        words
                            .next()
                            .ok_or_else(|| format!("`{word}` needs a value"))?
                            .clone(),
                    );
                    named |= word == "--file";
                }
                "--again" | "--no-compile-check" => command.push(word.clone()),
                option if option.starts_with('-') => {
                    return Err(format!(
                        "`{option}` is not an option a try run gives `mutate`: --jobs, --budget, \
                         --again, --no-compile-check and --file"
                    ));
                }
                name => {
                    command.push(name.to_owned());
                    named = true;
                }
            }
        }
        if !named {
            return Err(
                "name the breaks to run (`mutate step-12`, or a break's name, or the end of one): the whole of every list is more than one job's time"
                    .to_owned(),
            );
        }
        Ok(command)
    }

    /// One of the nightly's jobs, as the nightly runs it.
    fn nightly_job(&self, verb: &str, rest: &[String]) -> Result<Vec<String>> {
        let added = !rest.is_empty()
            || self.count.is_some()
            || self.load.is_some()
            || self.cores.is_some()
            || self.against.is_some()
            || self.profile != "ci";
        if added {
            return Err(format!(
                "`{verb}` is a job of the nightly workflow and runs as it does there: no words \
                 after it, no --count, --load, --cores or --against, and the ci profile"
            ));
        }
        Ok(vec![verb.to_owned()])
    }

    /// `-E EXPR` for each filter.
    fn filter_arguments(&self) -> Vec<String> {
        self.filters
            .iter()
            .flat_map(|filter| ["-E".to_owned(), filter.clone()])
            .collect()
    }

    /// `words` for nextest, with `-E EXPR` for each filter where nextest
    /// reads it: before a `--` of the words' own, after which the words go
    /// to the test binary.
    fn with_filters(&self, mut words: Vec<String>) -> Vec<String> {
        let at = words
            .iter()
            .position(|word| word == "--")
            .unwrap_or(words.len());
        let mut nextest = self.filter_arguments();
        if self.show_output {
            // nextest runs the tests one at a time with it, so that their
            // output does not mix.
            nextest.push("--no-capture".to_owned());
        }
        words.splice(at..at, nextest);
        words
    }

    /// `perf -p CRATE…`: the crates' ignored `perf_` tests, in a release
    /// build, one at a time and with their output shown, and nothing else.
    /// A word after the crates narrows them further (nextest ANDs a test
    /// name with the filter).
    fn perf_tests(&self, packages: Vec<String>, others: Vec<String>) -> Result<Vec<String>> {
        if self.count.is_some() {
            return Err(
                "`--count` repeats a stress run: `perf -p CRATE` runs each perf test once"
                    .to_owned(),
            );
        }
        let profile = if self.profile == "dist" {
            "dist"
        } else {
            "release"
        };
        // nextest's `perf` profile: a measurement's limit is half an hour,
        // where tier 2's `ci` limit stops a test at 10 s.
        let mut command: Vec<String> = ["test", "--cargo-profile", profile, "--profile", "perf"]
            .map(str::to_owned)
            .to_vec();
        for package in packages {
            command.extend(["-p".to_owned(), package]);
        }
        command.extend(
            [
                "--run-ignored",
                "only",
                "-E",
                PERF_TESTS,
                "--no-capture",
                "--test-threads",
                "1",
            ]
            .map(str::to_owned),
        );
        command.extend(others);
        Ok(command)
    }

    /// `size`: `cargo xtask size`, which builds the binary and reports its
    /// size. The `profile` input chooses the build (`ci`, the input's default,
    /// is the command's own: the shipped `dist` build); a Linux runner builds
    /// the static target the budgets are of, unless `--target` says another.
    fn size(&self, rest: &[String]) -> Result<Vec<String>> {
        for (given, flag) in [
            (
                self.against.is_some(),
                "--against (it is `size`'s own `--against-main`)",
            ),
            (self.count.is_some(), "--count"),
            (self.load.is_some(), "--load"),
            (self.cores.is_some(), "--cores"),
        ] {
            if given {
                return Err(format!(
                    "`{flag}` does not go with `size`, which builds once"
                ));
            }
        }
        if let Some(word) = rest.iter().find(|word| word.as_str() == "--profile") {
            return Err(format!(
                "`{word}`: the `profile` input chooses the build, release or dist"
            ));
        }
        let mut command = vec!["size".to_owned()];
        command.extend(rest.iter().cloned());
        if self.profile != "ci" {
            command.extend(["--profile".to_owned(), self.profile.clone()]);
        }
        if cfg!(target_os = "linux") && !rest.iter().any(|word| word == "--target") {
            command.extend([
                "--target".to_owned(),
                crate::nonblocking::LINUX_TARGET.to_owned(),
            ]);
        }
        Ok(command)
    }

    /// `perf -p CRATE… --against REF`: the crates' ignored `perf_` tests of
    /// both trees, built once each and run in turn on this runner, and the
    /// measurements they print compared (`cargo xtask perf-ab`).
    fn perf_ab(&self, rest: &[String], against: &str) -> Result<Vec<String>> {
        let (packages, names) = packages_and_others(rest)?;
        if packages.is_empty() {
            return Err(
                "`--against` with `perf` compares the crates' `perf_` tests: name them with -p \
                 (`perf` alone is the terminal's measurement, which has no other tree)"
                    .to_owned(),
            );
        }
        if self.load.is_some() || self.cores.is_some() {
            return Err(
                "`--load` and `--cores` go with a stress run, not a performance A/B".to_owned(),
            );
        }
        if let Some(word) = names.iter().find(|word| word.starts_with('-')) {
            return Err(format!(
                "`{word}`: a performance A/B takes the crates and the names of tests, and no \
                 other option"
            ));
        }
        let profile = if self.profile == "dist" {
            "dist"
        } else {
            "release"
        };
        let mut command: Vec<String> = [
            "perf-ab",
            "--base",
            AB_BASE,
            "--label",
            &self.label_of(against),
            "--cargo-profile",
            profile,
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(count) = self.count {
            command.extend(["--count".to_owned(), count.max(3).to_string()]);
        }
        for package in packages {
            command.extend(["-p".to_owned(), package]);
        }
        command.extend(names);
        Ok(command)
    }

    /// What an A/B's report calls the base: the label given (the ref as the
    /// dispatch was given it), else `against` ([`label_of`]).
    fn label_of(&self, against: &str) -> String {
        self.label.clone().unwrap_or_else(|| label_of(against))
    }

    /// The A/B run: `ab` against the tree the workflow checked out.
    fn ab(&self, verb: &str, rest: &[String], against: &str) -> Result<Vec<String>> {
        if verb != "test" || self.profile != "ci" {
            return Err(
                "`--against` compares the blocking jobs' tests of two refs: it goes with \
                 `test`, in the ci profile"
                    .to_owned(),
            );
        }
        if self.load.is_some() || self.cores.is_some() {
            return Err("`--load` and `--cores` go with a stress run, not an A/B run".to_owned());
        }
        let (job, rest) = job_words(rest)?;
        if let Some(job) = job {
            return self.job_ab(job, &rest, against);
        }
        let mut command: Vec<String> = [
            "ab",
            "--base",
            AB_BASE,
            "--label",
            &self.label_of(against),
            "--top",
            "20",
            "--keep-going",
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(count) = self.count {
            command.extend(["--count".to_owned(), count.max(2).to_string()]);
        }
        let (packages, others) = packages_and_others(&rest)?;
        for package in packages {
            command.extend(["-p".to_owned(), package]);
        }
        let others = self.with_filters(others);
        if !others.is_empty() {
            command.push("--".to_owned());
            command.extend(others);
        }
        Ok(command)
    }

    /// `test --job` with `against`: `ab --job`, the whole job of both trees
    /// round by round, with the setups `job` names, and `--minutes` so that
    /// it starts no round it would not finish within the job's limit.
    fn job_ab(&self, job: JobWords, rest: &[String], against: &str) -> Result<Vec<String>> {
        if self.show_output {
            return Err("`--capture no` does not go with a job A/B".to_owned());
        }
        let mut command: Vec<String> = [
            "ab",
            "--job",
            "--base",
            AB_BASE,
            "--label",
            &self.label_of(against),
            "--minutes",
            &JOB_AB_MINUTES.to_string(),
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(count) = self.count {
            command.extend(["--count".to_owned(), count.to_string()]);
        }
        for (flag, file) in [
            ("--setup-base", job.setup_base),
            ("--setup-head", job.setup_head),
        ] {
            if let Some(file) = file {
                command.extend([flag.to_owned(), file]);
            }
        }
        if job.keep_build {
            command.push("--keep-build".to_owned());
        }
        let (packages, others) = packages_and_others(rest)?;
        for package in packages {
            command.extend(["-p".to_owned(), package]);
        }
        let others = self.with_filters(others);
        if !others.is_empty() {
            command.push("--".to_owned());
            command.extend(others);
        }
        Ok(command)
    }
}

/// The minutes a `mutate` run's step may take on a runner: its job's limit
/// in `try.yml` (90) less the room the steps before it take. The step has
/// this as its own limit, so that a run that overruns it fails and the
/// steps after it still upload what it found, which a job cancelled at its
/// limit does not.
const MUTATE_STEP_MINUTES: u64 = 80;

/// The room a `mutate --budget` leaves within its step: the baselines of
/// the class it is in, and the breaks under way when the budget is spent.
const MUTATE_BUDGET_ROOM: u64 = 30;

/// Refuses a `mutate --budget` longer than its step's limit allows: such a
/// run would be stopped by the runner, and everything it found lost.
fn check_mutate_budget(text: &str) -> Result<()> {
    let budget = crate::mutate::budget::parse_time(text)?;
    let most = MUTATE_STEP_MINUTES - MUTATE_BUDGET_ROOM;
    if budget > std::time::Duration::from_secs(most * 60) {
        return Err(format!(
            "`--budget {text}` is longer than a try run's `mutate` may take: {most} minutes at \
             most, within the step's {MUTATE_STEP_MINUTES}, so that the breaks under way finish \
             and the results are uploaded"
        ));
    }
    Ok(())
}

/// The minutes a job A/B on a runner may start rounds within: the job's
/// limit for it in `try.yml` (240) less a round's worth of room for the
/// last round, the summary and the upload.
const JOB_AB_MINUTES: u32 = 200;

// The rounds start within the run step's own limit for a job A/B (230).
const _: () = assert!(JOB_AB_MINUTES < 230);

/// The words of `test` that make an A/B a job A/B (`ab --job`), taken out
/// of the words that go to nextest.
#[derive(Debug, Default, PartialEq)]
struct JobWords {
    setup_base: Option<String>,
    setup_head: Option<String>,
    keep_build: bool,
}

/// `--job` and the options that go with it, out of `words` (before a `--` of
/// their own): `Some` when `--job` is there. An option of a job A/B without
/// `--job` is refused, and so is one given twice.
fn job_words(words: &[String]) -> Result<(Option<JobWords>, Vec<String>)> {
    let mut job = JobWords::default();
    let mut asked = false;
    let mut options = Vec::new();
    let mut rest = Vec::new();
    let mut iter = words.iter();
    while let Some(word) = iter.next() {
        match word.as_str() {
            "--" => {
                rest.push(word.clone());
                rest.extend(iter.by_ref().cloned());
            }
            "--job" => asked = true,
            "--keep-build" => {
                options.push(word.clone());
                job.keep_build = true;
            }
            "--setup-base" | "--setup-head" => {
                let file = iter
                    .next()
                    .ok_or_else(|| format!("`{word}` needs a script's path in the tree"))?;
                options.push(word.clone());
                let slot = if word == "--setup-base" {
                    &mut job.setup_base
                } else {
                    &mut job.setup_head
                };
                if slot.replace(file.clone()).is_some() {
                    return Err(format!("`{word}` is given twice"));
                }
            }
            _ => rest.push(word.clone()),
        }
    }
    if !asked {
        if let Some(option) = options.first() {
            return Err(format!(
                "`{option}` is an option of a job A/B: it goes with `--job` (and `--against`)"
            ));
        }
        return Ok((None, rest));
    }
    Ok((Some(job), rest))
}

/// What an A/B run calls the other side in its report when no label says:
/// `against` as it was given, or a full commit hash's first twelve digits.
fn label_of(against: &str) -> String {
    if commit_hash(against) {
        against[..12].to_owned()
    } else {
        against.to_owned()
    }
}

/// `against` as a dispatch passes it: the full commit hash `resolve` gives
/// for the ref, as GitHub has it now, with the line that says so; a full
/// hash as it is, with nothing to say. A ref that does not resolve, or that
/// resolves to anything but a full hash, is an error naming the ref.
fn resolve_against(
    against: &str,
    resolve: impl FnOnce(&str) -> Result<String>,
) -> Result<(String, Option<String>)> {
    if commit_hash(against) {
        return Ok((against.to_owned(), None));
    }
    let resolved = resolve(against).map_err(|error| {
        format!(
            "`--against {against}` names no commit GitHub has: nothing was dispatched ({error})"
        )
    })?;
    let resolved = resolved.trim();
    if !commit_hash(resolved) {
        return Err(format!(
            "`--against {against}` resolved to `{resolved}`, which is not a full commit hash: \
             nothing was dispatched"
        ));
    }
    Ok((
        resolved.to_owned(),
        Some(format!(
            "against {against} is {} (resolved at dispatch)",
            &resolved[..12]
        )),
    ))
}

/// The inputs a dispatch sends: `against` resolved to its commit by
/// `resolve` ([`resolve_against`]), the ref as typed kept as the label the
/// report calls the base by (unless a label was given), and the line that
/// says so.
fn for_dispatch(
    mut inputs: Inputs,
    resolve: impl FnOnce(&str) -> Result<String>,
) -> Result<(Inputs, Option<String>)> {
    let mut said = None;
    if let Some(against) = inputs.against.take() {
        let (resolved, note) = resolve_against(&against, resolve)?;
        if let Some(note) = note {
            let label = inputs.label.get_or_insert_with(|| against.clone());
            said = Some(format!("{note}; the report calls it {label}"));
        }
        inputs.against = Some(resolved);
    }
    Ok((inputs, said))
}

/// `reference` as one segment of a URL's path: the characters of a ref's
/// name that need nothing kept as they are, `/` included (GitHub reads a
/// branch's slashes in a commit's path), and every other byte escaped.
fn url_path(reference: &str) -> String {
    let mut path = String::new();
    for byte in reference.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            path.push(char::from(byte));
        } else {
            path.push_str(&format!("%{byte:02X}"));
        }
    }
    path
}

/// The commit GitHub resolves `reference` to now: what the workflow's
/// checkout of the ref would take. Asked of GitHub and not of the local
/// repository: a desk's `main` may be behind GitHub's, and a copy of the
/// tree with no `.git` (the one `rx` builds in) has none.
/// GitHub answers with the hash alone when asked for it by its media type.
fn github_commit(reference: &str) -> Result<String> {
    crate::runs::gh(&[
        "api",
        "-H",
        "Accept: application/vnd.github.sha",
        &format!("repos/{{owner}}/{{repo}}/commits/{}", url_path(reference)),
    ])
}

/// The crates `words` name, in any of cargo's spellings (`-p x`,
/// `--package x`, `-px`, `--package=x`), and the rest of the words, which
/// go to nextest.
fn packages_and_others(words: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut packages = Vec::new();
    let mut others = Vec::new();
    let mut words = words.iter();
    while let Some(word) = words.next() {
        if word == "-p" || word == "--package" {
            packages.push(words.next().ok_or("`-p` needs a crate")?.clone());
        } else if let Some(package) = word.strip_prefix("--package=") {
            packages.push(package.to_owned());
        } else if let Some(package) = word.strip_prefix("-p").filter(|_| !word.starts_with("--")) {
            packages.push(package.to_owned());
        } else {
            others.push(word.clone());
        }
    }
    Ok((packages, others))
}

/// What the inputs come to: the inputs, the command the job runs, and what
/// is worth saying of how they were read.
#[derive(Debug)]
struct Plan {
    inputs: Inputs,
    command: Vec<String>,
    /// Lines to print before the command: the crates `--dependents` added,
    /// the commit, and in a dry run what the workflow's other inputs mean.
    notes: Vec<String>,
}

/// Reads the inputs, resolves `--dependents` (`packages` reads the
/// workspace's packages, and is asked only then) and makes the command, and
/// refuses what the job would refuse: all of it before anything is built.
fn plan(arguments: &[String], packages: impl FnOnce() -> Result<Vec<Package>>) -> Result<Plan> {
    let mut inputs = Inputs::parse(arguments)?;
    let mut notes = Vec::new();
    if let Some(expanded) = inputs.resolve_dependents(packages)? {
        notes.push(expanded.said());
    }
    let command = if inputs.is_final() && inputs.part.is_none() {
        // A dry run or a dispatch of a final run: what each of its jobs
        // will run, each checked as that job will check it.
        notes.extend(inputs.final_lines()?);
        if !inputs.dry_run {
            return Err(NO_PART.to_owned());
        }
        Vec::new()
    } else {
        inputs.command()?
    };
    if let Some(commit) = &inputs.commit {
        notes.push(format!(
            "commit {commit}: the branch's side of the run is that commit, whose own `xtask` \
             runs the command"
        ));
    }
    if inputs.dry_run {
        if inputs.dispatch.is_none()
            && let Some(against) = inputs.against.as_deref()
            && !commit_hash(against)
        {
            notes.push(format!(
                "against {against}: `try --dispatch` would resolve it to its commit, as GitHub \
                 has it then, and pass the hash; a dry run asks GitHub nothing"
            ));
        }
        if let Some(testkit @ ("profile" | "starts")) = inputs.testkit.as_deref()
            && inputs.against.is_some()
        {
            notes.push(format!(
                "testkit={testkit} is not set for an `against` run: the workflow leaves it out"
            ));
        }
        if inputs
            .words
            .first()
            .is_some_and(|verb| verb == "partial-landlock")
            && inputs.os.as_deref() != Some("linux-22.04")
        {
            notes.push(
                "partial-landlock runs on os=linux-22.04, the one runner whose kernel has the \
                 partial case"
                    .to_owned(),
            );
        }
    }
    Ok(Plan {
        inputs,
        command,
        notes,
    })
}

/// Whether a final run's part `part` may run here: its A/B is Windows', and
/// the others are Linux's. A job given another runner's part is a matrix
/// that went wrong, and must not measure the wrong system.
fn check_runner(part: &str, on_windows: bool) -> Result<()> {
    let wanted = FINAL_PARTS
        .iter()
        .find(|each| each.part == part)
        .ok_or_else(|| format!("`{part}` is not a part of a final run"))?;
    let windows = wanted.job == "windows";
    if windows == on_windows {
        Ok(())
    } else {
        Err(format!(
            "part {part} of a final run is {}, which runs on {}, and this runner is {}: \
             nothing was built",
            wanted.what,
            wanted.runs_on,
            if on_windows { "Windows" } else { "Linux" }
        ))
    }
}

/// Whether the checked-out tree is the commit asked for: `head` is what
/// `HEAD` names there. A run that was asked for one commit must never
/// measure another.
fn check_checkout(commit: &str, head: &str) -> Result<()> {
    if head.trim() == commit {
        Ok(())
    } else {
        Err(format!(
            "the checkout is at {}, not at the commit asked for ({commit}): nothing was built",
            head.trim()
        ))
    }
}

/// The commit `HEAD` names in this tree.
fn head_commit() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(crate::workspace::root())
        .output()
        .map_err(|error| format!("could not ask git for HEAD: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git could not say what HEAD is: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `cargo xtask try …`
/// The arguments of `gh` that dispatch the `try` workflow on `branch` with
/// `inputs`: each input that is set, as `-f name=value`, and nothing for
/// one left at its default.
fn dispatch_arguments(branch: &str, inputs: &Inputs) -> Vec<String> {
    let mut arguments: Vec<String> = ["workflow", "run", "try", "--ref", branch]
        .map(str::to_owned)
        .to_vec();
    let mut field = |name: &str, value: String| {
        arguments.push("-f".to_owned());
        arguments.push(format!("{name}={value}"));
    };
    field("command", inputs.words.join(" "));
    if let Some(os) = &inputs.os {
        field("os", os.clone());
    }
    if inputs.profile != "ci" {
        field("profile", inputs.profile.clone());
    }
    for (name, value) in [
        ("count", inputs.count.map(|value| value.to_string())),
        ("load", inputs.load.map(|value| value.to_string())),
        ("cores", inputs.cores.map(|value| value.to_string())),
    ] {
        if let Some(value) = value {
            field(name, value);
        }
    }
    if !inputs.filters.is_empty() {
        field("filter", inputs.filters.join("; "));
    }
    if let Some(against) = &inputs.against {
        field("against", against.clone());
    }
    if let Some(label) = &inputs.label {
        field("label", label.clone());
    }
    if let Some(commit) = &inputs.commit {
        field("commit", commit.clone());
    }
    if inputs.show_output {
        field("capture", "no".to_owned());
    }
    if let Some(testkit) = inputs
        .testkit
        .as_deref()
        .filter(|testkit| *testkit != "none")
    {
        field("testkit", testkit.to_owned());
    }
    arguments
}

/// The runs of `after` that are not in `before`: the ones a dispatch made.
fn new_runs(before: &[u64], after: &[u64]) -> Vec<u64> {
    after
        .iter()
        .filter(|run| !before.contains(run))
        .copied()
        .collect()
}

/// The ids of the `try` workflow's latest dispatched runs on `branch`.
fn dispatched_runs(branch: &str) -> Result<Vec<u64>> {
    let listed = crate::runs::gh(&[
        "run",
        "list",
        "--workflow",
        "try",
        "--branch",
        branch,
        "--event",
        "workflow_dispatch",
        "--limit",
        "20",
        "--json",
        "databaseId",
    ])?;
    let value: serde_json::Value =
        serde_json::from_str(&listed).map_err(|error| format!("gh run list: {error}"))?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| run.get("databaseId")?.as_u64())
        .collect())
}

/// The runs of `listed` (`gh run list --json databaseId,status,createdAt`)
/// that may have sat queued with no jobs past `limit_minutes` at `now`: those
/// not finished and older than the limit. Whether a job of one has started is
/// asked of each after ([`crate::runs::queued_note`]).
fn waiting_too_long(listed: &str, now: u64, limit_minutes: u64) -> Result<Vec<u64>> {
    let value: serde_json::Value =
        serde_json::from_str(listed).map_err(|error| format!("gh run list: {error}"))?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| {
            let id = run.get("databaseId")?.as_u64()?;
            let status = run.get("status")?.as_str()?;
            let created = crate::runs::seconds_of(run.get("createdAt")?.as_str()?)?;
            crate::runs::sat_queued(status, created, now, 0, limit_minutes).map(|_| id)
        })
        .collect())
}

/// What to say of each run of the `try` workflow on `branch` that has sat
/// queued with no jobs for longer than `limit_minutes`: the dispatch's own is
/// new, and an earlier one that GitHub has not given a runner says the new
/// one may wait as long.
fn queued_on(branch: &str, limit_minutes: u64) -> Result<Vec<String>> {
    let listed = crate::runs::gh(&[
        "run",
        "list",
        "--workflow",
        "try",
        "--branch",
        branch,
        "--limit",
        "20",
        "--json",
        "databaseId,status,createdAt",
    ])?;
    let now = crate::runs::now_seconds();
    let mut notes = Vec::new();
    for run in waiting_too_long(&listed, now, limit_minutes)? {
        let viewed = crate::runs::gh(&[
            "run",
            "view",
            &run.to_string(),
            "--json",
            "workflowName,status,createdAt,jobs",
        ])?;
        notes.extend(crate::runs::queued_note(&viewed, run, now, limit_minutes)?);
    }
    Ok(notes)
}

/// Dispatches the workflow on `branch` with `inputs`, and prints the run it
/// made: the runs dispatched on the branch are listed before and after, and
/// the new one is the one this made. GitHub lists a run a few seconds
/// after it is dispatched, so the list is asked again, every two seconds,
/// for at most a minute. Two runs dispatched on the branch at once are both
/// named.
#[expect(
    clippy::disallowed_methods,
    reason = "a command at a desk that waits for GitHub to list a run it dispatched, not a test"
)]
fn dispatch(branch: &str, inputs: &Inputs) -> Result<()> {
    let before = dispatched_runs(branch)?;
    let arguments = dispatch_arguments(branch, inputs);
    let shown: Vec<&str> = arguments.iter().map(String::as_str).collect();
    println!("$ gh {}", shown.join(" "));
    crate::runs::gh(&shown)?;
    for _ in 0..30 {
        let made = new_runs(&before, &dispatched_runs(branch)?);
        if !made.is_empty() {
            for run in &made {
                println!(
                    "dispatched: run {run}. Wait for it with `gh run watch {run}`, and read \
                     it with `cargo xtask try-result {run}`."
                );
            }
            if made.len() > 1 {
                println!(
                    "{} runs appeared on {branch} since the dispatch: another was dispatched \
                     at the same moment. Tell them apart by their inputs (`gh run view RUN`).",
                    made.len()
                );
            }
            let limit = inputs
                .queued_warn
                .unwrap_or(crate::runs::QUEUED_WARN_MINUTES);
            match queued_on(branch, limit) {
                Ok(notes) => {
                    for note in notes {
                        println!("{note}");
                    }
                }
                Err(error) => println!("(could not look for runs that sat queued: {error})"),
            }
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Err(format!(
        "the workflow was dispatched on {branch}, and no new run of it was listed within a \
         minute: `gh run list --workflow try --branch {branch}` shows it when GitHub does"
    ))
}

pub(crate) fn main(arguments: &[String]) -> ExitCode {
    match run(arguments) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<ExitCode> {
    let plan = plan(arguments, || {
        Ok(crate::workspace::metadata(&crate::workspace::root())?.packages)
    })?;
    for note in &plan.notes {
        println!("{note}");
    }
    if let Some(branch) = plan.inputs.dispatch.clone() {
        let (inputs, said) = for_dispatch(plan.inputs, github_commit)?;
        if let Some(said) = said {
            println!("{said}");
        }
        // A final run's jobs and their commands are in the notes above.
        if !inputs.is_final() {
            println!("$ cargo xtask {}", inputs.command()?.join(" "));
        }
        return dispatch(&branch, &inputs).map(|()| ExitCode::SUCCESS);
    }
    if !plan.command.is_empty() {
        println!("$ cargo xtask {}", plan.command.join(" "));
    }
    if plan.inputs.dry_run {
        println!("(a dry run: nothing was built or run)");
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(part) = &plan.inputs.part {
        check_runner(part, cfg!(windows))?;
    }
    if let Some(commit) = &plan.inputs.commit {
        check_checkout(commit, &head_commit()?)?;
    }
    if reads_the_corpus(&plan.command) {
        // A measurement of real code reads the pinned corpus: the same
        // bytes on a runner as at a desk. It is fetched, or found in place
        // and checked, before anything is built.
        crate::corpus::ensure_for_try()?;
    }
    crate::steps::append_job_summary(&format!(
        "### `cargo xtask {}`\n\n{}",
        plan.command.join(" "),
        plan.notes
            .iter()
            .map(|note| format!("{note}\n\n"))
            .collect::<String>()
    ));
    let (first, rest) = plan.command.split_first().ok_or("nothing to run")?;
    // Its exit code says more than pass or fail.
    if first == "mutate" {
        return Ok(crate::mutate::main(rest));
    }
    let variables = variables_for(first, plan.inputs.commit.as_deref());
    if !variables.is_empty() {
        return run_as_child(&plan.command, &variables);
    }
    crate::run(first, rest).map(|()| ExitCode::SUCCESS)
}

/// The variable that tells the tests their directories may be left on a
/// hosted runner (`cineris_testkit::scratch::KEEP`, which a test holds this
/// to).
const KEEP_DIRECTORIES: &str = "CINERIS_TESTKIT_KEEP_DIRECTORIES";

/// The variables a command needs set: [`commit_variable`]'s, and for the
/// runs of the tests as the blocking job makes them (`ci`, the tiers' run;
/// `ab`, an A/B of the tests or of the whole job) [`KEEP_DIRECTORIES`], so
/// that they measure the job as it is. A stress run and a break's are not
/// told, and remove what their tests made: they repeat the tests.
fn variables_for<'a>(verb: &str, commit: Option<&'a str>) -> Vec<(&'static str, &'a str)> {
    let mut variables: Vec<(&'static str, &'a str)> =
        commit_variable(verb, commit).into_iter().collect();
    if matches!(verb, "ci" | "ab") {
        variables.push((KEEP_DIRECTORIES, "1"));
    }
    variables
}

/// The variable a command run for `commit` needs set to it, when the
/// command reads the commit it measured from the environment: `size` writes
/// `GITHUB_SHA` in its record, which on a runner is the head of the ref the
/// run was dispatched on, not the commit checked out. The workflow cannot
/// set it (GitHub's own `GITHUB_*` variables are not overridden), so `try`
/// runs the command as a child of its own with it set.
fn commit_variable<'a>(verb: &str, commit: Option<&'a str>) -> Option<(&'static str, &'a str)> {
    match (verb, commit) {
        ("size", Some(commit)) => Some(("GITHUB_SHA", commit)),
        _ => None,
    }
}

/// `cargo xtask COMMAND…` as a child of this `xtask`, with `variables`
/// set, and its exit code.
fn run_as_child(command: &[String], variables: &[(&str, &str)]) -> Result<ExitCode> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not find this xtask to run it again: {error}"))?;
    let status = std::process::Command::new(exe)
        .args(command)
        .envs(variables.iter().copied())
        .status()
        .map_err(|error| format!("could not run `cargo xtask {}`: {error}", command.join(" ")))?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Whether a command is the crates' ignored `perf_` tests (`perf -p CRATE`,
/// alone or as a comparison of two trees), which may measure the corpus of
/// real code.
fn reads_the_corpus(command: &[String]) -> bool {
    command.first().is_some_and(|verb| verb == "perf-ab")
        || command.first().is_some_and(|verb| verb == "test")
            && command
                .windows(2)
                .any(|pair| pair[0] == "--profile" && pair[1] == "perf")
}

#[cfg(test)]
mod tests {
    use super::{
        FINAL_PARTS, Inputs, check_checkout, check_runner, dispatch_arguments, for_dispatch,
        job_words, label_of, new_runs, plan, resolve_against, url_path, waiting_too_long,
    };
    use crate::workspace::{DepKind, Dependency, Package};

    #[test]
    fn a_dispatch_passes_each_input_that_is_set_and_no_other() {
        let parse = |words: &[&str]| {
            Inputs::parse(
                &words
                    .iter()
                    .map(|word| (*word).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        let inputs = parse(&[
            "--command",
            "test -p cineris-core",
            "--os",
            "windows",
            "--against",
            "main",
            "--count",
            "5",
            "--filter",
            "test(/a/); test(=b)",
            "--testkit",
            "none",
            "--dispatch",
            "my-branch",
        ])
        .unwrap();
        assert!(inputs.dry_run, "a dispatch is checked as a dry run is");
        assert_eq!(
            dispatch_arguments("my-branch", &inputs),
            [
                "workflow",
                "run",
                "try",
                "--ref",
                "my-branch",
                "-f",
                "command=test -p cineris-core",
                "-f",
                "os=windows",
                "-f",
                "count=5",
                "-f",
                "filter=test(/a/); test(=b)",
                "-f",
                "against=main",
            ]
        );
        let other = parse(&[
            "--command",
            "startup",
            "--profile",
            "dist",
            "--capture",
            "no",
            "--testkit",
            "starts",
            "--dispatch",
            "b",
        ])
        .unwrap();
        let said = dispatch_arguments("b", &other).join(" ");
        assert!(said.contains("-f profile=dist"), "{said}");
        assert!(said.contains("-f capture=no"), "{said}");
        assert!(said.contains("-f testkit=starts"), "{said}");
        assert!(parse(&["--command", "test", "--dispatch", "a b"]).is_err());
    }

    #[test]
    fn against_is_resolved_to_its_commit_at_dispatch_and_a_hash_passes_as_it_is() {
        let asked = std::cell::RefCell::new(Vec::new());
        let github = |reference: &str| {
            asked.borrow_mut().push(reference.to_owned());
            Ok(format!("{HASH}\n"))
        };
        assert_eq!(
            resolve_against("main", github),
            Ok((
                HASH.to_owned(),
                Some("against main is 0123456789ab (resolved at dispatch)".to_owned())
            ))
        );
        assert_eq!(asked.borrow().as_slice(), ["main"]);
        // A full hash is not asked about.
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        assert_eq!(
            resolve_against(other, |_| Err("not asked".to_owned())),
            Ok((other.to_owned(), None))
        );
        // A ref GitHub does not know, and an answer that is not a hash.
        let error = resolve_against("no-such-branch", |_| Err("HTTP 422".to_owned())).unwrap_err();
        assert!(
            error.contains("`--against no-such-branch` names no commit")
                && error.contains("nothing was dispatched")
                && error.contains("HTTP 422"),
            "{error}"
        );
        let error = resolve_against("main", |_| Ok("0123abc".to_owned())).unwrap_err();
        assert!(error.contains("not a full commit hash"), "{error}");
        // The workflow and `try` take the hash as `against`, and the report
        // calls it by its first twelve digits.
        let command = try_command(&["--command", "test -p x", "--against", HASH]).unwrap();
        assert!(
            command.starts_with("ab --base ../ab-base --label 0123456789ab --top 20"),
            "{command}"
        );
        let command = try_command(&["--command", "perf -p x", "--against", HASH]).unwrap();
        assert!(command.contains("--label 0123456789ab "), "{command}");
        assert_eq!(label_of("main"), "main");
        // What is dispatched is the hash, and nothing is when the ref does
        // not resolve.
        let inputs = Inputs::parse(
            &["--command", "test", "--against", "main", "--dispatch", "b"].map(str::to_owned),
        )
        .unwrap();
        let (inputs, said) = for_dispatch(inputs, |_| Ok(HASH.to_owned())).unwrap();
        assert_eq!(
            said.as_deref(),
            Some("against main is 0123456789ab (resolved at dispatch); the report calls it main")
        );
        let sent = dispatch_arguments("b", &inputs).join(" ");
        assert!(
            sent.ends_with(&format!("-f against={HASH} -f label=main")),
            "{sent}"
        );
        // On the runner: the hash is the ref checked out, and the name the
        // user typed is what the report calls it, `-f against=main` and all.
        let command = try_command(&[
            "--command",
            "test -p x",
            "--against",
            HASH,
            "--label",
            "main",
        ])
        .unwrap();
        assert!(
            command.starts_with("ab --base ../ab-base --label main --top 20"),
            "{command}"
        );
        let command = try_command(&[
            "--command",
            "perf -p x",
            "--against",
            HASH,
            "--label",
            "main",
        ])
        .unwrap();
        assert!(command.contains("--label main "), "{command}");
        assert!(
            try_command(&["--command", "test", "--label", "main"])
                .unwrap_err()
                .contains("goes with `--against`")
        );
        // A label given is kept, and a hash given needs none.
        let inputs = Inputs::parse(
            &[
                "--command",
                "test",
                "--against",
                "main",
                "--label",
                "trunk",
                "--dispatch",
                "b",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        let (inputs, _) = for_dispatch(inputs, |_| Ok(HASH.to_owned())).unwrap();
        assert_eq!(inputs.label.as_deref(), Some("trunk"));
        let inputs = Inputs::parse(
            &["--command", "test", "--against", HASH, "--dispatch", "b"].map(str::to_owned),
        )
        .unwrap();
        let (inputs, said) = for_dispatch(inputs, |_| Err("not asked".to_owned())).unwrap();
        assert_eq!((inputs.label, said), (None, None));
        let inputs = Inputs::parse(
            &["--command", "test", "--against", "gone", "--dispatch", "b"].map(str::to_owned),
        )
        .unwrap();
        assert!(for_dispatch(inputs, |_| Err("HTTP 422".to_owned())).is_err());
        assert_eq!(url_path("step/x-1.2"), "step/x-1.2");
        assert_eq!(url_path("a#b&c"), "a%23b%26c");
    }

    #[test]
    fn a_dry_run_says_it_would_resolve_against_and_asks_nothing() {
        let arguments = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        let planned = plan(
            &arguments(&["--command", "test", "--dry-run", "--against", "main"]),
            || Ok(Vec::new()),
        )
        .unwrap();
        assert!(
            planned
                .notes
                .iter()
                .any(|note| note.contains("against main: `try --dispatch` would resolve it")),
            "{:?}",
            planned.notes
        );
        // A hash needs no resolving, and a run on the runner says nothing.
        for words in [
            &["--command", "test", "--dry-run", "--against", HASH][..],
            &["--command", "test", "--against", "main"],
        ] {
            let planned = plan(&arguments(words), || Ok(Vec::new())).unwrap();
            assert!(
                !planned
                    .notes
                    .iter()
                    .any(|note| note.contains("would resolve")),
                "{words:?}: {:?}",
                planned.notes
            );
        }
    }

    #[test]
    fn a_dispatch_takes_a_queued_limit_and_finds_the_runs_that_waited_past_it() {
        let parse = |words: &[&str]| {
            Inputs::parse(
                &words
                    .iter()
                    .map(|word| (*word).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        for flag in ["--queued-warn", "--queued-limit"] {
            let inputs = parse(&["--command", "test", "--dispatch", "b", flag, "25"]).unwrap();
            assert_eq!(inputs.queued_warn, Some(25), "{flag}");
        }
        let error = parse(&["--command", "test", "--queued-warn", "25"]).unwrap_err();
        assert!(error.contains("goes with `--dispatch`"), "{error}");
        assert!(parse(&["--command", "test", "--dispatch", "b", "--queued-warn", "0"]).is_err());
        // Not an input of the workflow: it is not dispatched.
        let inputs = parse(&[
            "--command",
            "test",
            "--dispatch",
            "b",
            "--queued-warn",
            "25",
        ])
        .unwrap();
        assert!(
            !dispatch_arguments("b", &inputs)
                .join(" ")
                .contains("queued")
        );

        let now = crate::runs::seconds_of("2026-11-16T11:00:00Z").unwrap();
        let listed = r#"[
            {"databaseId":1,"status":"queued","createdAt":"2026-11-16T10:30:00Z"},
            {"databaseId":2,"status":"queued","createdAt":"2026-11-16T10:55:00Z"},
            {"databaseId":3,"status":"in_progress","createdAt":"2026-11-16T10:00:00Z"},
            {"databaseId":4,"status":"completed","createdAt":"2026-11-16T09:00:00Z"}]"#;
        assert_eq!(waiting_too_long(listed, now, 10), Ok(vec![1, 3]));
        assert_eq!(waiting_too_long(listed, now, 45), Ok(vec![3]));
        assert!(waiting_too_long("{", now, 10).is_err());
    }

    #[test]
    fn the_run_a_dispatch_made_is_the_one_not_listed_before_it() {
        assert_eq!(new_runs(&[3, 2, 1], &[4, 3, 2, 1]), [4]);
        assert_eq!(new_runs(&[3, 2, 1], &[3, 2, 1]), Vec::<u64>::new());
        assert_eq!(new_runs(&[], &[5, 6]), [5, 6]);
    }

    #[test]
    fn capture_no_shows_a_passing_tests_output_and_goes_only_where_it_means_that() {
        assert_eq!(
            try_command(&["--command", "test -p cineris-tui", "--capture", "no"]).unwrap(),
            "ci --tiers --top 20 -p cineris-tui -- --no-capture"
        );
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cineris-tui",
                "--capture",
                "no",
                "--filter",
                "test(/frame/)"
            ])
            .unwrap(),
            "ci --tiers --top 20 -p cineris-tui -- -E test(/frame/) --no-capture"
        );
        assert!(
            try_command(&["--command", "stress -p x", "--capture", "no"])
                .unwrap()
                .contains("--no-capture")
        );
        assert_eq!(
            try_command(&["--command", "test -p x", "--capture", "yes"]).unwrap(),
            try_command(&["--command", "test -p x"]).unwrap()
        );
        for (arguments, said) in [
            (
                &[
                    "--command",
                    "test -p x",
                    "--capture",
                    "no",
                    "--against",
                    "main",
                ][..],
                "leave it out of an `against` run",
            ),
            (
                &["--command", "ci", "--capture", "no"][..],
                "goes with `test` or",
            ),
            (
                &["--command", "test", "--capture", "maybe"][..],
                "`--capture maybe`",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(said), "{arguments:?}: {error}");
        }
    }

    fn try_command(arguments: &[&str]) -> Result<String, String> {
        let arguments: Vec<String> = arguments.iter().map(|word| (*word).to_owned()).collect();
        Inputs::parse(&arguments)?
            .command()
            .map(|command| command.join(" "))
    }

    #[test]
    fn only_the_crates_perf_tests_fetch_the_corpus_of_real_code() {
        let command = |text: &str, profile: &str| {
            Inputs::parse(&[
                "--command".to_owned(),
                text.to_owned(),
                "--profile".to_owned(),
                profile.to_owned(),
            ])
            .unwrap()
            .command()
            .unwrap()
        };
        for profile in ["ci", "release", "dist"] {
            for fetches in ["perf -p cineris-index", "perf -p cineris-index warm"] {
                assert!(
                    super::reads_the_corpus(&command(fetches, profile)),
                    "{fetches} in {profile}"
                );
            }
        }
        // A `test` of a release build is `test --cargo-profile release
        // --profile ci`: it names a profile and is not a perf run.
        for (other, profile) in [
            ("test", "ci"),
            ("test", "release"),
            ("test -p cineris-index", "release"),
            ("test -p cineris-index", "ci"),
            ("stress -p cineris-index", "ci"),
            ("perf --only editor", "ci"),
            ("startup", "release"),
        ] {
            assert!(
                !super::reads_the_corpus(&command(other, profile)),
                "{other} in {profile}"
            );
        }
    }

    #[test]
    fn a_test_run_is_the_blocking_jobs_with_the_tier_report() {
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cineris-tools shell::",
                "--profile",
                "",
                "--count",
                ""
            ]),
            Ok("ci --tiers --top 20 -p cineris-tools -- shell::".to_owned())
        );
        assert_eq!(
            try_command(&["--command", "test"]),
            Ok("ci --tiers --top 20".to_owned())
        );
        assert_eq!(
            try_command(&["--command", "test -p cineris-base", "--profile", "release"]),
            Ok("test --cargo-profile release --profile ci -p cineris-base".to_owned())
        );
    }

    #[test]
    fn a_count_makes_a_test_run_a_stress_run() {
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cineris-tools shell::cmd",
                "--count",
                "100"
            ]),
            Ok("stress --count 100 --cargo-profile ci -p cineris-tools shell::cmd".to_owned())
        );
        assert_eq!(
            try_command(&[
                "--command",
                "stress -p cineris-testkit",
                "--count",
                "50",
                "--load",
                "6",
                "--cores",
                "2"
            ]),
            Ok(
                "stress --count 50 --cargo-profile ci -p cineris-testkit --load 6 --cores 2"
                    .to_owned()
            )
        );
        // A count in the command wins.
        assert_eq!(
            try_command(&[
                "--command",
                "stress -p cineris-agent --count 300",
                "--count",
                "5"
            ]),
            Ok("stress --cargo-profile ci -p cineris-agent --count 300".to_owned())
        );
    }

    #[test]
    fn filters_and_nextest_options_reach_nextest_as_they_are_meant() {
        // Several filter expressions, spaces and all, each one argument.
        let arguments: Vec<String> = [
            "--command",
            "test --package=cineris-tools -pcineris-agent --run-ignored only shell::",
            "--filter",
            "test(/shell::/) & !test(=slow) ; package(cineris-agent) ;",
        ]
        .map(str::to_owned)
        .to_vec();
        let command = Inputs::parse(&arguments).unwrap().command().unwrap();
        assert_eq!(
            command,
            [
                "ci",
                "--tiers",
                "--top",
                "20",
                "-p",
                "cineris-tools",
                "-p",
                "cineris-agent",
                "--",
                "--run-ignored",
                "only",
                "shell::",
                "-E",
                "test(/shell::/) & !test(=slow)",
                "-E",
                "package(cineris-agent)"
            ]
        );
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cineris-tools",
                "--count",
                "20",
                "--filter",
                "test(~shell)"
            ]),
            Ok("stress --count 20 --cargo-profile ci -p cineris-tools -E test(~shell)".to_owned())
        );
        assert_eq!(
            try_command(&[
                "--command",
                "test",
                "--profile",
                "release",
                "--filter",
                "test(=a)"
            ]),
            Ok("test --cargo-profile release --profile ci -E test(=a)".to_owned())
        );
        // Before a `--` of the command's own, which sends what follows to the
        // test binary.
        assert_eq!(
            try_command(&[
                "--command",
                "stress -p x -- --exact",
                "--filter",
                "test(=a)"
            ]),
            Ok("stress --count 100 --cargo-profile ci -p x -E test(=a) -- --exact".to_owned())
        );
        assert_eq!(
            try_command(&[
                "--command",
                "test -p x shell:: -- --exact",
                "--filter",
                "test(=a)"
            ]),
            Ok("ci --tiers --top 20 -p x -- shell:: -E test(=a) -- --exact".to_owned())
        );
        for (arguments, expected) in [
            (
                &["--command", "test", "--filter", "test(x); rm -rf /'"][..],
                "does not pass on",
            ),
            (
                &["--command", "test", "--filter", "test($HOME)"],
                "does not pass on",
            ),
            (
                &["--command", "perf", "--filter", "test(x)"],
                "goes with `test` or `stress`",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn against_makes_a_test_run_an_ab_run_of_the_two_refs() {
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cineris-core",
                "--against",
                "main",
                "--count",
                "6",
                "--filter",
                "test(/slow/)"
            ]),
            Ok("ab --base ../ab-base --label main --top 20 --keep-going --count 6 -p cineris-core -- -E test(/slow/)"
                .to_owned())
        );
        assert_eq!(
            try_command(&["--command", "test", "--against", "origin/main"]),
            Ok("ab --base ../ab-base --label origin/main --top 20 --keep-going".to_owned())
        );
        for (arguments, expected) in [
            (
                &["--command", "stress -p x", "--against", "main"][..],
                "goes with `test`",
            ),
            (
                &[
                    "--command",
                    "test",
                    "--against",
                    "main",
                    "--profile",
                    "release",
                ],
                "in the ci profile",
            ),
            (&["--command", "test", "--against", "a b"], "not a ref"),
            (
                &["--command", "test", "--against", "main", "--load", "2"],
                "not an A/B run",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn startup_is_measured_on_a_release_build() {
        assert_eq!(
            try_command(&["--command", "startup", "--profile", "dist", "--count", "40"]),
            Ok("startup --profile dist --runs 40".to_owned())
        );
        let error = try_command(&["--command", "startup"]).unwrap_err();
        assert!(error.contains("release build"), "{error}");
    }

    #[test]
    fn perf_and_ci_take_their_words_as_they_are() {
        assert_eq!(
            try_command(&["--command", "perf --runs 5"]),
            Ok("perf --runs 5".to_owned())
        );
        assert_eq!(
            try_command(&["--command", "ci --tiers -p cineris-llm"]),
            Ok("ci --tiers -p cineris-llm".to_owned())
        );
    }

    #[test]
    fn perf_with_a_crate_is_that_crates_ignored_perf_tests_in_release_and_nothing_else() {
        let tests = "--run-ignored only -E test(/(^|::)perf_/) --no-capture --test-threads 1";
        assert_eq!(
            try_command(&["--command", "perf -p cineris-index"]),
            Ok(format!(
                "test --cargo-profile release --profile perf -p cineris-index {tests}"
            ))
        );
        // The workflow's own default profile is `ci`: still a release build.
        assert_eq!(
            try_command(&[
                "--command",
                "perf --package=cineris-index -pcineris-session",
                "--profile",
                "ci"
            ]),
            Ok(format!(
                "test --cargo-profile release --profile perf -p cineris-index -p cineris-session {tests}"
            ))
        );
        assert_eq!(
            try_command(&["--command", "perf -p cineris-index", "--profile", "dist"]),
            Ok(format!(
                "test --cargo-profile dist --profile perf -p cineris-index {tests}"
            ))
        );
        // A name after the crate narrows the tests: nextest takes a test that
        // matches the name and the filter.
        assert_eq!(
            try_command(&["--command", "perf -p cineris-index warm"]),
            Ok(format!(
                "test --cargo-profile release --profile perf -p cineris-index {tests} warm"
            ))
        );
        // Without a crate it is step 20's measurement, as it was.
        assert_eq!(
            try_command(&["--command", "perf --only editor"]),
            Ok("perf --only editor".to_owned())
        );
        for (arguments, expected) in [
            (
                &["--command", "perf -p cineris-index", "--count", "3"][..],
                "runs each perf test once",
            ),
            (
                &["--command", "perf -p cineris-index", "--filter", "test(x)"],
                "goes with `test` or `stress`",
            ),
            (
                &["--command", "perf -p cineris-index", "--load", "2"],
                "go with a stress run",
            ),
            (
                &["--command", "perf --only editor", "--against", "main"],
                "name them with -p",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn the_nightlys_jobs_run_as_the_nightly_runs_them() {
        for job in ["partial-landlock", "advisories", "unused"] {
            assert_eq!(try_command(&["--command", job]), Ok(job.to_owned()));
            for (arguments, expected) in [
                (&["--command", job, "--count", "3"][..], "as it does there"),
                (&["--command", &format!("{job} --x")], "as it does there"),
                (
                    &["--command", job, "--profile", "release"],
                    "as it does there",
                ),
                (&["--command", job, "--against", "main"], "as it does there"),
                (
                    &["--command", job, "--filter", "test(x)"],
                    "goes with `test`",
                ),
            ] {
                let error = try_command(arguments).unwrap_err();
                assert!(error.contains(expected), "{arguments:?}: {error}");
            }
        }
    }

    #[test]
    fn what_a_try_run_does_not_run_is_refused() {
        for (arguments, expected) in [
            (&["--command", "fmt"][..], "not a command a try run runs"),
            (&["--command", ""], "say what to run"),
            (&["--command", "test; curl evil"], "has a character"),
            (&["--command", "test $(id)"], "has a character"),
            (&["--command", "test -p 'x'"], "has a character"),
            (
                &["--command", "test", "--profile", "dev"],
                "ci, release or dist",
            ),
            (&["--command", "test", "--count", "0"], "a number from 1"),
            (&["--command", "test", "--count", "lots"], "a number from 1"),
            (
                &["--command", "test", "--load", "4"],
                "go with a stress run",
            ),
            (
                &["--command", "startup", "--cores", "2", "--profile", "dist"],
                "go with a stress run",
            ),
            (&["--command"], "needs a value"),
            (&["--shell", "x"], "unknown flag"),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    // The commit, `mutate`, `--dependents`, and what cannot be passed on.

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_commit_is_a_full_lower_case_hash_or_nothing() {
        let parse = |commit: &str| {
            Inputs::parse(&[
                "--command".to_owned(),
                "test".to_owned(),
                "--commit".to_owned(),
                commit.to_owned(),
            ])
        };
        assert_eq!(parse(HASH).unwrap().commit.as_deref(), Some(HASH));
        assert_eq!(parse("").unwrap().commit, None);
        assert_eq!(parse("  ").unwrap().commit, None);
        for bad in [
            "main",
            "0123456",
            // 39 and 41 digits.
            &HASH[..39],
            &format!("{HASH}0"),
            // Upper case, a letter past f, and the flag-looking.
            &HASH.to_uppercase(),
            &format!("{}g", &HASH[..39]),
            "--help",
            &format!("{}; id", &HASH[..30]),
        ] {
            let error = parse(bad).unwrap_err();
            assert!(error.contains("40 hexadecimal digits"), "{bad}: {error}");
        }
    }

    #[test]
    fn the_checkout_must_be_the_commit_asked_for() {
        assert_eq!(check_checkout(HASH, &format!("{HASH}\n")), Ok(()));
        let error = check_checkout(HASH, "fedcba9876543210fedcba9876543210fedcba98\n").unwrap_err();
        assert!(error.contains("not at the commit asked for"), "{error}");
        assert!(check_checkout(HASH, "").is_err());
    }

    #[test]
    fn mutate_runs_the_breaks_to_the_end_with_the_options_it_may_have() {
        assert_eq!(
            try_command(&["--command", "mutate tooling-7"]),
            Ok("mutate --minutes 0 tooling-7".to_owned())
        );
        assert_eq!(
            try_command(&[
                "--command",
                "mutate step-12 step-14/deny --jobs 2 --again --no-compile-check"
            ]),
            Ok(
                "mutate --minutes 0 step-12 step-14/deny --jobs 2 --again --no-compile-check"
                    .to_owned()
            )
        );
        assert_eq!(
            try_command(&["--command", "mutate -j 3 --file mutations/x.breaks"]),
            Ok("mutate --minutes 0 -j 3 --file mutations/x.breaks".to_owned())
        );
        // A budget stops starting breaks within the job; `--minutes 0` beside
        // it is what `mutate` takes.
        assert_eq!(
            try_command(&["--command", "mutate step-12 --budget 30m"]),
            Ok("mutate --minutes 0 step-12 --budget 30m".to_owned())
        );
        assert_eq!(
            try_command(&["--command", "mutate step-12 --budget 50m"]),
            Ok("mutate --minutes 0 step-12 --budget 50m".to_owned()),
            "the most a try run's budget may be"
        );
        let error = try_command(&["--command", "mutate step-12 --budget 51m"]).unwrap_err();
        assert!(error.contains("50 minutes at most"), "{error}");
        assert!(try_command(&["--command", "mutate step-12 --budget soon"]).is_err());
        for (arguments, expected) in [
            (
                &["--command", "mutate step-12", "--against", "main"][..],
                "`--against` does not go with `mutate`",
            ),
            (
                &["--command", "mutate step-12", "--count", "3"],
                "`--count` does not go with `mutate`",
            ),
            (
                &["--command", "mutate step-12", "--load", "3"],
                "`--load` does not go with `mutate`",
            ),
            (
                &["--command", "mutate step-12", "--cores", "2"],
                "`--cores` does not go with `mutate`",
            ),
            (
                &["--command", "mutate step-12", "--filter", "test(x)"],
                "goes with `test` or `stress`",
            ),
            (
                &["--command", "mutate step-12", "--profile", "release"],
                "the ci profile",
            ),
            (&["--command", "mutate"], "name the breaks to run"),
            (&["--command", "mutate --jobs 2"], "name the breaks to run"),
            (&["--command", "mutate --check step-12"], "runs no break"),
            (&["--command", "mutate --report step-12"], "runs no break"),
            (&["--command", "mutate --tests step-12"], "runs no break"),
            (
                &["--command", "mutate --minutes 5 step-12"],
                "gives `--minutes 0` itself",
            ),
            (
                &["--command", "mutate --fast step-12"],
                "not an option a try run gives",
            ),
            (&["--command", "mutate step-12 --jobs"], "needs a value"),
            // What `mutate` itself would say of the number.
            (
                &["--command", "mutate step-12 --jobs 99"],
                "a number from 1 to 16",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    /// base <- tools <- agent <- core; stats dev-uses base; other alone.
    fn workspace() -> Vec<Package> {
        let package = |name: &str, edges: &[(&str, DepKind)]| Package {
            name: name.to_owned(),
            manifest: format!("crates/{name}/Cargo.toml"),
            publish_disabled: true,
            dependencies: edges
                .iter()
                .map(|(to, kind)| Dependency {
                    name: (*to).to_owned(),
                    kind: *kind,
                    in_workspace: true,
                    default_features: true,
                    features: Vec::new(),
                    optional: false,
                })
                .collect(),
            targets: Vec::new(),
            features: Vec::new(),
        };
        vec![
            package("core", &[("agent", DepKind::Normal)]),
            package("agent", &[("tools", DepKind::Normal)]),
            package("tools", &[("base", DepKind::Normal)]),
            package("base", &[]),
            package("stats", &[("base", DepKind::Dev)]),
            package("other", &[]),
        ]
    }

    fn planned(arguments: &[&str]) -> Result<(String, Vec<String>), String> {
        let arguments: Vec<String> = arguments.iter().map(|word| (*word).to_owned()).collect();
        plan(&arguments, || Ok(workspace())).map(|plan| (plan.command.join(" "), plan.notes))
    }

    #[test]
    fn dependents_become_the_crates_the_job_runs_and_are_named() {
        let (command, notes) =
            planned(&["--command", "test --dependents -p tools shell::"]).unwrap();
        assert_eq!(
            command,
            "ci --tiers --top 20 -p agent -p core -p tools -- shell::"
        );
        assert_eq!(
            notes,
            ["--dependents: tools and 2 crate(s) whose tests can see it: agent, core"]
        );
        // A change to the base reaches the crate that dev-uses it.
        let (command, _) = planned(&["--command", "test --dependents -pbase"]).unwrap();
        assert_eq!(
            command,
            "ci --tiers --top 20 -p agent -p base -p core -p stats -p tools"
        );
        // With a count, in the release profile, for an A/B, a stress run and
        // the perf tests: each takes the crates.
        let (command, _) = planned(&[
            "--command",
            "test --dependents -p tools",
            "--against",
            "main",
            "--count",
            "3",
        ])
        .unwrap();
        assert_eq!(
            command,
            "ab --base ../ab-base --label main --top 20 --keep-going --count 3 -p agent -p core -p tools"
        );
        let (command, _) =
            planned(&["--command", "stress --dependents -p tools --count 9"]).unwrap();
        assert_eq!(
            command,
            "stress --cargo-profile ci --count 9 -p agent -p core -p tools"
        );
        let (command, _) = planned(&[
            "--command",
            "test --dependents -p tools",
            "--profile",
            "release",
        ])
        .unwrap();
        assert_eq!(
            command,
            "test --cargo-profile release --profile ci -p agent -p core -p tools"
        );
        let (command, _) = planned(&["--command", "perf --dependents -p tools warm"]).unwrap();
        assert!(
            command.contains("-p agent -p core -p tools --run-ignored only"),
            "{command}"
        );
        // A crate with nothing built on it adds none, and says so.
        let (_, notes) = planned(&["--command", "test --dependents -p other"]).unwrap();
        assert_eq!(
            notes,
            ["--dependents: other and 0 crate(s) whose tests can see it: none"]
        );
    }

    #[test]
    fn dependents_that_cannot_be_resolved_are_refused_before_anything_is_built() {
        for (arguments, expected) in [
            (
                &["--command", "test --dependents"][..],
                "needs the crates whose dependents to test",
            ),
            (
                &["--command", "test --dependents -p nope"],
                "`nope` is not a package",
            ),
            (
                &["--command", "ci --dependents -p tools"],
                "goes with `test`, `stress` or `perf -p`, not `ci`",
            ),
            (
                &[
                    "--command",
                    "startup --dependents -p tools",
                    "--profile",
                    "dist",
                ],
                "not `startup`",
            ),
            (
                &["--command", "advisories --dependents -p tools"],
                "not `advisories`",
            ),
        ] {
            let error = planned(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
        // The flag after the command's own `--` is for the test binaries.
        let (command, notes) = planned(&["--command", "test -p tools -- --dependents"]).unwrap();
        assert_eq!(command, "ci --tiers --top 20 -p tools -- -- --dependents");
        assert!(notes.is_empty());
        // The workspace is read only when the flag is there.
        let arguments = ["--command".to_owned(), "test -p tools".to_owned()];
        let command = plan(&arguments, || panic!("no --dependents: no workspace read"));
        assert!(command.is_ok());
    }

    #[test]
    fn a_word_that_nextest_would_not_take_is_refused_with_its_reason() {
        for (arguments, word, why) in [
            (
                &["--command", "test -p x --tiers"][..],
                "--tiers",
                "tier report",
            ),
            (&["--command", "test -p x --top 5"], "--top", "lists 20"),
            (
                &["--command", "test -p x --profile fast"],
                "--profile",
                "`try` chooses nextest's profile",
            ),
            (
                &["--command", "test -p x --cargo-profile dist"],
                "--cargo-profile",
                "`profile` input",
            ),
            (
                &["--command", "test -p x --count 3"],
                "--count",
                "`count` input",
            ),
            (
                &["--command", "test -p x --load=3"],
                "--load=3",
                "`load` input",
            ),
            (
                &["--command", "test -p x --keep-going"],
                "--keep-going",
                "cargo xtask ab",
            ),
            (
                &["--command", "test -p x --filter test(a)"],
                "--filter",
                "`filter` input",
            ),
            (
                &["--command", "stress -p x --cargo-profile dist"],
                "--cargo-profile",
                "`profile` input",
            ),
            (
                &["--command", "stress -p x --tiers"],
                "--tiers",
                "tier report",
            ),
            (
                &["--command", "perf -p x --profile fast"],
                "--profile",
                "chooses nextest's profile",
            ),
            (
                &["--command", "test -p x --tiers", "--against", "main"],
                "--tiers",
                "tier report",
            ),
            (
                &["--command", "test -p x --commit abc"],
                "--commit",
                "`commit` input",
            ),
            // With `against` a count is rounds, not a stress run's: the word
            // is not the stress run's own.
            (
                &[
                    "--command",
                    "test -p x --count 3",
                    "--against",
                    "main",
                    "--count",
                    "4",
                ],
                "--count",
                "`count` input",
            ),
            // What `try`'s runs give nextest already, which it refuses
            // twice: found before the two trees of an A/B are built.
            (
                &["--command", "test -p x --locked", "--against", "main"],
                "--locked",
                "`--locked` already",
            ),
            (
                &["--command", "test --workspace"],
                "--workspace",
                "given twice",
            ),
            (
                &["--command", "test -p x --no-tests=pass"],
                "--no-tests=pass",
                "given twice",
            ),
            // A short option with its value joined to it.
            (
                &["--command", "test -p x -Pfast"],
                "-Pfast",
                "nextest's profile",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(
                error.contains(&format!("`{word}` cannot be passed on")) && error.contains(why),
                "{arguments:?}: {error}"
            );
            assert!(error.contains("Nothing was built"), "{error}");
        }
        // What is the stress run's own is allowed in one, and a `--` ends
        // the words that are nextest's.
        assert!(try_command(&["--command", "stress -p x --count 5 --load 2 --cores 1"]).is_ok());
        assert!(
            try_command(&["--command", "test -p x --count 5", "--count", "9"]).is_ok(),
            "a test with a count is a stress run"
        );
        assert!(try_command(&["--command", "test -p x -- --profile"]).is_ok());
        // A crate that is named like an option is a crate.
        assert!(try_command(&["--command", "test -p top"]).is_ok());
        // The real nextest options pass.
        assert!(
            try_command(&[
                "--command",
                "test -p x --run-ignored only --no-capture -E test(a)"
            ])
            .is_ok()
        );
    }

    #[test]
    fn what_the_commands_own_parser_would_refuse_is_refused_here() {
        for (arguments, command, expected) in [
            (
                &["--command", "ci --nope"][..],
                "ci",
                "`ci --nope` is not a form of `ci`",
            ),
            (
                &["--command", "ci lint test"],
                "ci",
                "`ci lint test` is not a form of `ci`",
            ),
            (
                &["--command", "ci --tiers --top many"],
                "ci",
                "`--top many` is not a number",
            ),
            (
                &["--command", "stress -p x --stress-count 3"],
                "stress",
                "is set by this command",
            ),
            (
                &["--command", "test", "--against", "main", "--count", "100"],
                "ab",
                "a number of rounds from 2 to 50",
            ),
            (
                &["--command", "stress --count 3"],
                "stress",
                "say which crates to stress",
            ),
            (
                &["--command", "perf --nope"],
                "perf",
                "unknown argument `--nope`",
            ),
            (
                &["--command", "startup foo", "--profile", "dist"],
                "startup",
                "names no binary",
            ),
            (
                &[
                    "--command",
                    "startup --profile release",
                    "--profile",
                    "dist",
                ],
                "startup",
                "a second `--profile`",
            ),
            (
                &["--command", "startup --runs 0", "--profile", "dist"],
                "startup",
                "not a number above zero",
            ),
            (
                &["--command", "startup --budget moon", "--profile", "dist"],
                "startup",
                "desktop, linux or windows",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(
                error.contains(&format!("`cargo xtask {command}` would refuse this"))
                    && error.contains(expected),
                "{arguments:?}: {error}"
            );
        }
        for arguments in [
            &["--command", "ci lint"][..],
            &["--command", "ci test"],
            &["--command", "ci"],
            &["--command", "ci --tiers --top 3 -p x -- -E test(a)"],
            &[
                "--command",
                "startup --check --budget windows",
                "--profile",
                "dist",
            ],
            &["--command", "stress -p x --first-seed 7 -- --exact"],
        ] {
            assert!(try_command(arguments).is_ok(), "{arguments:?}");
        }
    }

    #[test]
    fn a_dry_run_and_the_workflows_other_inputs_are_read() {
        let parse = |words: &[&str]| {
            let arguments: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
            Inputs::parse(&arguments)
        };
        let inputs = parse(&[
            "--command",
            "test",
            "--dry-run",
            "--os",
            "linux-22.04",
            "--testkit",
            "profile",
        ])
        .unwrap();
        assert!(inputs.dry_run);
        assert_eq!(inputs.os.as_deref(), Some("linux-22.04"));
        assert_eq!(inputs.testkit.as_deref(), Some("profile"));
        assert!(!parse(&["--command", "test"]).unwrap().dry_run);
        assert_eq!(parse(&["--command", "test", "--os", ""]).unwrap().os, None);
        for (words, expected) in [
            (
                &["--command", "test", "--os", "macos"][..],
                "both, linux, windows, linux-22.04",
            ),
            (
                &["--command", "test", "--testkit", "all"],
                "none, profile or starts",
            ),
        ] {
            let error = parse(words).unwrap_err();
            assert!(error.contains(expected), "{words:?}: {error}");
        }
        // Notes of a dry run only.
        let arguments: Vec<String> = [
            "--command",
            "test",
            "--dry-run",
            "--against",
            "main",
            "--testkit",
            "profile",
        ]
        .map(str::to_owned)
        .to_vec();
        let planned = plan(&arguments, || Ok(Vec::new())).unwrap();
        assert!(
            planned
                .notes
                .iter()
                .any(|note| note.contains("testkit=profile is not set"))
        );
        let mut starts = arguments.clone();
        *starts.last_mut().unwrap() = "starts".to_owned();
        let planned = plan(&starts, || Ok(Vec::new())).unwrap();
        assert!(
            planned
                .notes
                .iter()
                .any(|note| note.contains("testkit=starts is not set"))
        );
        let arguments: Vec<String> = [
            "--command",
            "partial-landlock",
            "--dry-run",
            "--os",
            "linux",
        ]
        .map(str::to_owned)
        .to_vec();
        let planned = plan(&arguments, || Ok(Vec::new())).unwrap();
        assert!(
            planned
                .notes
                .iter()
                .any(|note| note.contains("os=linux-22.04"))
        );
        assert_eq!(planned.command, ["partial-landlock"]);
    }

    #[test]
    fn against_makes_a_crates_perf_run_a_comparison_of_the_two_trees() {
        assert_eq!(
            try_command(&["--command", "perf -p cineris-index", "--against", "main"]),
            Ok(
                "perf-ab --base ../ab-base --label main --cargo-profile release -p cineris-index"
                    .to_owned()
            )
        );
        // Several crates, a test's name, dist, and a count of rounds (three
        // at least: with two, ranges of one tree often do not overlap).
        assert_eq!(
            try_command(&[
                "--command",
                "perf -p cineris-index -pcineris-tools warm",
                "--against",
                "origin/main",
                "--profile",
                "dist",
                "--count",
                "1"
            ]),
            Ok(
                "perf-ab --base ../ab-base --label origin/main --cargo-profile dist --count 3 \
                -p cineris-index -p cineris-tools warm"
                    .to_owned()
            )
        );
        assert_eq!(
            try_command(&[
                "--command",
                "perf -p cineris-index",
                "--against",
                "main",
                "--profile",
                "release",
                "--count",
                "5"
            ]),
            Ok(
                "perf-ab --base ../ab-base --label main --cargo-profile release --count 5 \
                -p cineris-index"
                    .to_owned()
            )
        );
        for (arguments, expected) in [
            (
                &[
                    "--command",
                    "perf -p x --run-ignored all",
                    "--against",
                    "main",
                ][..],
                "no other option",
            ),
            (
                &["--command", "perf -p x", "--against", "main", "--load", "2"],
                "go with a stress run",
            ),
            (
                &[
                    "--command",
                    "perf -p x",
                    "--against",
                    "main",
                    "--count",
                    "21",
                ],
                "from 3 to 20",
            ),
            (
                &["--command", "perf -p x --against y", "--against", "main"],
                "no other option",
            ),
            (&["--command", "perf -p x", "--against", "a b"], "not a ref"),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    #[test]
    fn only_a_perf_comparison_and_a_perf_run_fetch_the_corpus() {
        let command = |words: &[&str]| {
            let arguments: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
            Inputs::parse(&arguments).unwrap().command().unwrap()
        };
        assert!(super::reads_the_corpus(&command(&[
            "--command",
            "perf -p cineris-index",
            "--against",
            "main"
        ])));
        assert!(!super::reads_the_corpus(&command(&[
            "--command",
            "test -p cineris-index",
            "--against",
            "main"
        ])));
    }

    /// The target `size` builds for on this runner when none is named.
    fn size_target() -> &'static str {
        if cfg!(target_os = "linux") {
            " --target x86_64-unknown-linux-musl"
        } else {
            ""
        }
    }

    #[test]
    fn a_size_run_takes_its_options_and_the_profile_chooses_the_build() {
        let musl = size_target();
        assert_eq!(
            try_command(&["--command", "size --check --against-main"]),
            Ok(format!("size --check --against-main{musl}"))
        );
        assert_eq!(
            try_command(&["--command", "size", "--profile", "release"]),
            Ok(format!("size --profile release{musl}"))
        );
        // The breakdown reads an ELF binary's symbols: a Linux runner's.
        let breakdown = try_command(&[
            "--command",
            "size --breakdown --top 10",
            "--profile",
            "dist",
        ]);
        if cfg!(target_os = "linux") {
            assert_eq!(
                breakdown,
                Ok(format!("size --breakdown --top 10 --profile dist{musl}"))
            );
        } else {
            assert!(
                breakdown
                    .as_ref()
                    .is_err_and(|error| error.contains("--breakdown")),
                "{breakdown:?}"
            );
        }
        // A target that is named is the one built.
        let own = if cfg!(target_os = "linux") {
            "x86_64-unknown-linux-musl"
        } else {
            "x86_64-pc-windows-msvc"
        };
        assert_eq!(
            try_command(&["--command", &format!("size --target {own}")]),
            Ok(format!("size --target {own}"))
        );
    }

    #[test]
    fn a_size_run_refuses_what_makes_no_sense_before_anything_is_built() {
        let other_os = if cfg!(target_os = "linux") {
            "x86_64-pc-windows-msvc"
        } else {
            "x86_64-unknown-linux-musl"
        };
        for (arguments, expected) in [
            (
                &["--command", "size --check", "--profile", "release"][..],
                "the budgets are of the shipped binary",
            ),
            (
                &["--command", "size --against-main", "--profile", "release"],
                "records the `dist` build",
            ),
            (
                &["--command", "size --profile dist"],
                "the `profile` input chooses",
            ),
            (&["--command", "size --at abc123"], "use the `commit` input"),
            (&["--command", "size --nope"], "usage: cargo xtask size"),
            (
                &["--command", "size", "--against", "main"],
                "`size`'s own `--against-main`",
            ),
            (&["--command", "size", "--count", "3"], "builds once"),
            (
                &["--command", "size", "--filter", "test(x)"],
                "goes with `test` or `stress`",
            ),
            (&["--command", "size --top 0"], "is not a number above zero"),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
        let error = try_command(&["--command", &format!("size --target {other_os}")]).unwrap_err();
        assert!(
            error.contains("is not a target this runner builds for"),
            "{error}"
        );
        // The breakdown reads symbols with `nm`: a Windows binary has none.
        let windows = if cfg!(target_os = "linux") {
            "size --breakdown --target x86_64-pc-windows-msvc"
        } else {
            "size --breakdown"
        };
        let error = try_command(&["--command", windows]).unwrap_err();
        assert!(
            error.contains("for the Linux targets") || error.contains("not a target this runner"),
            "{error}"
        );
    }

    #[test]
    fn a_commit_is_said_in_the_plan() {
        let arguments: Vec<String> = ["--command", "test -p cineris-base", "--commit", HASH]
            .map(str::to_owned)
            .to_vec();
        let planned = plan(&arguments, || Ok(Vec::new())).unwrap();
        assert_eq!(
            planned.command,
            ["ci", "--tiers", "--top", "20", "-p", "cineris-base"]
        );
        assert!(planned.notes[0].contains(HASH), "{:?}", planned.notes);
    }

    #[test]
    fn a_final_dry_run_shows_each_of_its_three_jobs_and_what_it_runs() {
        let (command, notes) = planned(&["--final", "--dry-run"]).unwrap();
        assert_eq!(command, "", "the jobs' commands are the notes");
        let jobs: Vec<&String> = notes
            .iter()
            .filter(|note| note.starts_with("try-"))
            .collect();
        assert_eq!(jobs.len(), 3, "{notes:?}");
        assert!(
            jobs[0].starts_with("try-windows (the Windows A/B")
                && jobs[0].ends_with(
                    "part ab): $ cargo xtask ab --base ../ab-base --label main --top 20 \
                     --keep-going"
                ),
            "{}",
            jobs[0]
        );
        assert!(
            jobs[1].starts_with("try-linux-22.04 (the tests on Linux 22.04")
                && jobs[1].ends_with("part tests): $ cargo xtask ci --tiers --top 20"),
            "{}",
            jobs[1]
        );
        assert!(
            jobs[2].starts_with("try-linux (the binary's size")
                && jobs[2].ends_with(&format!(
                    "part size): $ cargo xtask size --check --against-main{}",
                    size_target()
                )),
            "{}",
            jobs[2]
        );
        assert!(
            notes
                .iter()
                .any(|note| note.contains("would resolve it to its commit")),
            "main is resolved at a dispatch: {notes:?}"
        );
        // `--command final` is the same run; a count and an against reach
        // the A/B alone.
        let (_, notes) = planned(&[
            "--command",
            "final",
            "--dry-run",
            "--count",
            "3",
            "--against",
            "my-base",
        ])
        .unwrap();
        assert!(
            notes[0].ends_with("--label my-base --top 20 --keep-going --count 3"),
            "{notes:?}"
        );
        assert!(notes[1].ends_with("ci --tiers --top 20"), "{notes:?}");
        assert!(!notes[2].contains("--count"), "{notes:?}");
    }

    #[test]
    fn a_final_runs_job_runs_the_part_it_is_given_and_nothing_else() {
        let part = |name: &str| {
            planned(&[
                "--command",
                "final",
                "--part",
                name,
                "--against",
                HASH,
                "--label",
                "main",
                "--count",
                "4",
                "--commit",
                HASH,
            ])
            .map(|(command, _)| command)
        };
        assert_eq!(
            part("ab").unwrap(),
            "ab --base ../ab-base --label main --top 20 --keep-going --count 4"
        );
        assert_eq!(part("tests").unwrap(), "ci --tiers --top 20");
        assert_eq!(
            part("size").unwrap(),
            format!("size --check --against-main{}", size_target())
        );
        let error = planned(&["--final"]).unwrap_err();
        assert!(
            error.contains("each given its part by the workflow"),
            "{error}"
        );
        let error = planned(&["--final", "--part", "all"]).unwrap_err();
        assert!(error.contains("ab, tests, size"), "{error}");
        // A job given no `against` (a dispatch by hand) is told so before
        // anything is built, and does not look for a base that is not there.
        let error = planned(&["--command", "final", "--part", "ab"]).unwrap_err();
        assert!(error.contains("needs `-f against=main`"), "{error}");
        assert_eq!(
            planned(&["--command", "final", "--part", "tests"])
                .unwrap()
                .0,
            "ci --tiers --top 20",
            "the other parts need none"
        );
    }

    #[test]
    fn a_final_run_refuses_the_inputs_that_would_change_a_part() {
        for (arguments, said) in [
            (&["--final", "--os", "windows"][..], "`--os"),
            (&["--final", "--filter", "test(=a)"], "`--filter`"),
            (&["--final", "--load", "2"], "`--load`"),
            (&["--final", "--cores", "2"], "`--cores`"),
            (&["--final", "--capture", "no"], "`--capture no`"),
            (&["--final", "--testkit", "profile"], "`--testkit`"),
            (&["--final", "--profile", "dist"], "`--profile"),
            (&["--command", "final -p x"], "takes no words"),
            (
                &["--command", "test", "--part", "ab"],
                "goes with `--final`",
            ),
        ] {
            let error = Inputs::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(said), "{arguments:?}: {error}");
        }
        // `--testkit none` is the default, and no change.
        assert!(Inputs::parse(&strings(&["--final", "--testkit", "none"])).is_ok());
    }

    #[test]
    fn a_final_dispatch_sends_final_and_main_resolved_and_no_os() {
        let inputs =
            Inputs::parse(&strings(&["--final", "--count", "3", "--dispatch", "b"])).unwrap();
        let (inputs, said) = for_dispatch(inputs, |_| Ok(HASH.to_owned())).unwrap();
        assert!(
            said.unwrap()
                .ends_with("(resolved at dispatch); the report calls it main")
        );
        assert_eq!(
            dispatch_arguments("b", &inputs).join(" "),
            format!(
                "workflow run try --ref b -f command=final -f count=3 -f against={HASH} \
                 -f label=main"
            )
        );
    }

    #[test]
    fn a_size_of_a_commit_records_that_commit_and_no_other() {
        assert_eq!(
            super::commit_variable("size", Some(HASH)),
            Some(("GITHUB_SHA", HASH))
        );
        assert_eq!(super::commit_variable("size", None), None, "the ref's head");
        assert_eq!(super::commit_variable("ci", Some(HASH)), None);
    }

    /// The runs of the tests as the job makes them leave their directories
    /// on a hosted runner, as the job does; a stress run and a break's,
    /// which repeat the tests, are not told to.
    #[test]
    fn only_the_jobs_runs_of_the_tests_are_told_to_leave_their_directories() {
        let keep = (super::KEEP_DIRECTORIES, "1");
        assert_eq!(super::variables_for("ci", None), [keep]);
        assert_eq!(super::variables_for("ab", None), [keep]);
        for repeats in ["stress", "mutate", "perf", "startup"] {
            assert_eq!(super::variables_for(repeats, None), [], "{repeats}");
        }
        assert_eq!(
            super::variables_for("size", Some(HASH)),
            [("GITHUB_SHA", HASH)]
        );
        // The test kit's own name for it.
        assert_eq!(super::KEEP_DIRECTORIES, cineris_testkit::scratch::KEEP);
    }

    #[test]
    fn a_final_part_runs_only_on_its_own_system() {
        assert!(check_runner("ab", true).is_ok());
        assert!(check_runner("tests", false).is_ok());
        assert!(check_runner("size", false).is_ok());
        let error = check_runner("ab", false).unwrap_err();
        assert!(error.contains("windows-latest"), "{error}");
        assert!(check_runner("size", true).is_err());
        assert!(check_runner("other", true).is_err());
    }

    /// The JSON of the final run's matrix in `try.yml`: what follows
    /// `inputs.command == 'final' && '`, up to the next quote.
    fn final_matrix(workflow: &str) -> serde_json::Value {
        let start = workflow
            .find("inputs.command == 'final' && '")
            .expect("try.yml chooses a matrix for a final run")
            + "inputs.command == 'final' && '".len();
        let end = start + workflow[start..].find('\'').expect("the JSON ends");
        serde_json::from_str(&workflow[start..end]).expect("the matrix is JSON")
    }

    #[test]
    fn the_workflows_final_matrix_is_the_final_runs_parts() {
        let workflow = include_str!("../../.github/workflows/try.yml");
        let matrix = final_matrix(workflow);
        let entries = matrix.as_array().expect("a list of jobs");
        assert_eq!(entries.len(), FINAL_PARTS.len());
        for (entry, part) in entries.iter().zip(FINAL_PARTS) {
            assert_eq!(entry["name"], part.job, "{entry}");
            assert_eq!(entry["os"], part.runs_on, "{entry}");
            assert_eq!(entry["part"], part.part, "{entry}");
        }
        // Each job is told its part, and only a final run's job.
        assert!(
            workflow.contains("TRY_PART: ${{ inputs.command == 'final' && matrix.part || '' }}"),
            "the part reaches the job"
        );
        // Only the A/B's job checks out a base; only the size job has the
        // token; each run step stops under its job's limit, so that the
        // upload after it runs.
        assert_eq!(
            workflow
                .matches(
                    "if: inputs.against != '' && (inputs.command != 'final' || matrix.part == 'ab')"
                )
                .count(),
            2,
            "the base's checkout and its move"
        );
        assert_eq!(
            workflow
                .matches(
                    "GH_TOKEN: ${{ (startsWith(inputs.command, 'size') || inputs.command == 'final' && matrix.part == 'size') && github.token || '' }}"
                )
                .count(),
            2
        );
        assert!(workflow.contains(
            "timeout-minutes: ${{ contains(format(' {0} ', inputs.command), ' --job ') && 240 || (startsWith(inputs.command, 'mutate') && 90 || ((inputs.command == 'final' || inputs.against != '') && 60 || 45)) }}"
        ));
        assert_eq!(
            workflow
                .matches(
                    "timeout-minutes: ${{ contains(format(' {0} ', inputs.command), ' --job ') && 230 || (startsWith(inputs.command, 'mutate') && 80 || ((inputs.command == 'final' || inputs.against != '') && 50 || 35)) }}"
                )
                .count(),
            2,
            "each run step, ten minutes under its job"
        );
        assert_eq!(
            super::MUTATE_STEP_MINUTES,
            80,
            "the run step's limit for mutate"
        );
        assert_eq!(
            workflow.matches("added+=(--part \"$TRY_PART\")").count()
                + workflow
                    .matches("$added += @('--part', $env:TRY_PART)")
                    .count(),
            2,
            "both runners pass it on"
        );
    }

    #[test]
    fn a_job_ab_is_test_job_with_against_and_its_options_go_to_ab_job() {
        assert_eq!(
            try_command(&[
                "--command",
                "test --job --setup-head .github/x.sh --keep-build -p cineris-base",
                "--against",
                "main",
                "--count",
                "4",
            ])
            .unwrap(),
            "ab --job --base ../ab-base --label main --minutes 200 --count 4 --setup-head \
             .github/x.sh --keep-build -p cineris-base"
        );
        // An odd count gives one side the first place more often.
        let error = try_command(&[
            "--command",
            "test --job",
            "--against",
            "main",
            "--count",
            "3",
        ])
        .unwrap_err();
        assert!(error.contains("an even number of rounds"), "{error}");
        assert_eq!(
            try_command(&[
                "--command",
                "test --job --setup-base a.sh --setup-head b.sh",
                "--against",
                HASH,
                "--label",
                "main",
            ])
            .unwrap(),
            "ab --job --base ../ab-base --label main --minutes 200 --setup-base a.sh \
             --setup-head b.sh"
        );
        for (arguments, said) in [
            (
                &["--command", "test --job"][..],
                "it goes with the `against` input",
            ),
            (
                &["--command", "test --setup-base a.sh", "--against", "main"],
                "it goes with `--job`",
            ),
            (
                &[
                    "--command",
                    "test --job --setup-base a --setup-base b",
                    "--against",
                    "main",
                ],
                "given twice",
            ),
            (
                &[
                    "--command",
                    "test --job",
                    "--against",
                    "main",
                    "--capture",
                    "no",
                ],
                "`--capture no`",
            ),
        ] {
            let error = try_command(arguments).unwrap_err();
            assert!(error.contains(said), "{arguments:?}: {error}");
        }
        // After a `--` of the command's own, a word is the test binaries'.
        assert_eq!(
            job_words(&strings(&["-p", "x", "--", "--job"])).unwrap(),
            (None, strings(&["-p", "x", "--", "--job"]))
        );
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }
}
