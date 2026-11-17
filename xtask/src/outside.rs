//! The commands that reach outside the repository: the real provider
//! (`live`, `record`) and the fuzzer (`fuzz`). None of them is part of the
//! blocking checks (`docs/testing-and-ci.md`, tiers 5 and 6).
//!
//! # `live` and `record`
//!
//! Both run ignored tests of `cortex-llm` (the `live` module of each
//! protocol's tests), which is where the HTTP client is. Doing it here
//! would mean building the whole HTTP and TLS stack into this crate, which
//! every CI job builds before it does anything else. For Anthropic, `live`
//! then runs the one live run of the whole harness: an ignored system test
//! of `cortex-cli` that starts the real binary headless (step 13b).
//!
//! Each takes the provider to talk to ([`PROVIDERS`]); without one it is
//! Anthropic. The API key is never an argument and never in the
//! environment. The environment names a **file**, in the provider's own
//! variable, and the test reads the key from it. Without the variable
//! these commands say so and stop.
//!
//! # `fuzz`
//!
//! Runs each fuzz target of the `fuzz/` package for a fixed time, with
//! `cargo fuzz`, which needs a nightly toolchain and the `cargo-fuzz`
//! subcommand. The recorded fixtures are the corpus each run starts from.
//!
//! The nightly toolchain is pinned, to a date, in `fuzz/rust-toolchain.toml`
//! and nowhere else: the commands here run inside `fuzz/`, where that file
//! decides the toolchain.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::Result;
use crate::workspace;

/// A provider the live commands can talk to.
#[derive(Debug)]
struct LiveProvider {
    /// The name the commands take.
    name: &'static str,
    /// The environment variable that names the file its API key is in.
    key_file: &'static str,
    /// Its smoke test and its recorder: ignored tests of `cortex-llm`.
    smoke: &'static str,
    record: &'static str,
    /// The free check of its usage endpoint, `live --refresh`: an ignored
    /// test of `cortex-llm`. `None` for a provider with no endpoint.
    refresh: Option<&'static str>,
    /// Headless runs of the real binary against it: ignored tests of
    /// `cortex-cli`'s system tests (`tests/system/live.rs`). `live` runs
    /// the first after the smoke test; `live --headless` runs them all.
    headless: &'static [&'static str],
}

/// The providers the live commands can talk to. The first is the one a
/// command talks to when it names none.
const PROVIDERS: &[LiveProvider] = &[
    LiveProvider {
        name: "anthropic",
        key_file: "CORTEX_LIVE_ANTHROPIC_KEY_FILE",
        smoke: "anthropic::live::live_smoke",
        record: "anthropic::live::live_record",
        refresh: None,
        headless: &[
            "live::live_headless_run",
            "live::live_headless_compaction",
            "live::live_headless_budget_summary",
        ],
    },
    LiveProvider {
        name: "openrouter",
        key_file: "CORTEX_LIVE_OPENROUTER_KEY_FILE",
        smoke: "openai_chat::live::live_smoke_openrouter",
        record: "openai_chat::live::live_record_openrouter",
        refresh: Some("openai_chat::live::live_refresh_openrouter"),
        headless: &[],
    },
    LiveProvider {
        name: "deepseek",
        key_file: "CORTEX_LIVE_DEEPSEEK_KEY_FILE",
        smoke: "openai_chat::live::live_smoke_deepseek",
        record: "openai_chat::live::live_record_deepseek",
        refresh: Some("openai_chat::live::live_refresh_deepseek"),
        headless: &["live::live_headless_run_deepseek"],
    },
    LiveProvider {
        name: "venice",
        key_file: "CORTEX_LIVE_VENICE_KEY_FILE",
        smoke: "openai_chat::live::live_smoke_venice",
        record: "openai_chat::live::live_record_venice",
        refresh: Some("openai_chat::live::live_refresh_venice"),
        headless: &[],
    },
];

fn live_provider(name: &str) -> Result<&'static LiveProvider> {
    PROVIDERS
        .iter()
        .find(|provider| provider.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = PROVIDERS.iter().map(|provider| provider.name).collect();
            format!(
                "there are no live tests for a provider `{name}`. The providers are: {}",
                known.join(", ")
            )
        })
}

/// The environment variable the recorder reads its scenarios from.
const SCENARIOS: &str = "CORTEX_LIVE_SCENARIOS";

/// Set for a dry run: requests are built and printed, and none is sent.
const DRY_RUN: &str = "CORTEX_LIVE_DRY_RUN";

/// The file each live call's spend is added to, as a line of JSON.
const LEDGER: &str = "CORTEX_LIVE_LEDGER";

/// What a dry run prints when it has printed its request.
const DRY_RUN_ENDED: &str = "dry run: nothing was sent.";

/// Where the spend ledger is: `CORTEX_LIVE_LEDGER` if it is set, or a file
/// in the user's state directory, outside every repository. On the desktop
/// that `rx` runs on, the ledger is the desktop's.
fn ledger() -> Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(LEDGER).filter(|path| !path.is_empty()) {
        return Ok(path.into());
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::Path::new(&home).join(".local").join("state"))
            })
    };
    let base = base
        .ok_or("there is no state directory to keep the spend ledger in: set CORTEX_LIVE_LEDGER")?;
    let directory = base.join("cortex-live");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    Ok(directory.join("spend.jsonl"))
}

