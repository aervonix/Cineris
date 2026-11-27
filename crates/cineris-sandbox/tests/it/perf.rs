//! What a look at protected paths costs on a real workspace: a
//! measurement, run by hand in a release build, outside nextest.
//!
//! ```text
//! CINERIS_PERF_DIR=<a workspace> cargo test --release -p cineris-sandbox --test it perf_ -- --ignored --nocapture
//! ```
//!
//! Without `CINERIS_PERF_DIR` it looks at this repository.
//!
//! And what a command's process tree costs, from its start to its end, and
//! on Windows what its interrupt takes: the signed approvals' part
//! "commands without the harness's console" gave each command a console of
//! its own, whose host starts with it, and sends the interrupt from inside
//! it. `cargo xtask perf -p cineris-sandbox` runs them all, and `try`'s
//! `perf -p cineris-sandbox` with `against` compares two trees on a runner.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use cineris_sandbox::protect::{Limits, Watch};
use cineris_sandbox::tree::Tree;

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_a_look_at_the_protected_paths_of_a_workspace() {
    let workspace = std::env::var_os("CINERIS_PERF_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    );
    let fixed: Vec<PathBuf> = [
        ".git/hooks",
        ".git/config",
        ".git/info",
        ".cineris",
        ".github/workflows",
    ]
    .iter()
    .map(|name| workspace.join(name))
    .collect();
    for (name, limits) in [
        ("the standard bounds", Limits::default()),
        (
            "no bound",
            Limits {
                directories: usize::MAX,
                time: std::time::Duration::from_secs(600),
                ..Limits::default()
            },
        ),
    ] {
        let watch = Watch::new(&workspace, fixed.clone()).with_limits(limits);
        let _ = watch.take();
        let mut times: Vec<f64> = (0..10)
            .map(|_| {
                let start = Instant::now();
                let _ = watch.take();
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let snapshot = watch.take();
        println!(
            "{name:<20} median {:>8.1} ms   {} entries   complete: {}",
            times[times.len() / 2],
            snapshot.len(),
            snapshot.is_complete()
        );
    }
}

/// The median of a set of times, in milliseconds.
fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// One command that does nothing, started as a tree, waited for and ended.
#[test]
#[ignore = "a measurement, run in a release build: see the module documentation"]
fn perf_a_tree_starts_and_ends_one_command() {
    let times: Vec<f64> = (0..41)
        .map(|_| {
            let mut command = if cfg!(windows) {
                let mut command = Command::new("cmd.exe");
                command.args(["/d", "/c", "exit 0"]);
                command
            } else {
                let mut command = Command::new("/bin/sh");
                command.args(["-c", "true"]);
                command
            };
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let start = Instant::now();
            let tree = Tree::spawn(command, None).expect("start the command");
            assert!(tree.wait().expect("wait").success());
            tree.end();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let least = times.iter().copied().fold(f64::INFINITY, f64::min);
    let median = median(times);
    println!("a tree of one command, start to end: median {median:.2} ms, least {least:.2} ms");
    cineris_testkit::numbers::report("tree.start_to_end", median, "ms");
}

/// From the interrupt to the command's being gone, for a command that
/// listens: on Windows the helper's start, its attach to the command's
/// console, and the event.
#[test]
#[ignore = "a measurement, run in a release build: see the module documentation"]
fn perf_an_interrupt_stops_a_command_that_listens() {
    use cineris_testkit::children::{ChildSpec, Roll};
    let times: Vec<f64> = (0..15)
        .map(|_| {
            let roll = Roll::new();
            let command = ChildSpec::new(&roll, "only").command(crate::ENTRY);
            let tree = Tree::spawn(command, Some(&crate::guard())).expect("start the command");
            let only = roll.next(crate::LIMIT);
            let start = Instant::now();
            tree.interrupt().expect("interrupt the command");
            assert!(only.gone_within(crate::LIMIT));
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            tree.end();
            elapsed
        })
        .collect();
    let least = times.iter().copied().fold(f64::INFINITY, f64::min);
    let median = median(times);
    println!("an interrupt to the command's end: median {median:.2} ms, least {least:.2} ms");
    cineris_testkit::numbers::report("tree.interrupt_to_gone", median, "ms");
}
