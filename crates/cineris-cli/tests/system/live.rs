//! Runs of the whole harness against a real model (tier 5): the real
//! binary, the provider's real endpoint, a one-line task with one tool
//! call, and `--output json`. They cost money, so they are ignored, and only
//! `cargo xtask live` runs them, by hand and never in CI: Anthropic's on
//! `anthropic-messages`, and DeepSeek's on `openai-chat`, each with the
//! whole built-in toolbox.
//!
//! The API key is read from the file that `CINERIS_LIVE_<PROVIDER>_KEY_FILE`
//! names and handed to the binary's environment, as a user's would be. It
//! is never printed: the test fails if it shows up in either stream.
//!
//! **What each run spent goes in the spend ledger** that `cargo xtask live
//! --spent` totals, when `CINERIS_LIVE_LEDGER` names it (`cargo xtask live`
//! sets it): one line a run, from the `--output json` record's tokens and
//! cost (`docs/testing-and-ci.md`, "Fixtures").
//!
//! `live_headless_compaction` is step 15's run by hand: a session that
//! compacts in the background and carries on, with what each context epoch
//! read from the cache.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Machine, Run, shown};

#[test]
#[ignore = "talks to Anthropic's API and costs money: `cargo xtask live` runs it"]
fn live_headless_run() {
    // The cheapest Anthropic model in the built-in catalog.
    headless(
        "CINERIS_LIVE_ANTHROPIC_KEY_FILE",
        "ANTHROPIC_API_KEY",
        "model.main = \"claude-haiku-5-5\"\nmodel.effort = \"low\"\n",
    );
}

#[test]
#[ignore = "talks to DeepSeek's API and costs money: `cargo xtask live deepseek` runs it"]
fn live_headless_run_deepseek() {
    headless(
        "CINERIS_LIVE_DEEPSEEK_KEY_FILE",
        "DEEPSEEK_API_KEY",
        "model.main = \"deepseek-flash\"\n",
    );
}

/// The API key in the file `key_file` names.
fn key_from(key_file: &str) -> String {
    let file = std::env::var_os(key_file)
        .unwrap_or_else(|| panic!("{key_file} names the file the API key is in"));
    let key = fs::read_to_string(file).expect("the key file can be read");
    let key = key.trim().to_owned();
    assert!(!key.is_empty(), "the key file holds a key");
    key
}

/// Adds a run's spend to the ledger, when there is one: the record's
/// tokens and cost, under the provider and model it ran on.
fn ledger(record: &Value, provider: &str, model: &str, key: &str, test: &str) {
    let Some(path) = std::env::var_os("CINERIS_LIVE_LEDGER").filter(|path| !path.is_empty()) else {
        return;
    };
    let usage = &record["usage"];
    let costs = record["costs"].as_array().cloned().unwrap_or_default();
    let usd_nanos: u64 = costs
        .iter()
        .filter_map(|cost| cost["usd_nanos"].as_u64())
        .max()
        .unwrap_or(0);
    let stated = costs.iter().any(|cost| cost["basis"] == "recorded");
    let date = cineris_base::UtcTime::from_system(std::time::SystemTime::now())
        .map(|now| now.to_string().chars().take(10).collect::<String>())
        .unwrap_or_default();
    let line = serde_json::json!({
        "date": date,
        "provider": provider,
        "model": model,
        "input": usage["input"].as_u64().unwrap_or(0),
        "cache_read": usage["cache_read"].as_u64().unwrap_or(0),
        "cache_write": usage["cache_write"].as_u64().unwrap_or(0),
        "output": usage["output"].as_u64().unwrap_or(0),
        "usd_nanos": usd_nanos,
        "stated_by_provider": stated,
        "test": test,
    })
    .to_string();
    assert!(!line.contains(key), "the key would have gone in the ledger");
    #[expect(
        clippy::disallowed_types,
        reason = "a line added to a file by a test run by hand"
    )]
    let opened = fs::OpenOptions::new().create(true).append(true).open(&path);
    let written = opened.and_then(|mut file| {
        use std::io::Write as _;
        // One write, so that two runs adding at once do not mix lines.
        file.write_all(format!("{line}\n").as_bytes())
    });
    if let Err(error) = written {
        eprintln!("(the spend could not be added to the ledger: {error})");
    }
}