/// The ledger's variable, for a live test to add its spend to.
fn ledger_variable() -> Result<(&'static str, String)> {
    Ok((LEDGER, ledger()?.to_string_lossy().into_owned()))
}

/// What the ledger says: each provider's and model's calls, tokens and
/// cost, and the total. `since` keeps the lines of that date or later
/// (`YYYY-MM-DD`).
fn spent(text: &str, since: Option<&str>) -> String {
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Sum {
        calls: u64,
        input: u64,
        output: u64,
        nanos: u64,
    }
    let mut sums: BTreeMap<(String, String), Sum> = BTreeMap::new();
    let mut unread = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            unread += 1;
            continue;
        };
        let field = |name: &str| {
            entry
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?")
        };
        let number = |name: &str| {
            entry
                .get(name)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        if since.is_some_and(|since| field("date") < since) {
            continue;
        }
        let sum = sums
            .entry((field("provider").to_owned(), field("model").to_owned()))
            .or_default();
        sum.calls += 1;
        sum.input += number("input") + number("cache_read") + number("cache_write");
        sum.output += number("output");
        sum.nanos += number("usd_nanos");
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a figure for a person to read; the sums are whole nano-dollars"
    )]
    let dollars = |nanos: u64| nanos as f64 / 1e9;
    let mut shown = format!(
        "  {:<12} {:<40} {:>6} {:>12} {:>10} {:>11}\n",
        "provider", "model", "calls", "input", "output", "cost"
    );
    let mut total = Sum::default();
    let mut by_provider: BTreeMap<&str, u64> = BTreeMap::new();
    for ((provider, model), sum) in &sums {
        shown.push_str(&format!(
            "  {provider:<12} {model:<40} {:>6} {:>12} {:>10} {:>10.6}$\n",
            sum.calls,
            sum.input,
            sum.output,
            dollars(sum.nanos)
        ));
        *by_provider.entry(provider).or_default() += sum.nanos;
        total.calls += sum.calls;
        total.nanos += sum.nanos;
    }
    for (provider, nanos) in by_provider {
        shown.push_str(&format!("  {provider}: ${:.6}\n", dollars(nanos)));
    }
    shown.push_str(&format!(
        "  in all: {} call(s), ${:.6}{}\n",
        total.calls,
        dollars(total.nanos),
        since
            .map(|since| format!(" since {since}"))
            .unwrap_or_default()
    ));
    if unread > 0 {
        shown.push_str(&format!(
            "  ({unread} line(s) of the ledger could not be read)\n"
        ));
    }
    shown
}

/// The fuzz targets, and the fixtures each one starts from, a directory
/// under the repository: a protocol's recorded streams in `cortex-llm`, the
/// outline extractors' sample files in `cortex-index` (since the fixes of
/// step 18's review), or the lines a client sends the RPC server in
/// `cortex-rpc` (since step 27).
const FUZZ_TARGETS: &[(&str, &str)] = &[
    (
        "sse_parser",
        "crates/cortex-llm/fixtures/anthropic-messages",
    ),
    (
        "anthropic_decoder",
        "crates/cortex-llm/fixtures/anthropic-messages",
    ),
    (
        "openai_chat_decoder",
        "crates/cortex-llm/fixtures/openai-chat",
    ),
    ("extractors", "crates/cortex-index/fixtures/outlines"),
    ("rpc_decoder", "crates/cortex-rpc/fixtures/lines"),
];

/// How long each fuzz target runs when no time is given, in seconds.
const FUZZ_SECONDS: u64 = 60;

fn require_key_file(provider: &LiveProvider) -> Result<()> {
    require_key_file_in(provider, |name| std::env::var_os(name))
}

