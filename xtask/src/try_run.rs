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
                       [--against REF] [--commit SHA] [--dry-run]
                       [--os OS] [--testkit none|profile]

COMMAND is one of:
  test [-p CRATE]... [FILTER]...   the tests, as the blocking job builds and runs
                                   them, with the tier report; with --count,
                                   `stress` with that many repeats
  stress -p CRATE... [ARGS]...     the tests many times over (--count repeats)
  startup                          start-up of `cortex` built in the profile
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
                                   --again, --no-compile-check and --file PATH
                                   pass; --count, --load, --cores, --filter
                                   and --against are refused with it
  partial-landlock                 the nightly's job: cortex-sandbox's tests on
                                   a kernel with the partial Landlock (on
                                   os=linux-22.04, the one runner that has it)
  advisories                       the nightly's advisory check
  unused                           the nightly's unused-dependency check

The nightly's jobs take no words, no --filter, --count, --load, --cores or
--against, and run in the ci profile.

A word of `test` or `stress` that is not `-p CRATE` goes to nextest: a test's
name, or an option such as `--run-ignored only`. --filter takes nextest filter
expressions, separated by `;`, each passed as one `-E` argument, spaces and
all: `test(/shell::/) & !test(=slow); package(cortex-agent)`. nextest runs a
test that any of them matches.

--against REF makes `test` an A/B run: the workflow checks REF out beside the
branch, and `cargo xtask ab` builds both once and runs them in turn, --count
rounds (5 when not given), on one runner, then compares them by tier, crate
and test. It runs with --keep-going: a round in which a test fails is recorded
and left out, the run goes on, and it ends as a failure after the comparison of
the rounds that finished.

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
const COMMANDS: [&str; 10] = [
    "test",
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
#[derive(Debug, Default, PartialEq)]
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
    /// The workflow's own inputs, which a dry run checks and a run is not
    /// given.
    os: Option<String>,
    testkit: Option<String>,
    /// `capture no`: a passing test's output is shown (nextest's
    /// `--no-capture`, which runs the tests one at a time).
    show_output: bool,
}

/// The `os` inputs of the workflow.
const OSES: [&str; 4] = ["both", "linux", "windows", "linux-22.04"];

