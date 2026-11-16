//! Developer commands for the Cortex-Harness repository.
//!
//! `cargo xtask <command>`, or `./x <command>` from a shell. The commands are
//! listed in [`USAGE`]. CI runs these same commands, so what passes here
//! passes there.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool for developers; its output is its interface"
)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a synchronous tool: it reads files and runs commands, and has no async runtime"
)]

use std::process::ExitCode;

mod ab;
mod alias;
mod api_diff;
mod checks;
mod ci_log;
mod combine;
mod compare;
mod corpus;
mod decide;
mod dependents;
mod junit;
mod last_good;
mod like_runner;
mod load;
mod mutate;
mod nonblocking;
mod outside;
mod owner_guard;
mod pattern;
mod perf;
mod records;
mod regen;
mod replace;
mod rules;
mod runs;
mod schema;
mod scratch_test;
mod screens;
mod shipped;
mod stall;
mod startup;
mod steps;
mod stress;
mod tiers;
mod try_result;
mod try_run;
mod workspace;

/// Failures are messages for the person at the terminal.
type Result<T> = std::result::Result<T, String>;

const USAGE: &str = "\
Developer commands for the Cortex-Harness repository.

Usage: cargo xtask <command> [arguments]        (or: ./x <command> [arguments])

Checks and tests
  check [STEP...]   Static checks (tier 0): fmt, rules, breaks, generated, clippy,
                    deny. With step names, only those steps.
  test [ARGS...]    All tests, with nextest. Arguments go to `cargo nextest run`,
                    for example: test --profile fast     (tiers 1 and 2 only)
                                 test -p cortex-agent    (one crate, and only it)
                    --dependents -p CRATE: the crate's tests and those of
                    every workspace crate that depends on it, directly or
                    not. Run it after a change of behaviour.
  ci [lint|test]    Exactly what the blocking CI jobs run. No argument: both.
  ci --tiers [-p CRATE]... [--top N]
                    Build and run the tests as CI does (third-party crates not
                    optimised) and report each tier's time against its budget
                    and its slowest tests. With --from JUNIT.XML [--os
                    linux|windows], report on a run that was already made:
                    the report a CI job uploads, for example.
  ci --tiers --compare NEW [--against OLD] [--os linux|windows|both] [--top N]
                    Two runs' tiers side by side, each above its own start
                    cost: by tier, by crate, and the tests that moved most.
                    NEW and OLD are JUnit reports, run ids of the `ci`
                    workflow or of a `try` run of `test` (read through gh), or
                    `main`, the main branch's latest successful run (OLD when
                    it is not given). --os both: each OS's comparison in turn.
  ci-times RUN [--against RUN|main] [--pr N] [--write FILE]
           [--section NAME] [--heading TEXT]
                    A run's job times and tier times, as tables for a step's
                    ci.md, read through gh; --write puts them in its section
                    `On the pull request`, or in the one --section names (its
                    own marks) or --heading adds (for an older page).
  ci-times --ab TRY-RUN [--against REF] [--write FILE] [--section NAME]
           [--heading TEXT] [--top N]
                    A try A/B run's tiers on both sides, for each OS's section
                    `<OS>, both trees on one runner`, and the PR's CI section,
                    then each tier's crates and tests that moved, by OS.
  try-result RUN [--os linux|windows|linux-22.04] [--top N] [--tail N]
                    What a try run found, in one step: for each OS, the A/B's
                    tables or the tiers or the stress summary, every failed
                    test with its message, and for a job that failed with no
                    report the end of its log. Fails when the run did.
  ci-log RUN|JOB [--job NAME] [--all] [--grep RE] [--context N] [--tail N]
                    A CI job's log through gh, escape codes taken out, for any
                    workflow (the nightly's too): a run's failed jobs, the jobs
                    named, or all; or one job by its id. --grep prints the
                    matching lines with their neighbours.
  rules             The repo-rule checks alone: no home-directory paths, the
                    crate dependency rules, docs/dependencies.md, workflows,
                    links and the layout of the step records.
  links             The part of `rules` that is about the docs: every link
                    inside the repository leads to a file and a heading that
                    are there, and docs/steps/ and docs/outstanding/ hold
                    what they should. It asks cargo nothing.
  owner-guard --base REV --head REV
                    Fail when the change adds one of the owner's personal
                    details: the terms of OWNER_TERMS, which CI fills from a
                    secret. Without terms it passes with a warning.
  owner-guard --pre-push REMOTE [URL] | --unpushed [REMOTE]
                    The same check before a push, of each commit it sends,
                    with the terms from ~/.config/cortex-harness/owner-terms
                    or CORTEX_OWNER_TERMS_FILE: the git hook in
                    xtask/hooks/pre-push runs the first.

Showing that the tests bite
  stress -p CRATE... [--count N] [--first-seed S] [--load N] [--cores K] [ARGS...]
                    Run the crates' tests N times over (100 when not given),
                    each time under the next seed, and report the tests that
                    fail on some, with the seed that brings each failure back.
                    --cores K pins the run to K cores, beside --load N busy
                    loops, to make a race show.
  mutate [BREAK...] Break the code on purpose, one break of mutations/*.breaks
                    at a time, on a copy of the tree, and report which tests
                    fail. --check only checks that every break still applies
                    (`check` runs that), and that each `caught-by` names a
                    test that exists where a fresh list of tests is kept;
                    --check --tests builds that list, --check --fix-in
                    [--write] offers the new `in:` of code that moved.
                    `mutate --help` has the rest.

On the runners, without a pull request (the `try` workflow runs this)
  try --command 'COMMAND [ARGS]' [--profile ci|release|dist] [--count N]
      [--load N] [--cores K] [--filter 'EXPR; EXPR'] [--against REF]
      [--commit SHA] [--dry-run]
                    One of test, stress, startup, perf, ci or mutate, as the
                    workflow's inputs ask: `try --help` has the rules. `perf
                    -p CRATE` is the crate's ignored perf_ tests in a release
                    build; `--dependents` is resolved to the crates it adds.
                    --dry-run prints the command the job would run and fails
                    as the job would, building nothing.
  ab --base DIR [--label NAME] [--count N] [--keep-going] [-p CRATE]...
     [-- NEXTEST ARGS]
                    This tree's tests and those of the tree at DIR, each built
                    once, run in turn N rounds on this machine, and compared
                    by each test's median: a change's cost apart from the
                    runner's speed. --keep-going records a round with a
                    failing test and carries on.

Reviews
  scratch-test CRATE FILE [--cargo-profile P] [--ignored|--include-ignored]
               [--no-capture] [-- NEXTEST ARGS]
                    Run the tests of one throwaway file as a module of CRATE's
                    integration tests, in a copy of the tree under
                    target/scratch-test/: nothing to register, nothing left
                    in the worktree. --cargo-profile release builds it
                    optimised; --ignored runs its ignored tests (a
                    measurement) alone, one at a time, with their output.
  api-diff [--against REF] [-p CRATE]... [--format text|markdown]
                    The public items added, removed and changed between REF
                    (main when not given) and the working tree, by crate, each
                    with its signature before and after: read from rustdoc's
                    JSON, in build directories of their own under
                    target/api-diff/. For a fix session's list of changed
                    signatures.

Before a push
  combine PR...     Put this branch on top of the open pull requests named, in
                    order, in a scratch worktree under the build directory,
                    and run `cargo xtask ci` there. This worktree and branch
                    are not touched. `combine --help` has the rest.

Not blocking (the `main` and `nightly` workflows run these)
  size [--check] [--target TRIPLE] [--against-main]
                    Build the `dist` profile, with the build machine's folders
                    taken out of its paths, and report the binary's size and
                    the number of crates in it. It fails when the binary holds
                    a path under a home folder; --check fails over the budget
                    too. --against-main also reports the difference from the
                    size the `main` workflow last recorded (through gh).
  home-paths FILE...
                    Fail when a built file holds a path under a home folder
                    (/home/…, /Users/…, C:\\Users\\…) or under cargo's home.
  startup BINARY|--profile PROFILE [--runs N] [--budget desktop|linux|windows]
          [--check]
                    Time `--version` under both names, and a headless run to
                    its request, against the start-up budgets. With
                    --profile, build `cortex` in that profile first.
  perf [--runs N] [--seconds N] [--json PATH]
                    Measure the terminal layer's demo, optimised, in a
                    pseudo-terminal: start to first frame, key to frame idle
                    and while streaming, frame cost, frame intervals and bytes
                    per frame, against the TUI's budgets.
  tui-screens [--write]
                    Draw the reference screens of the TUI's look again with
                    the real views and compare every cell (a tier-2 test);
                    print each screen's rows compared and not built yet.
                    --write rewrites the rows that differ, from the views,
                    and lists every cell changed.
  unused            Look for dependencies that no code uses (cargo-machete).
  partial-landlock  The sandbox's tests on a kernel whose Landlock is older
                    than 6, failing unless the run showed that partial case.
                    `try` runs it, and advisories and unused, as named jobs.
  advisories        Check the lockfile against the advisory database.
  fuzz [--seconds N] [TARGET...]
                    Fuzz the stream decoders, each target for N seconds
                    (60 when not given). Needs a nightly toolchain and
                    cargo-fuzz.

By hand only, against a real provider (never in CI)
  live [PROVIDER]   The live smoke test, against anthropic (when no provider
                    is named), openrouter, deepseek or venice. For anthropic
                    and deepseek, then one headless run of the real binary.
  live --headless [PROVIDER] [--test NAME]... [--dry-run]
                    Every headless run of the real binary against the provider
                    (anthropic when none is named, or deepseek): cortex-cli's
                    tests/system/live.rs, under cargo test, with no time limit.
                    --test runs the one named (its name, or the end of it:
                    compaction); --dry-run prints what would run and sends,
                    builds and reads nothing.
  live --spent [--since YYYY-MM-DD]
                    Total the spend ledger: every live call's provider, model,
                    tokens and cost, kept outside the repository.
  record [--provider PROVIDER] [--dry-run] [SCENARIO...]
                    Record provider fixtures, scrubbed. No name: all of them.
                    --dry-run prints the first request and sends nothing.
  record --toolbox [--provider PROVIDER]
                    Keep the tools the binary sends each provider, and record
                    each one's toolbox fixture: the provider named, or every
                    provider whose key file is named.
                    Both need CORTEX_LIVE_<PROVIDER>_KEY_FILE set to the path
                    of a file that holds that provider's API key. The key is
                    never given on a command line.

Working on the code
  fmt               Format the workspace.
  decide [--level L] [--rule RULE]... [--untrusted] [--] COMMAND...
                    What the permission engine decides of a shell command,
                    with its reasons. `decide --help` has the rest.
  replace FILE --from CODE --to CODE [--in ITEM] [--occurrence N] [--dry-run]
                    Replace Rust code found by its tokens, so the match holds
                    however rustfmt laid it out. It must be there once.
                    --from-file and --to-file read the code from files.
                    --text: exact text, for Markdown and other files.
                    --script FILE: many edits across many files, all or none.
                    When xtask does not build, `./x replace …` runs the last
                    binary that worked, from target/xtask-last-good/ (it says
                    how old it is, and offers only `replace` and `links`).
  schema [--check]  Regenerate the protocol's JSON Schema and TypeScript
                    declarations in docs/protocol/. --check only compares.
  gen               Regenerate every checked-in generated file: what `schema`
                    writes, and the configuration schema and reference in
                    docs/configuration/.
  regen [--list] [GENERATOR...]
                    Rebuild everything that can be rebuilt from the tree, with
                    no network and no key: what `gen` writes, every snapshot
                    (CORTEX_UPDATE_SNAPSHOTS) and the reference screens
                    (tui-screens --write). For after a rebase that met changes
                    to a generated file: settle the sources, take either side
                    of the file, run this. Ends by listing what differs from
                    HEAD and checking `links`. --list names what it writes and
                    what it leaves to other commands, and why.
  alias [--release] Build `cortex` and put its alias `cx` beside it.
  corpus fetch|check|path|pin [--update]
                    The pinned corpus of about 20,000 `.rs` files of real
                    code that perf_ tests measure, kept outside the repository
                    (~/.cache/cortex-harness/corpus): fetch makes it when it
                    is not there whole, check and path verify it against
                    corpus/PIN, pin makes a new one. `corpus` alone has the
                    rest.

Not built yet
  dist              Package release binaries (step 34).
";

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next();
    let rest: Vec<String> = arguments.collect();

    // The copy of xtask kept for when it does not build (`last_good`) says
    // that it is one, and offers only some commands.
    let copy = std::env::current_exe()
        .ok()
        .filter(|exe| last_good::is_the_copy(exe));
    if let Some(exe) = &copy
        && let Err(message) = last_good::announce(&workspace::root(), exe, command.as_deref())
    {
        eprintln!("xtask: {message}");
        return ExitCode::FAILURE;
    }

    let outcome = match command.as_deref() {
        None | Some("help" | "--help" | "-h") => {
            print!("{USAGE}");
            Ok(())
        }
        // Its exit code says more than pass or fail.
        Some("mutate") => return mutate::main(&rest),
        // It runs `mutate` too, and passes that one's exit code on.
        Some("try") => return try_run::main(&rest),
        Some(command) => run(command, &rest),
    };

    // A binary that has just run a command to its end is a good one.
    if outcome.is_ok() && copy.is_none() {
        last_good::keep_after_success();
    }

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Runs one command. `try` comes back here with the command it chose.
fn run(command: &str, rest: &[String]) -> Result<()> {
    match command {
        "check" => checks::check(rest),
        "test" => checks::test(rest),
        "ci" => checks::ci(rest),
        "ci-times" => runs::ci_times(rest),
        "ci-log" => ci_log::run(rest),
        "replace" => replace::run(rest),
        "rules" => checks::rules(),
        "links" => checks::links(),
        "fmt" => checks::fmt(),
        "decide" => decide::run(rest),
        "gen" => generate(),
        "regen" => regen::run(rest),
        "schema" => schema::run(rest),
        "size" => nonblocking::size(rest),
        "home-paths" => shipped::home_paths_command(rest),
        "startup" => startup::run(rest),
        "unused" => nonblocking::unused(),
        "partial-landlock" => nonblocking::partial_landlock(),
        "advisories" => nonblocking::advisories(),
        "alias" => alias::run(rest),
        "corpus" => corpus::run(rest),
        "stress" => stress::run(rest),
        "try-result" => try_result::run(rest),
        "ab" => ab::run(rest),
        "api-diff" => api_diff::run(rest),
        "combine" => combine::run(rest),
        "scratch-test" => scratch_test::run(rest),
        "owner-guard" => owner_guard::run(rest),
        "fuzz" => outside::fuzz(rest),
        "live" => outside::live(rest),
        "record" => outside::record(rest),
        "perf" => perf::run(rest),
        "tui-screens" => screens::run(rest),
        "dist" => not_built("dist", 34),
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

/// `cargo xtask gen`: every checked-in generated file.
///
/// The protocol's files are generated here, from the protocol crate. The
/// configuration's are generated by the test in `cortex-config` that checks
/// them, run in its writing mode, so that this crate does not have to build
/// the configuration crate and its TOML parser before every CI job.
fn generate() -> Result<()> {
    schema::write()?;
    generate_configuration()
}

/// The configuration's schema and reference, written by the test of
/// `cortex-config` that checks them.
fn generate_configuration() -> Result<()> {
    steps::cargo_with(
        &[
            "test",
            "--locked",
            "-p",
            "cortex-config",
            "--test",
            "it",
            "--",
            "generated::",
            "--nocapture",
        ],
        &[("CORTEX_UPDATE_GENERATED", "1")],
    )
}

fn not_built(command: &str, step: u32) -> Result<()> {
    Err(format!(
        "`{command}` is not built yet. It arrives with step {step} of docs/build-plan.md."
    ))
}