/// [`require_key_file`], with the environment `var` reads.
fn require_key_file_in(
    provider: &LiveProvider,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<()> {
    let variable = provider.key_file;
    match var(variable) {
        Some(path) if !path.is_empty() => Ok(()),
        _ => Err(format!(
            "{variable} is not set. Set it to the path of a file that holds the API key of \
             `{}` and nothing else. The key itself is never given on a command line or in \
             the environment. Nothing was sent.",
            provider.name
        )),
    }
}

/// Runs one ignored test of `cortex-llm`'s integration binary, and shows
/// its output.
///
/// A test that was renamed is a test that does not run: cargo then runs
/// nothing and reports success. So the output is read, and the command
/// fails unless exactly one test ran and passed. What the test prints has
/// already been checked for the key by the test itself.
fn ignored_test(name: &str, variables: &[(&str, &str)]) -> Result<()> {
    run_ignored(
        &[
            "--package",
            "cortex-llm",
            "--features",
            "http",
            "--test",
            "it",
        ],
        name,
        variables,
    )
}

/// Runs one ignored test of the test binary `selection` names, and shows
/// its output. Fails unless exactly one test ran and passed.
pub(crate) fn run_ignored(
    selection: &[&str],
    name: &str,
    variables: &[(&str, &str)],
) -> Result<()> {
    run_ignored_with(selection, name, variables, false)
}

/// Runs one test that is not ignored, and shows its output. Fails unless
/// exactly one test ran and passed: a renamed test is a test that does not
/// run, and cargo calls that a success.
pub(crate) fn run_test(
    selection: &[&str],
    name: &str,
    variables: &[(&str, &str)],
    ignored: bool,
) -> Result<()> {
    let mut arguments = vec!["test", "--locked"];
    arguments.extend_from_slice(selection);
    arguments.push("--");
    if ignored {
        arguments.push("--ignored");
    }
    arguments.extend_from_slice(&["--exact", name, "--nocapture"]);
    println!("$ cargo {}", arguments.join(" "));
    let output = Command::new(workspace::cargo())
        .args(arguments)
        .envs(variables.iter().copied())
        .current_dir(workspace::root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("could not run cargo test: {error}"))?;
    let shown = String::from_utf8_lossy(&output.stdout);
    print!("{shown}");
    if !output.status.success() {
        return Err(format!("the test `{name}` failed"));
    }
    if ran_one_test(&shown) {
        Ok(())
    } else {
        Err(format!(
            "the test `{name}` did not run: cargo found no test of that name"
        ))
    }
}

/// [`run_ignored`]; for a dry run, which ends the test's process when it
/// has printed its request, the dry run's last line is what says it ran.
fn run_ignored_with(
    selection: &[&str],
    name: &str,
    variables: &[(&str, &str)],
    dry_run: bool,
) -> Result<()> {
    let mut arguments = vec!["test", "--locked"];
    arguments.extend_from_slice(selection);
    arguments.extend_from_slice(&["--", "--ignored", "--exact", name, "--nocapture"]);
    println!("$ cargo {}", arguments.join(" "));
    let output = Command::new(workspace::cargo())
        .args(arguments)
        .envs(variables.iter().copied())
        .current_dir(workspace::root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("could not run cargo test: {error}"))?;
    let shown = String::from_utf8_lossy(&output.stdout);
    print!("{shown}");
    if !output.status.success() {
        return Err(format!("the test `{name}` failed"));
    }
    if dry_run {
        return if shown.contains(DRY_RUN_ENDED) {
            Ok(())
        } else {
            Err(format!(
                "the dry run of `{name}` printed no request: the test did not run, or sent \
                 nothing before it ended"
            ))
        };
    }
    if ran_one_test(&shown) {
        Ok(())
    } else {
        Err(format!(
            "the test `{name}` did not run: cargo found no test of that name. Nothing was sent."
        ))
    }
}

/// Whether the output of `cargo test` says that exactly one test ran and
/// passed.
fn ran_one_test(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;"))
}

/// `cargo xtask live [PROVIDER]`: the smoke test against a real API.
/// Anthropic's when no provider is named.
pub(crate) fn live(arguments: &[String]) -> Result<()> {
    if arguments.first().is_some_and(|first| first == "--spent") {
        let since = match &arguments[1..] {
            [] => None,
            [flag, date] if flag == "--since" => Some(date.as_str()),
            _ => return Err("usage: cargo xtask live --spent [--since YYYY-MM-DD]".to_owned()),
        };
        let path = ledger()?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        println!("The spend ledger, {}:", path.display());
        print!("{}", spent(&text, since));
        return Ok(());
    }
    if arguments.first().is_some_and(|first| first == "--refresh") {
        // One request to the provider's usage endpoint, which costs
        // nothing: no model is asked.
        let provider = match &arguments[1..] {
            [name] => live_provider(name)?,
            _ => return Err("usage: cargo xtask live --refresh PROVIDER".to_owned()),
        };
        let Some(test) = provider.refresh else {
            return Err(format!(
                "`{}` documents no usage endpoint that a key for model calls can ask",
                provider.name
            ));
        };
        require_key_file(provider)?;
        let (name, path) = ledger_variable()?;
        return ignored_test(test, &[(name, &path)]);
    }
    if arguments.first().is_some_and(|first| first == "--headless") {
        let chosen = headless_tests(&arguments[1..], |name| std::env::var_os(name))?;
        let (provider, tests) = (chosen.provider, &chosen.tests);
        if chosen.dry_run {
            print!(
                "{}",
                headless_dry_run(&chosen, |name| std::env::var_os(name))
            );
            return Ok(());
        }
        let (name, path) = ledger_variable()?;
        println!(
            "{} headless run(s) of the real binary against {}, with cargo test: no time limit",
            tests.len(),
            provider.name
        );
        for test in tests {
            run_ignored(&HEADLESS_SELECTION, test, &[(name, &path)])?;
        }
        return Ok(());
    }
    let provider = match arguments {
        [] => &PROVIDERS[0],
        [name] => live_provider(name)?,
        [_, extra, ..] => {
            return Err(format!(
                "`live` takes one provider at most, and was also given `{extra}`"
            ));
        }
    };
    require_key_file(provider)?;
    let (name, path) = ledger_variable()?;
    ignored_test(provider.smoke, &[(name, &path)])?;
    // The headless run of the binary adds its spend to the same ledger.
    match provider.headless.first() {
        Some(test) => run_ignored(
            &["--package", "cortex-cli", "--test", "system"],
            test,
            &[(name, &path)],
        ),
        None => Ok(()),
    }
}

/// The test binary the headless live tests are in.
const HEADLESS_SELECTION: [&str; 4] = ["--package", "cortex-cli", "--test", "system"];

/// What `live --headless` was asked to run.
#[derive(Debug)]
struct Headless {
    provider: &'static LiveProvider,
    /// The tests, in the provider's order.
    tests: Vec<&'static str>,
    /// Print what would run, and run and send nothing.
    dry_run: bool,
}

/// What `live --headless [PROVIDER] [--test NAME]... [--dry-run]` runs:
/// every headless live test of the provider (Anthropic's when none is
/// named), or those `--test` names, each by its whole name
/// (`live::live_headless_compaction`), its name in the module
/// (`live_headless_compaction`) or the end of it (`compaction`). They run
/// under `cargo test`, not nextest, so a long session is not stopped at a
/// system test's limit. Without the provider's key file it stops before
/// anything is built or sent; a dry run needs none, and says whether it is
/// named.
fn headless_tests(
    arguments: &[String],
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<Headless> {
    let mut names = Vec::new();
    let mut wanted = Vec::new();
    let mut dry_run = false;
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--dry-run" => dry_run = true,
            "--test" => wanted.push(
                words
                    .next()
                    .ok_or("`--test` needs a test's name: `live --headless --test compaction`")?
                    .clone(),
            ),
            option if option.starts_with('-') => {
                return Err(format!(
                    "unknown option `{option}`: `live --headless [PROVIDER] [--test NAME]... \
                     [--dry-run]`"
                ));
            }
            name => names.push(name),
        }
    }
    let provider = match names.as_slice() {
        [] => &PROVIDERS[0],
        [name] => live_provider(name)?,
        [_, extra, ..] => {
            return Err(format!(
                "`live --headless` takes one provider at most, and was also given `{extra}`"
            ));
        }
    };
    if provider.headless.is_empty() {
        let with: Vec<&str> = PROVIDERS
            .iter()
            .filter(|provider| !provider.headless.is_empty())
            .map(|provider| provider.name)
            .collect();
        return Err(format!(
            "there is no headless live test against `{}`: there are against {}",
            provider.name,
            with.join(" and ")
        ));
    }
    let tests = if wanted.is_empty() {
        provider.headless.to_vec()
    } else {
        let mut tests = Vec::new();
        for want in &wanted {
            let found: Vec<&'static str> = provider
                .headless
                .iter()
                .copied()
                .filter(|test| {
                    let own = test.rsplit("::").next().unwrap_or(test);
                    *test == want || own == want || own.ends_with(&format!("_{want}"))
                })
                .collect();
            match found.as_slice() {
                [one] => {
                    if !tests.contains(one) {
                        tests.push(*one);
                    }
                }
                [] => {
                    return Err(format!(
                        "no headless live test against {} is called `{want}`: there are {}. \
                         Nothing was sent.",
                        provider.name,
                        provider.headless.join(", ")
                    ));
                }
                many => {
                    return Err(format!(
                        "`{want}` names {} headless live tests against {}: {}. Name one. \
                         Nothing was sent.",
                        many.len(),
                        provider.name,
                        many.join(", ")
                    ));
                }
            }
        }
        // In the provider's order, whatever order they were named in.
        provider
            .headless
            .iter()
            .copied()
            .filter(|test| tests.contains(test))
            .collect()
    };
    if !dry_run {
        require_key_file_in(provider, var)?;
    }
    Ok(Headless {
        provider,
        tests,
        dry_run,
    })
}