fn headless(key_file: &str, variable: &str, model: &str) {
    let key = key_from(key_file);
    let key = key.as_str();

    let machine = Machine::new("live_headless_run");
    // The real endpoint: no base URL of the test's own.
    machine.raw_user_config(&format!(
        "session.durability = \"never\"\ncredentials.store = \"file\"\n{model}"
    ));
    fs::write(machine.workspace.join("answer.txt"), "forty-two\n").unwrap();

    let output = machine
        .command(&[
            "-p",
            "Read the file answer.txt with the read tool, then reply with its content and \
             nothing else.",
            "--output",
            "json",
            "--max-requests",
            "4",
        ])
        .env(variable, key)
        .output()
        .unwrap();
    let run = Run::of(output);
    assert!(
        !run.stdout.contains(key) && !run.stderr.contains(key),
        "the key was printed"
    );
    eprintln!("cineris -p … --output json wrote:\n{}", run.stdout);
    eprintln!("and on standard error:\n{}", run.stderr);
    let record = run.expect_code(0).json();
    let (provider, model_id) = if variable == "ANTHROPIC_API_KEY" {
        ("anthropic", "claude-haiku-5-5")
    } else {
        ("deepseek", "deepseek-flash")
    };
    ledger(&record, provider, model_id, key, "live_headless_run");
    assert_eq!(record["outcome"], "completed", "{}", shown(&run));
    assert_eq!(record["tool_calls"], 1, "{}", shown(&run));
    assert_eq!(
        record["files_changed"],
        serde_json::json!([]),
        "a read changes nothing"
    );
    assert!(
        record["result"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("forty-two"),
        "{}",
        shown(&run)
    );
}

/// The cheapest Anthropic model in the catalog that caches, and its window
/// less what the loop keeps back: room for the answer (32,768) and a
/// margin (a fiftieth).
const COMPACTION_MODEL: &str = "claude-haiku-5-5";
const COMPACTION_LIMIT: f64 = 1_000_000.0 - 32_768.0 - 20_000.0;

/// What one request was counted as: input, cache read, cache write, output.
type Counted = (u64, u64, u64, u64);

/// The session logs under a directory.
fn logs_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(logs_under(&path));
        } else if path.file_name().is_some_and(|name| name == "log.jsonl") {
            found.push(path);
        }
    }
    found
}

/// One headless run of the compaction session's machine.
fn compaction_run(machine: &Machine, key: &str, prompt: &str, requests: &str) -> Value {
    run_on(
        machine,
        key,
        prompt,
        requests,
        COMPACTION_MODEL,
        "live_headless_compaction",
    )
}

/// One headless run of a machine, on `model`, its spend in the ledger as
/// `test`'s.
fn run_on(
    machine: &Machine,
    key: &str,
    prompt: &str,
    requests: &str,
    model: &str,
    test: &str,
) -> Value {
    let output = machine
        .command(&["-p", prompt, "--output", "json", "--max-requests", requests])
        .env("ANTHROPIC_API_KEY", key)
        .output()
        .unwrap();
    let run = Run::of(output);
    assert!(
        !run.stdout.contains(key) && !run.stderr.contains(key),
        "the key was printed"
    );
    eprintln!("cineris -p … --output json wrote:\n{}", run.stdout);
    eprintln!("and on standard error:\n{}", run.stderr);
    let record = run.expect_code(0).json();
    ledger(&record, "anthropic", model, key, test);
    record
}