/// The `testkit` inputs of the workflow.
const TESTKITS: [&str; 2] = ["none", "profile"];

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
                            return Err(format!("`--testkit {testkit}`: none or profile"));
                        }
                        inputs.testkit = Some(testkit.to_owned());
                    }
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
const NOT_FOR_NEXTEST: [(&str, &str); 23] = [
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
        refuse_foreign_words(rest, into_stress)
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
                "`mutate` builds its own copy of the tree, as it does at a desk: the ci profile"
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
                        "`{word}` runs no break: it is for a desk, which reads the lists and the tree. A try run runs the breaks"
                    ));
                }
                "--minutes" => {
                    return Err(
                        "`--minutes`: a try run gives `--minutes 0` itself, so that the run goes to its end in one job"
                            .to_owned(),
                    );
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
                        "`{option}` is not an option a try run gives `mutate`: --jobs, --again,                          --no-compile-check and --file"
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
            against,
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
        let mut command: Vec<String> = [
            "ab",
            "--base",
            AB_BASE,
            "--label",
            against,
            "--top",
            "20",
            "--keep-going",
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(count) = self.count {
            command.extend(["--count".to_owned(), count.max(2).to_string()]);
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
    let command = inputs.command()?;
    if let Some(commit) = &inputs.commit {
        notes.push(format!(
            "commit {commit}: the branch's side of the run is that commit, whose own `xtask` \
             runs the command"
        ));
    }
    if inputs.dry_run {
        if inputs.testkit.as_deref() == Some("profile") && inputs.against.is_some() {
            notes.push(
                "testkit=profile is not set for an `against` run: the workflow leaves it out"
                    .to_owned(),
            );
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
    println!("$ cargo xtask {}", plan.command.join(" "));
    if plan.inputs.dry_run {
        println!("(a dry run: nothing was built or run)");
        return Ok(ExitCode::SUCCESS);
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
    crate::run(first, rest).map(|()| ExitCode::SUCCESS)
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
    use super::{Inputs, check_checkout, plan};
    use crate::workspace::{DepKind, Dependency, Package};

    #[test]
    fn capture_no_shows_a_passing_tests_output_and_goes_only_where_it_means_that() {
        assert_eq!(
            try_command(&["--command", "test -p cortex-tui", "--capture", "no"]).unwrap(),
            "ci --tiers --top 20 -p cortex-tui -- --no-capture"
        );
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cortex-tui",
                "--capture",
                "no",
                "--filter",
                "test(/frame/)"
            ])
            .unwrap(),
            "ci --tiers --top 20 -p cortex-tui -- -E test(/frame/) --no-capture"
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
            for fetches in ["perf -p cortex-index", "perf -p cortex-index warm"] {
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
            ("test -p cortex-index", "release"),
            ("test -p cortex-index", "ci"),
            ("stress -p cortex-index", "ci"),
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
                "test -p cortex-tools shell::",
                "--profile",
                "",
                "--count",
                ""
            ]),
            Ok("ci --tiers --top 20 -p cortex-tools -- shell::".to_owned())
        );
        assert_eq!(
            try_command(&["--command", "test"]),
            Ok("ci --tiers --top 20".to_owned())
        );
        assert_eq!(
            try_command(&["--command", "test -p cortex-base", "--profile", "release"]),
            Ok("test --cargo-profile release --profile ci -p cortex-base".to_owned())
        );
    }

    #[test]
    fn a_count_makes_a_test_run_a_stress_run() {
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cortex-tools shell::cmd",
                "--count",
                "100"
            ]),
            Ok("stress --count 100 --cargo-profile ci -p cortex-tools shell::cmd".to_owned())
        );
        assert_eq!(
            try_command(&[
                "--command",
                "stress -p cortex-testkit",
                "--count",
                "50",
                "--load",
                "6",
                "--cores",
                "2"
            ]),
            Ok(
                "stress --count 50 --cargo-profile ci -p cortex-testkit --load 6 --cores 2"
                    .to_owned()
            )
        );
        // A count in the command wins.
        assert_eq!(
            try_command(&[
                "--command",
                "stress -p cortex-agent --count 300",
                "--count",
                "5"
            ]),
            Ok("stress --cargo-profile ci -p cortex-agent --count 300".to_owned())
        );
    }

    #[test]
    fn filters_and_nextest_options_reach_nextest_as_they_are_meant() {
        // Several filter expressions, spaces and all, each one argument.
        let arguments: Vec<String> = [
            "--command",
            "test --package=cortex-tools -pcortex-agent --run-ignored only shell::",
            "--filter",
            "test(/shell::/) & !test(=slow) ; package(cortex-agent) ;",
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
                "cortex-tools",
                "-p",
                "cortex-agent",
                "--",
                "--run-ignored",
                "only",
                "shell::",
                "-E",
                "test(/shell::/) & !test(=slow)",
                "-E",
                "package(cortex-agent)"
            ]
        );
        assert_eq!(
            try_command(&[
                "--command",
                "test -p cortex-tools",
                "--count",
                "20",
                "--filter",
                "test(~shell)"
            ]),
            Ok("stress --count 20 --cargo-profile ci -p cortex-tools -E test(~shell)".to_owned())
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
                "test -p cortex-core",
                "--against",
                "main",
                "--count",
                "6",
                "--filter",
                "test(/slow/)"
            ]),
            Ok("ab --base ../ab-base --label main --top 20 --keep-going --count 6 -p cortex-core -- -E test(/slow/)"
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
            try_command(&["--command", "ci --tiers -p cortex-llm"]),
            Ok("ci --tiers -p cortex-llm".to_owned())
        );
    }

    #[test]
    fn perf_with_a_crate_is_that_crates_ignored_perf_tests_in_release_and_nothing_else() {
        let tests = "--run-ignored only -E test(/(^|::)perf_/) --no-capture --test-threads 1";
        assert_eq!(
            try_command(&["--command", "perf -p cortex-index"]),
            Ok(format!(
                "test --cargo-profile release --profile perf -p cortex-index {tests}"
            ))
        );
        // The workflow's own default profile is `ci`: still a release build.
        assert_eq!(
            try_command(&[
                "--command",
                "perf --package=cortex-index -pcortex-session",
                "--profile",
                "ci"
            ]),
            Ok(format!(
                "test --cargo-profile release --profile perf -p cortex-index -p cortex-session {tests}"
            ))
        );
        assert_eq!(
            try_command(&["--command", "perf -p cortex-index", "--profile", "dist"]),
            Ok(format!(
                "test --cargo-profile dist --profile perf -p cortex-index {tests}"
            ))
        );
        // A name after the crate narrows the tests: nextest takes a test that
        // matches the name and the filter.
        assert_eq!(
            try_command(&["--command", "perf -p cortex-index warm"]),
            Ok(format!(
                "test --cargo-profile release --profile perf -p cortex-index {tests} warm"
            ))
        );
        // Without a crate it is step 20's measurement, as it was.
        assert_eq!(
            try_command(&["--command", "perf --only editor"]),
            Ok("perf --only editor".to_owned())
        );
        for (arguments, expected) in [
            (
                &["--command", "perf -p cortex-index", "--count", "3"][..],
                "runs each perf test once",
            ),
            (
                &["--command", "perf -p cortex-index", "--filter", "test(x)"],
                "goes with `test` or `stress`",
            ),
            (
                &["--command", "perf -p cortex-index", "--load", "2"],
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
                "none or profile",
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
            try_command(&["--command", "perf -p cortex-index", "--against", "main"]),
            Ok(
                "perf-ab --base ../ab-base --label main --cargo-profile release -p cortex-index"
                    .to_owned()
            )
        );
        // Several crates, a test's name, dist, and a count of rounds (three
        // at least: with two, ranges of one tree often do not overlap).
        assert_eq!(
            try_command(&[
                "--command",
                "perf -p cortex-index -pcortex-tools warm",
                "--against",
                "origin/main",
                "--profile",
                "dist",
                "--count",
                "1"
            ]),
            Ok(
                "perf-ab --base ../ab-base --label origin/main --cargo-profile dist --count 3 \
                -p cortex-index -p cortex-tools warm"
                    .to_owned()
            )
        );
        assert_eq!(
            try_command(&[
                "--command",
                "perf -p cortex-index",
                "--against",
                "main",
                "--profile",
                "release",
                "--count",
                "5"
            ]),
            Ok(
                "perf-ab --base ../ab-base --label main --cargo-profile release --count 5 \
                -p cortex-index"
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
            "perf -p cortex-index",
            "--against",
            "main"
        ])));
        assert!(!super::reads_the_corpus(&command(&[
            "--command",
            "test -p cortex-index",
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
        let arguments: Vec<String> = ["--command", "test -p cortex-base", "--commit", HASH]
            .map(str::to_owned)
            .to_vec();
        let planned = plan(&arguments, || Ok(Vec::new())).unwrap();
        assert_eq!(
            planned.command,
            ["ci", "--tiers", "--top", "20", "-p", "cortex-base"]
        );
        assert!(planned.notes[0].contains(HASH), "{:?}", planned.notes);
    }
}