/// What a dry run of `live --headless` prints: each command it would run,
/// and whether the key file's variable is named. It reads no key, builds
/// nothing and sends nothing.
fn headless_dry_run(chosen: &Headless, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> String {
    let variable = format!(
        "CORTEX_LIVE_{}_KEY_FILE",
        chosen.provider.name.to_ascii_uppercase()
    );
    let mut text = format!(
        "dry run: {} headless run(s) of the real binary against {}, nothing built or sent.\n",
        chosen.tests.len(),
        chosen.provider.name
    );
    for test in &chosen.tests {
        text.push_str(&format!(
            "  would run: cargo test --locked {} -- --ignored --exact {test} --nocapture\n",
            HEADLESS_SELECTION.join(" ")
        ));
    }
    text.push_str(&match var(&variable) {
        Some(_) => format!("  {variable} is set: the key would be read from the file it names.\n"),
        None => {
            format!("  {variable} is not set: a real run would stop before anything is sent.\n")
        }
    });
    text
}

/// `cargo xtask record [--provider PROVIDER] [SCENARIO…]`: record fixtures
/// from a real API, Anthropic's when no provider is named. With no scenario
/// named, all of them.
pub(crate) fn record(arguments: &[String]) -> Result<()> {
    let dry_run = arguments.iter().any(|argument| argument == "--dry-run");
    let toolbox = arguments.iter().any(|argument| argument == "--toolbox");
    let arguments: Vec<String> = arguments
        .iter()
        .filter(|argument| *argument != "--dry-run" && *argument != "--toolbox")
        .cloned()
        .collect();
    if toolbox {
        if dry_run {
            // The toolbox scenario is one request each: `--provider P
            // --dry-run toolbox` prints it. A dry run that kept the tools
            // and then recorded would send what it promised not to.
            return Err(
                "`--toolbox` records, and `--dry-run` sends nothing: to see the toolbox request, \
                 run `cargo xtask record --provider PROVIDER --dry-run toolbox`. Nothing was sent."
                    .to_owned(),
            );
        }
        return record_toolbox(&arguments);
    }
    let (provider, scenarios) = record_arguments(&arguments)?;
    let scenarios = scenarios.join(",");
    if dry_run {
        // No key is read and nothing is sent, so none is needed.
        return run_ignored_with(
            &[
                "--package",
                "cortex-llm",
                "--features",
                "http",
                "--test",
                "it",
            ],
            provider.record,
            &[(SCENARIOS, &scenarios), (DRY_RUN, "1")],
            true,
        );
    }
    require_key_file(provider)?;
    let (name, path) = ledger_variable()?;
    ignored_test(provider.record, &[(SCENARIOS, &scenarios), (name, &path)])
}

/// The system test of `cortex-cli` that keeps the tools the binary sends
/// each provider (`docs/testing-and-ci.md`, "The toolbox fixtures").
const TOOLBOX_CAPTURE: &str =
    "tools::sys_the_toolbox_the_binary_sends_is_the_one_its_fixtures_were_recorded_with";

/// `cargo xtask record --toolbox [--provider PROVIDER]`: keeps the tools the
/// binary sends now, and records the toolbox fixture of the provider named,
/// or of every provider whose key file is named in the environment.
fn record_toolbox(arguments: &[String]) -> Result<()> {
    let named = match arguments {
        [] => None,
        [flag, name] if flag == "--provider" => Some(live_provider(name)?),
        _ => return Err("usage: cargo xtask record --toolbox [--provider PROVIDER]".to_owned()),
    };
    let chosen: Vec<&LiveProvider> = match named {
        Some(provider) => {
            require_key_file(provider)?;
            vec![provider]
        }
        None => PROVIDERS
            .iter()
            .filter(|provider| {
                std::env::var_os(provider.key_file).is_some_and(|path| !path.is_empty())
            })
            .collect(),
    };
    if chosen.is_empty() {
        return Err(
            "no provider's key file is named in the environment (CORTEX_LIVE_<PROVIDER>_KEY_FILE), \
             so nothing would be recorded. Nothing was sent."
                .to_owned(),
        );
    }
    // Only the chosen providers' kept tools are written: a provider whose
    // tools changed and that is not recorded keeps the tools of its
    // recording, so its fixture still matches and the change stays a
    // warning, and is not a failure of the blocking tests.
    let names: Vec<&str> = chosen.iter().map(|provider| provider.name).collect();
    println!(
        "Keeping the tools the binary sends {} ...",
        names.join(", ")
    );
    run_test(
        &["--package", "cortex-cli", "--test", "system"],
        TOOLBOX_CAPTURE,
        &[("CORTEX_UPDATE_TOOLBOX", &names.join(","))],
        false,
    )?;
    let (name, path) = ledger_variable()?;
    for provider in &chosen {
        println!("Recording the toolbox fixture of `{}` ...", provider.name);
        ignored_test(provider.record, &[(SCENARIOS, "toolbox"), (name, &path)])?;
    }
    let skipped: Vec<&str> = PROVIDERS
        .iter()
        .filter(|provider| !chosen.iter().any(|chosen| chosen.name == provider.name))
        .map(|provider| provider.name)
        .collect();
    if !skipped.is_empty() {
        println!(
            "Not recorded, for want of a key file: {}. Their fixtures no longer match the kept \
             tools if the tools changed.",
            skipped.join(", ")
        );
    }
    Ok(())
}

/// The provider and the scenarios `record` was asked for.
fn record_arguments(arguments: &[String]) -> Result<(&'static LiveProvider, Vec<&str>)> {
    let mut provider = &PROVIDERS[0];
    let mut scenarios = Vec::new();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        if argument == "--provider" {
            let name = rest
                .next()
                .ok_or("`--provider` needs the provider's name after it")?;
            provider = live_provider(name)?;
        } else if argument.starts_with('-') {
            return Err(format!(
                "`record` takes `--provider NAME` and scenario names, and was given `{argument}`"
            ));
        } else {
            scenarios.push(argument.as_str());
        }
    }
    Ok((provider, scenarios))
}