#[test]
#[ignore = "talks to Anthropic's API and costs money: run by hand, `docs/steps/15/answered.md`"]
fn live_headless_compaction() {
    let key = key_from("CINERIS_LIVE_ANTHROPIC_KEY_FILE");
    let machine = Machine::new("live_headless_compaction");
    let config = |extra: &str| {
        machine.raw_user_config(&format!(
            "session.durability = \"never\"\ncredentials.store = \"file\"\nmodel.main = \
             \"{COMPACTION_MODEL}\"\nmodel.effort = \"low\"\n{extra}"
        ));
    };
    // What the prompt and the tools come to, asked of the model itself.
    config("");
    let measured = compaction_run(&machine, &key, "Reply with the word ready.", "2");
    let usage = &measured["usage"];
    let base = ["input", "cache_read", "cache_write"]
        .iter()
        .map(|class| usage[class].as_u64().unwrap_or(0))
        .sum::<u64>();
    eprintln!("the prompt and the tools: {base} tokens");

    // Six files of about 2,500 tokens each. The thresholds are set so that
    // the summary is prepared in the background after the second, and the
    // epoch is compacted after the fourth.
    for name in ["a", "b", "c", "d", "e", "f"] {
        let lines: String = (1..=300)
            .map(|line| format!("{name}{line:03}: the quick brown fox jumps over the lazy dog\n"))
            .collect();
        fs::write(machine.workspace.join(format!("{name}.txt")), lines).unwrap();
    }
    #[expect(clippy::cast_precision_loss, reason = "a fraction of a limit")]
    let fraction = |tokens: u64| (base + tokens) as f64 / COMPACTION_LIMIT;
    config(&format!(
        "context.prepare_at = {:.6}\ncontext.compact_at = {:.6}\n",
        fraction(4_500),
        fraction(9_500)
    ));
    let record = compaction_run(
        &machine,
        &key,
        "Read the files a.txt, b.txt, c.txt, d.txt, e.txt and f.txt with the read tool, one \
         call at a time and in that order. Then reply with the first line of each file, and \
         nothing else.",
        "14",
    );
    assert_eq!(record["outcome"], "completed", "{record}");

    // The session's log: its epochs, and what each request of each read
    // from the cache.
    let logs = logs_under(&machine.home);
    let log = logs
        .iter()
        .max_by_key(|path| fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .expect("the session has a log");
    let lines: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let mut epochs: Vec<(String, Vec<Counted>)> = Vec::new();
    let mut summaries = 0;
    let mut out_of_band = Vec::new();
    for line in &lines {
        match line["kind"].as_str() {
            Some("epoch") => epochs.push((
                line["reason"].as_str().unwrap_or("?").to_owned(),
                Vec::new(),
            )),
            Some("summary") => summaries += 1,
            Some("assistant") => {
                let tokens = &line["item"]["usage"]["tokens"];
                let count = |class: &str| tokens[class].as_u64().unwrap_or(0);
                if let Some((_, requests)) = epochs.last_mut() {
                    requests.push((
                        count("input"),
                        count("cache_read"),
                        count("cache_write"),
                        count("output"),
                    ));
                }
            }
            Some("usage") => out_of_band.push(line["usage"]["tokens"].clone()),
            _ => {}
        }
    }
    eprintln!("epochs, and each request's input, cache read, cache write, output:");
    for (index, (reason, requests)) in epochs.iter().enumerate() {
        let read: u64 = requests.iter().map(|request| request.1).sum();
        eprintln!(
            "  epoch {} ({reason}): {} requests, {read} tokens read from the cache: {requests:?}",
            index + 1,
            requests.len()
        );
    }
    eprintln!("summary requests: {out_of_band:?}");
    eprintln!("the run's record: {}", record["usage"]);
    eprintln!("its cost: {}", record["costs"]);
    assert!(summaries >= 1, "the session compacted");
    assert!(epochs.iter().any(|(reason, _)| reason == "compaction"));
    for (reason, requests) in &epochs {
        if !requests.is_empty() {
            let read: u64 = requests.iter().map(|request| request.1).sum();
            assert!(
                read > 0,
                "the epoch ({reason}) read from the cache: {requests:?}"
            );
        }
    }
}

/// The cheapest Anthropic model whose reasoning is a token budget, which
/// no entry of the built-in catalog is: a user's catalog entry describes
/// it, with the shape the review of step 15 found every summary refused on
/// (a budget of 1,024 to 64,000 tokens, an output of 64,000).
const BUDGET_MODEL: &str = "claude-haiku-4-5";
const BUDGET_LIMIT: f64 = 200_000.0 - 32_768.0 - 4_000.0;
const BUDGET_ENTRY: &str = "[models.haiku45]
provider = \"anthropic\"
id = \"claude-haiku-4-5\"
name = \"Claude Haiku 4.5\"
protocol = \"anthropic-messages\"
context_window = 200_000
max_output = 64_000
input = [\"text\"]

[models.haiku45.prices]
input = 1.00
output = 5.00
cache_read = 0.10
cache_write = [{ class = \"5m\", price = 1.25 }]

[models.haiku45.reasoning]
min_tokens = 1_024
max_tokens = 64_000
";

/// Finding 2 of the review of step 15, against the real API: on a model
/// whose reasoning is a budget of 16,384 tokens, one compaction, whose
/// summary request asks for its output on top of the budget and is
/// accepted. Run by hand (`cargo xtask live --headless`), with the
/// owner's allowance for it (`docs/steps/15/answered.md`).
#[test]
#[ignore = "talks to Anthropic's API and costs money: run by hand, `docs/steps/15/answered.md`"]
fn live_headless_budget_summary() {
    let key = key_from("CINERIS_LIVE_ANTHROPIC_KEY_FILE");
    let machine = Machine::new("live_headless_budget_summary");
    let test = "live_headless_budget_summary";
    let config = |extra: &str| {
        machine.raw_user_config(&format!(
            "session.durability = \"never\"\ncredentials.store = \"file\"\nmodel.main = \
             \"{BUDGET_MODEL}\"\nmodel.effort = \"16384\"\ncontext.background = false\n\
             {extra}\n{BUDGET_ENTRY}"
        ));
    };
    // What the prompt and the tools come to, asked of the model itself.
    config("");
    let measured = run_on(
        &machine,
        &key,
        "Reply with the word ready.",
        "2",
        BUDGET_MODEL,
        test,
    );
    let usage = &measured["usage"];
    let base = ["input", "cache_read", "cache_write"]
        .iter()
        .map(|class| usage[class].as_u64().unwrap_or(0))
        .sum::<u64>();
    eprintln!("the prompt and the tools: {base} tokens");
    // Four files of about 2,500 tokens each; the trigger is past the third.
    for name in ["a", "b", "c", "d"] {
        let lines: String = (1..=300)
            .map(|line| format!("{name}{line:03}: the quick brown fox jumps over the lazy dog\n"))
            .collect();
        fs::write(machine.workspace.join(format!("{name}.txt")), lines).unwrap();
    }
    #[expect(clippy::cast_precision_loss, reason = "a fraction of a limit")]
    let compact_at = (base + 9_500) as f64 / BUDGET_LIMIT;
    config(&format!("context.compact_at = {compact_at:.6}\n"));
    let record = run_on(
        &machine,
        &key,
        "Read the files a.txt, b.txt, c.txt and d.txt with the read tool, one call at a time \
         and in that order. Then reply with the first line of each file, and nothing else.",
        "10",
        BUDGET_MODEL,
        test,
    );
    assert_eq!(record["outcome"], "completed", "{record}");
    let logs = logs_under(&machine.home);
    let log = logs
        .iter()
        .max_by_key(|path| fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .expect("the session has a log");
    let lines: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let summaries = lines
        .iter()
        .filter(|line| line["kind"] == "summary")
        .count();
    let compactions = lines
        .iter()
        .filter(|line| line["kind"] == "epoch" && line["reason"] == "compaction")
        .count();
    let summary_usage: Vec<&Value> = lines
        .iter()
        .filter(|line| line["kind"] == "usage")
        .map(|line| &line["usage"]["tokens"])
        .collect();
    eprintln!("summary requests' usage: {summary_usage:?}");
    eprintln!("the run's record: {}", record["usage"]);
    eprintln!("its cost: {}", record["costs"]);
    assert!(summaries >= 1, "the session compacted");
    assert_eq!(compactions, summaries, "each summary began an epoch");
}
