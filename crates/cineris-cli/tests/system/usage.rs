//! `cineris usage` and `cineris stats` as the binary runs them (step 16): they
//! read the local files and connect to nothing, and the month's report
//! meets its budget.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cineris_base::UtcTime;
use serde_json::Value;

use crate::Machine;

/// Today's date in UTC, `YYYY-MM-DD`: the binary is run with `TZ=UTC`.
fn today() -> String {
    UtcTime::from_system(std::time::SystemTime::now())
        .unwrap()
        .to_string()
        .chars()
        .take(10)
        .collect()
}

fn stats_dir(machine: &Machine) -> PathBuf {
    machine.dirs().state.unwrap().join("stats")
}

/// One request event of `day`, as the writer stores it.
fn request_line(day: &str, n: usize) -> String {
    format!(
        "{{\"day\":\"{day}\",\"kind\":\"request\",\"at\":\"{day}T12:00:00Z\",\"session\":\"s_{n}\",\
         \"turn\":\"t_1\",\"project\":\"pk_{p}\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5-5\",\
         \"role\":\"main\",\"variants\":{{\"edit_format\":\"replace\",\"tool_schema\":\"strict\",\
         \"prompt_style\":\"concise\",\"effort\":\"high\"}},\"tokens\":{{\"input\":1200,\"cache_read\":\
         40000,\"cache_write\":900,\"output\":600,\"reasoning\":200}},\"cost\":{{\"usd_nanos\":47000000,\
         \"basis\":\"metered\"}},\"catalog_version\":\"2026-11-13\",\"billing\":\"metered\",\"ttft_ms\":900,\
         \"tokens_per_second\":61.5,\"time\":{{\"connect_ms\":120,\"first_byte_ms\":780,\"streaming_ms\":\
         9800}},\"stop\":\"tool_calls\"}}\n",
        p = n % 3
    )
}

fn tool_line(day: &str, n: usize) -> String {
    format!(
        "{{\"day\":\"{day}\",\"kind\":\"tool\",\"at\":\"{day}T12:00:01Z\",\"session\":\"s_{n}\",\
         \"turn\":\"t_1\",\"project\":\"pk_{p}\",\"tool\":\"{tool}\",\"model\":\"claude-opus-5-5\",\
         \"variants\":{{\"edit_format\":\"replace\",\"tool_schema\":\"strict\",\"prompt_style\":\
         \"concise\"}},\"outcome\":\"ok\",\"running_ms\":350,\"queued_ms\":4,\"approval_ms\":0}}\n",
        p = n % 3,
        tool = ["read", "search", "shell", "edit"][n % 4]
    )
}

fn turn_line(day: &str, n: usize) -> String {
    format!(
        "{{\"day\":\"{day}\",\"kind\":\"turn\",\"at\":\"{day}T12:05:00Z\",\"session\":\"s_{n}\",\
         \"turn\":\"t_1\",\"project\":\"pk_{p}\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5-5\",\
         \"variants\":{{\"edit_format\":\"replace\",\"tool_schema\":\"strict\",\"prompt_style\":\
         \"concise\"}},\"requests\":15,\"tool_calls\":22,\"outcome\":\"completed\",\"wall_ms\":300000,\
         \"model_time\":{{\"connect_ms\":2000,\"first_byte_ms\":12000,\"streaming_ms\":150000}},\
         \"tools_ms\":120000,\"approval_ms\":10000,\"harness_ms\":6000,\"by_tool\":[{{\"tool\":\"shell\",\
         \"ms\":100000}},{{\"tool\":\"read\",\"ms\":20000}}]}}\n",
        p = n % 3
    )
}