/// `cargo xtask fuzz [--seconds N] [TARGET…]`: run the fuzz targets, each
/// for N seconds. With no target named, all of them.
pub(crate) fn fuzz(arguments: &[String]) -> Result<()> {
    let mut seconds = FUZZ_SECONDS;
    let mut wanted: Vec<&str> = Vec::new();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--seconds" => {
                seconds = rest
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|seconds| *seconds > 0)
                    .ok_or("`--seconds` needs a number of seconds above zero")?;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("`fuzz` does not know the flag `{flag}`"));
            }
            target => wanted.push(target),
        }
    }
    for target in &wanted {
        if !FUZZ_TARGETS.iter().any(|(name, _)| name == target) {
            let known: Vec<&str> = FUZZ_TARGETS.iter().map(|(name, _)| *name).collect();
            return Err(format!(
                "there is no fuzz target `{target}`. The targets are: {}",
                known.join(", ")
            ));
        }
    }

    let root = workspace::root();
    let package = root.join("fuzz");
    let host = nightly_host()?;
    let mut failed = Vec::new();
    for (target, fixtures) in FUZZ_TARGETS {
        if !wanted.is_empty() && !wanted.contains(target) {
            continue;
        }
        let corpus = package.join("corpus").join(target);
        let seeded = seed(&root.join(fixtures), &corpus)?;
        println!("fuzz {target}: {seconds} s, starting from {seeded} fixture files");
        // `cargo fuzz` builds with the nightly compiler that
        // `fuzz/rust-toolchain.toml` names, whatever toolchain this command
        // itself was started with.
        let status = Command::new("cargo")
            // The target is named. `cargo fuzz` otherwise builds for the
            // target it was itself built for, and a prebuilt `cargo-fuzz`
            // is a static binary for a target the fuzzer cannot use: the
            // sanitizer does not work with a statically linked C library.
            .args(["fuzz", "run", "--target", &host, target, "--"])
            .arg(format!("-max_total_time={seconds}"))
            // A stream event is a few hundred bytes. Inputs up to this size
            // hold a whole response of several blocks.
            .arg("-max_len=8192")
            .arg("-timeout=10")
            .arg("-print_final_stats=1")
            .current_dir(&package)
            // The toolchain this command runs under must not be handed on.
            .env_remove("RUSTUP_TOOLCHAIN")
            .env_remove("CARGO")
            .env_remove("RUSTC")
            .env_remove("RUSTDOC")
            .status()
            .map_err(|error| {
                format!(
                    "could not start `cargo fuzz`: {error}. It needs the nightly \
                     toolchain that fuzz/rust-toolchain.toml names (`rustup toolchain \
                     install`, run inside fuzz/) and cargo-fuzz (`cargo install cargo-fuzz`)."
                )
            })?;
        if !status.success() {
            failed.push(*target);
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the fuzz target(s) {} failed. The input that failed is under fuzz/artifacts/, \
             and `cargo fuzz run <target> <file>`, run inside fuzz/, runs it again.",
            failed.join(", ")
        ))
    }
}

/// The target the nightly compiler builds for when none is named: the
/// machine's own.
fn nightly_host() -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .current_dir(workspace::root().join("fuzz"))
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC")
        .output()
        .map_err(|error| format!("could not run `rustc -vV` in fuzz/: {error}"))?;
    if !output.status.success() {
        return Err(
            "`rustc -vV` failed in fuzz/. The fuzzer needs the nightly toolchain that \
             fuzz/rust-toolchain.toml names: run `rustup toolchain install` inside fuzz/."
                .to_owned(),
        );
    }
    host_of(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "`rustc -vV` did not name a host".to_owned())
}

/// The host target in the output of `rustc -vV`.
fn host_of(version: &str) -> Option<String> {
    version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty())
}

/// Copies the fixtures of a directory into a target's corpus: a response
/// stream (a directory's `response.sse`) with two bytes in front for the
/// cuts the decoders' targets read there, twice; a file of its own, as it
/// is, once. Returns how many fixtures the corpus was given.
fn seed(fixtures: &Path, corpus: &Path) -> Result<usize> {
    fs::create_dir_all(corpus)
        .map_err(|error| format!("cannot make {}: {error}", corpus.display()))?;
    let entries = fs::read_dir(fixtures)
        .map_err(|error| format!("cannot read {}: {error}", fixtures.display()))?;
    let mut seeded = 0;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", fixtures.display()))?;
        let name = entry.file_name();
        // A file of its own (an extractor's sample) is an input as it is,
        // once: its target reads the whole input as a file.
        if entry.path().is_file() {
            let bytes = fs::read(entry.path())
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            let path = corpus.join(format!("fixture-{}", name.to_string_lossy()));
            fs::write(&path, bytes)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            seeded += 1;
            continue;
        }
        let stream = entry.path().join("response.sse");
        let Ok(bytes) = fs::read(&stream) else {
            continue;
        };
        // The same stream at two speeds of cutting: whole, and in small
        // pieces.
        for (seed, label) in [([0u8, 0u8], "whole"), ([0x40, 0x07], "cut")] {
            let mut input = seed.to_vec();
            input.extend_from_slice(&bytes);
            let path = corpus.join(format!("fixture-{}-{label}", name.to_string_lossy()));
            fs::write(&path, input)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        }
        seeded += 1;
    }
    Ok(seeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_command_passes_only_when_its_one_test_ran() {
        let ran = "running 1 test\nsmoke passed.\ntest anthropic::live::live_smoke ... ok\n\n\
                   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 142 filtered out; finished in 3.25s\n";
        assert!(ran_one_test(ran));
        // A test that was renamed: cargo runs nothing and calls it a success.
        let none = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 143 filtered out; finished in 0.00s\n";
        assert!(!ran_one_test(none));
        let failed = "running 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;\n";
        assert!(!ran_one_test(failed));
        assert!(!ran_one_test(""));
    }

    #[test]
    fn a_toolbox_dry_run_is_refused_before_anything_runs() {
        let strings = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        for words in [
            &["--toolbox", "--dry-run"][..],
            &["--dry-run", "--toolbox", "--provider", "deepseek"],
        ] {
            let error = record(&strings(words)).unwrap_err();
            assert!(
                error.contains("`--dry-run` sends nothing"),
                "{words:?}: {error}"
            );
        }
    }

    #[test]
    fn the_ledger_is_totalled_by_provider_and_model() {
        let ledger = r#"{"date":"2026-11-13","provider":"anthropic","model":"claude-haiku","input":100,"cache_read":20,"cache_write":0,"output":30,"usd_nanos":150000,"stated_by_provider":false,"test":"t"}
{"date":"2026-11-14","provider":"anthropic","model":"claude-haiku","input":50,"cache_read":0,"cache_write":0,"output":10,"usd_nanos":50000,"stated_by_provider":false,"test":"t"}
{"date":"2026-11-14","provider":"deepseek","model":"deepseek-chat","input":10,"cache_read":0,"cache_write":0,"output":5,"usd_nanos":7,"stated_by_provider":true,"test":"t"}
not a line of the ledger
"#;
        let all = spent(ledger, None);
        assert!(all.contains("anthropic    claude-haiku"), "{all}");
        assert!(
            all.contains("     2          170         40   0.000200$"),
            "{all}"
        );
        assert!(all.contains("  anthropic: $0.000200\n"), "{all}");
        assert!(all.contains("  deepseek: $0.000000\n"), "{all}");
        assert!(all.contains("  in all: 3 call(s), $0.000200\n"), "{all}");
        assert!(
            all.contains("1 line(s) of the ledger could not be read"),
            "{all}"
        );
        let later = spent(ledger, Some("2026-11-14"));
        assert!(
            later.contains("in all: 2 call(s), $0.000050 since 2026-11-14"),
            "{later}"
        );
        assert!(spent("", None).contains("in all: 0 call(s), $0.000000"));
    }

    #[test]
    fn the_host_is_read_from_the_compilers_own_report() {
        let report = "rustc 1.102.0-nightly (8eb778d04 2026-11-12)\nbinary: rustc\n\
                      host: x86_64-unknown-linux-gnu\nrelease: 1.102.0-nightly\n";
        assert_eq!(host_of(report).as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(host_of("rustc 1.99.1\n"), None);
        assert_eq!(host_of("host: \n"), None);
    }

    #[test]
    fn a_headless_run_without_its_key_says_so_and_stops() {
        let strings = |arguments: &[&str]| -> Vec<String> {
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect()
        };
        // No key file named: an error that names the variable, before
        // anything is built or sent.
        let none = |_: &str| None;
        let error = super::headless_tests(&[], none).unwrap_err();
        assert!(
            error.contains("CORTEX_LIVE_ANTHROPIC_KEY_FILE is not set"),
            "{error}"
        );
        assert!(error.contains("Nothing was sent"), "{error}");
        let error = super::headless_tests(&strings(&["deepseek"]), none).unwrap_err();
        assert!(error.contains("CORTEX_LIVE_DEEPSEEK_KEY_FILE"), "{error}");
        let error = super::headless_tests(&strings(&["venice"]), none).unwrap_err();
        assert!(
            error.contains("no headless live test against `venice`"),
            "{error}"
        );
        assert!(super::headless_tests(&strings(&["a", "b"]), none).is_err());

        // With one, every headless test of the provider.
        let named = |name: &str| {
            (name == "CORTEX_LIVE_ANTHROPIC_KEY_FILE").then(|| std::ffi::OsString::from("k"))
        };
        let chosen = super::headless_tests(&[], named).unwrap();
        assert_eq!(chosen.provider.name, "anthropic");
        assert_eq!(
            chosen.tests,
            [
                "live::live_headless_run",
                "live::live_headless_compaction",
                "live::live_headless_budget_summary"
            ]
        );
        assert!(!chosen.dry_run);
    }

    #[test]
    fn a_headless_test_is_chosen_by_name_and_a_dry_run_sends_nothing() {
        let strings = |arguments: &[&str]| -> Vec<String> {
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect()
        };
        let named = |name: &str| {
            (name == "CORTEX_LIVE_ANTHROPIC_KEY_FILE").then(|| std::ffi::OsString::from("k"))
        };
        let none = |_: &str| None;
        let tests = |arguments: &[&str]| {
            super::headless_tests(&strings(arguments), named).map(|chosen| chosen.tests)
        };
        // By the end of its name, its name in the module, or its whole
        // name; in the provider's order whatever order they were named in.
        assert_eq!(
            tests(&[
                "--test",
                "budget_summary",
                "--test",
                "live_headless_compaction"
            ])
            .unwrap(),
            [
                "live::live_headless_compaction",
                "live::live_headless_budget_summary"
            ]
        );
        assert_eq!(
            tests(&["--test", "live::live_headless_run"]).unwrap(),
            ["live::live_headless_run"]
        );
        // A name that is no test, and one that is two, are refused.
        let error = tests(&["--test", "nothing"]).unwrap_err();
        assert!(
            error.contains("no headless live test against anthropic is called `nothing`"),
            "{error}"
        );
        assert!(error.contains("live::live_headless_compaction"), "{error}");
        let error =
            super::headless_tests(&strings(&["deepseek", "--test", "x"]), none).unwrap_err();
        assert!(error.contains("against deepseek"), "{error}");
        assert!(tests(&["--test"]).is_err());
        assert!(tests(&["--fast"]).is_err());

        // A dry run needs no key file, and prints the command and that the
        // key's variable is not set.
        let chosen =
            super::headless_tests(&strings(&["--dry-run", "--test", "compaction"]), none).unwrap();
        assert!(chosen.dry_run);
        let text = super::headless_dry_run(&chosen, none);
        assert!(
            text.contains(
                "would run: cargo test --locked --package cortex-cli --test system -- --ignored \
                 --exact live::live_headless_compaction --nocapture"
            ),
            "{text}"
        );
        assert!(text.contains("nothing built or sent"), "{text}");
        assert!(
            text.contains("CORTEX_LIVE_ANTHROPIC_KEY_FILE is not set"),
            "{text}"
        );
        assert!(!text.contains("live_headless_run "), "{text}");
        assert!(super::headless_dry_run(&chosen, named).contains("is set"));
    }

    /// Every ignored test of `cortex-cli`'s `tests/system/live.rs` is one
    /// that `live --headless` runs: a new one is not left to be run by hand.
    #[test]
    fn every_headless_live_test_has_a_command() {
        let source = std::fs::read_to_string(
            crate::workspace::root().join("crates/cortex-cli/tests/system/live.rs"),
        )
        .unwrap();
        let mut in_file: Vec<String> = Vec::new();
        let mut ignored = false;
        for line in source.lines() {
            let line = line.trim();
            if line.starts_with("#[ignore") {
                ignored = true;
            } else if let Some(rest) = line.strip_prefix("fn ").filter(|_| ignored) {
                in_file.push(format!("live::{}", rest.split('(').next().unwrap()));
                ignored = false;
            }
        }
        let mut known: Vec<String> = super::PROVIDERS
            .iter()
            .flat_map(|provider| provider.headless.iter().map(|test| (*test).to_owned()))
            .collect();
        in_file.sort();
        known.sort();
        assert_eq!(in_file, known);
    }

    #[test]
    fn the_live_commands_know_their_providers() {
        let strings = |arguments: &[&str]| -> Vec<String> {
            arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect()
        };
        // No provider named: Anthropic, as before this command took one.
        let arguments = strings(&["text", "fast"]);
        let (provider, scenarios) = record_arguments(&arguments).unwrap();
        assert_eq!(provider.name, "anthropic");
        assert_eq!(scenarios, ["text", "fast"]);
        let arguments = strings(&["text", "--provider", "deepseek", "switch"]);
        let (provider, scenarios) = record_arguments(&arguments).unwrap();
        assert_eq!(provider.name, "deepseek");
        assert_eq!(provider.key_file, "CORTEX_LIVE_DEEPSEEK_KEY_FILE");
        assert_eq!(scenarios, ["text", "switch"]);
        for bad in [&["--provider"][..], &["--provider", "openai"], &["--all"]] {
            assert!(record_arguments(&strings(bad)).is_err(), "{bad:?}");
        }
        // Each provider has a key file of its own, and tests of its own.
        for (at, provider) in PROVIDERS.iter().enumerate() {
            for other in &PROVIDERS[at + 1..] {
                assert_ne!(provider.name, other.name);
                assert_ne!(provider.key_file, other.key_file);
                assert_ne!(provider.smoke, other.smoke);
                assert_ne!(provider.record, other.record);
            }
            assert!(provider.key_file.starts_with("CORTEX_LIVE_"));
            assert!(provider.key_file.ends_with("_KEY_FILE"));
        }
        assert!(live(&strings(&["openrouter", "deepseek"])).is_err());
        assert!(live(&strings(&["nobody"])).is_err());
    }

    #[test]
    fn the_fuzz_targets_have_fixtures_to_start_from() {
        let root = workspace::root();
        for (target, fixtures) in FUZZ_TARGETS {
            assert!(
                root.join("fuzz/fuzz_targets")
                    .join(format!("{target}.rs"))
                    .is_file(),
                "{target}"
            );
            assert!(root.join(fixtures).is_dir(), "{fixtures}");
        }
    }
}