/// Writes `days` days of events before today into this month's file
/// (today among them), `requests` requests a day with a tool call and a
/// half for each, and a turn for every fifteen requests.
fn history(machine: &Machine, days: usize, requests: usize) -> u64 {
    let dir = stats_dir(machine);
    fs::create_dir_all(&dir).unwrap();
    let today = today();
    let month = &today[..7];
    let day_of_month: usize = today[8..10].parse().unwrap();
    let mut text = String::new();
    for day in 1..=days.min(day_of_month) {
        let date = format!("{month}-{day:02}");
        for n in 0..requests {
            text.push_str(&request_line(&date, n));
            text.push_str(&tool_line(&date, n));
            if n % 2 == 0 {
                text.push_str(&tool_line(&date, n + 1));
            }
            if n % 15 == 0 {
                text.push_str(&turn_line(&date, n));
            }
        }
    }
    let path = dir.join(format!("events-{month}.jsonl"));
    fs::write(&path, &text).unwrap();
    text.len() as u64
}

#[test]
fn sys_cineris_usage_reads_the_local_files_and_connects_to_nothing() {
    let machine = Machine::new("usage_month");
    history(&machine, 1, 3);
    let output = machine
        .command(&["usage", "--range", "month", "--json"])
        .env("TZ", "UTC")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["totals"]["requests"], 3, "{report}");
    assert_eq!(report["totals"]["costs"][0]["usd_nanos"], 141_000_000);
    assert_eq!(report["tools"][0]["tool"], "shell");
    // The text form says where the time went.
    let text = machine
        .command(&["usage", "--by", "project"])
        .env("TZ", "UTC")
        .output()
        .unwrap();
    let shown = String::from_utf8_lossy(&text.stdout);
    assert!(shown.contains("project pk_0"), "{shown}");
    assert!(shown.contains("Time "), "{shown}");
    // `--refresh` with nothing listed asks nobody.
    let refresh = machine
        .command(&["usage", "--refresh"])
        .env("TZ", "UTC")
        .output()
        .unwrap();
    assert!(refresh.status.success());
    assert!(String::from_utf8_lossy(&refresh.stderr).contains("nothing to refresh"));
    let stats = machine.command(&["stats"]).output().unwrap();
    let said = String::from_utf8_lossy(&stats.stdout);
    assert!(said.contains("Stored: 3 requests"), "{said}");
    assert!(
        machine.server.requests().is_empty(),
        "nothing connected to the provider"
    );
}

/// The month's report of a heavy user's month: `cineris usage --range month`
/// against its budget (the headless start-up budget plus 5 ms). Run by
/// hand: `cargo test --release -p cineris-cli --test system perf_usage_month
/// -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_usage_month() {
    let machine = Machine::new("perf_usage_month");
    // A heavy user: 3,000 requests a working day, 20 days.
    let bytes = history(&machine, 20, 3000);
    let run = || {
        let started = Instant::now();
        let output = machine
            .command(&["usage", "--range", "month"])
            .env("TZ", "UTC")
            .output()
            .unwrap();
        assert!(output.status.success());
        started.elapsed()
    };
    // The first run reads the month's file into the aggregates; the writer
    // keeps them current every few seconds, so this is the rare case.
    let first = run();
    let mut runs: Vec<Duration> = (0..30).map(|_| run()).collect();
    runs.sort();
    let version = || {
        let started = Instant::now();
        let output = machine.command(&["--version"]).output().unwrap();
        assert!(output.status.success());
        started.elapsed()
    };
    let mut versions: Vec<Duration> = (0..30).map(|_| version()).collect();
    versions.sort();
    let aggregates = fs::metadata(stats_dir(&machine).join("aggregates.json"))
        .unwrap()
        .len();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "a month of {bytes} bytes of events; aggregates {aggregates} bytes"
    );
    let _ = writeln!(out, "first run, reading the month: {first:?}");
    let _ = writeln!(
        out,
        "cineris usage --range month: median {:?}, 95th {:?}",
        runs[runs.len() / 2],
        runs[runs.len() * 95 / 100]
    );
    let _ = writeln!(
        out,
        "cineris --version: median {:?}",
        versions[versions.len() / 2]
    );
    println!("{out}");
}
