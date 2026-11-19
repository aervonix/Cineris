//! What a test was doing when it ran out of time (`docs/testing-and-ci.md`,
//! "When a test runs out of time").
//!
//! nextest stops a test at its tier's limit and says only that it timed
//! out. A test that hangs on a gate, a socket or a child has nothing to say
//! for itself by then: it is killed. So the first time a test process uses
//! one of the test kit's waits (a [`Gate`](crate::Gate), the scripted
//! server, a [`Roll`](crate::children::Roll), [`until`](crate::children::until))
//! this module starts one thread that does nothing until the test has used
//! four fifths of its limit. If the test is still running then, the thread
//! writes a report to the test's standard error, which nextest keeps and
//! shows with the failure, in the log and in the JUnit report:
//!
//! - **what each waiting thread is waiting for:** which gate (by where it
//!   was made), which connection of the scripted server and which step of
//!   its script, which child of a roll;
//! - **the process tree under the test,** each process with its state and
//!   its command line; on Linux each thread's name, state and the kernel
//!   function it sleeps in (its wait channel), and its kernel stack where
//!   the system lets a process read it (as root);
//! - on Windows, the tree, each process's command line and each thread's
//!   state and wait reason, asked of PowerShell. Thread stacks are not read
//!   there.
//!
//! A test that passes never reaches the report. What it costs is one
//! thread that waits, and a lock taken when a wait starts and ends.
//!
//! The thread runs only under nextest (`NEXTEST` is set) and never in a
//! child that a test started from its own binary. It reads the limit from
//! the profile nextest names and the test's name; a test of the watch
//! itself shortens it with `CINERIS_WATCH_LIMIT_MS`.
//!
//! # Widening a race on purpose
//!
//! `CINERIS_TESTKIT_PAUSE_MS=<n>` makes the scripted server pause `n`
//! milliseconds before each write and before opening each gate of its
//! script. A race between what the server writes and what the test does
//! next, which a loaded machine loses once in a thousand runs, then loses
//! every time, so that a fix can be shown to hold. It is off unless set,
//! and nothing else reads it.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test support: it reads /proc and asks PowerShell, on a thread of its own, only when a test is about to be stopped"
)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// How much of its limit a test uses before the report is written.
const SHARE: f64 = 0.8;

/// What one thread is doing, for as long as the handle lives.
#[derive(Debug)]
pub struct Doing {
    id: u64,
    /// The same wait in the time profile, when it is on: none when it is
    /// off, so that a wait takes no further lock.
    mark: Option<Mutex<crate::profile::Mark>>,
}

struct Entry {
    id: u64,
    thread: String,
    since: Instant,
    what: String,
}

fn entries() -> &'static Mutex<Vec<Entry>> {
    static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    &ENTRIES
}

/// Says what the calling thread is doing until the handle is dropped, and
/// starts the watch if it has not started. For a test's own long wait that
/// none of the test kit's waits covers.
#[must_use]
pub fn doing(what: impl Into<String>) -> Doing {
    doing_as(what.into(), true)
}

/// What [`doing`] does for the timeout report alone, with no span in the
/// time profile: for a wait that is not real time spent, such as a fake
/// timer's, which can stay pending for the whole test without taking any
/// of it.
#[must_use]
pub(crate) fn naming(what: impl Into<String>) -> Doing {
    doing_as(what.into(), false)
}

fn doing_as(what: String, profiled: bool) -> Doing {
    arm();
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let thread = std::thread::current();
    let mark = (profiled && crate::profile::enabled())
        .then(|| Mutex::new(crate::profile::wait(|| what.clone())));
    let entry = Entry {
        id,
        thread: thread.name().unwrap_or("an unnamed thread").to_owned(),
        since: Instant::now(),
        what,
    };
    lock().push(entry);
    Doing { id, mark }
}

impl Doing {
    /// Says what the thread is doing now.
    pub fn now(&self, what: impl Into<String>) {
        let what = what.into();
        if let Some(mark) = &self.mark {
            mark.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next(|| what.clone());
        }
        if let Some(entry) = lock().iter_mut().find(|entry| entry.id == self.id) {
            entry.what = what;
            entry.since = Instant::now();
        }
    }
}

impl Drop for Doing {
    fn drop(&mut self) {
        lock().retain(|entry| entry.id != self.id);
    }
}

fn lock() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    entries().lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the watch for this test process, once. Does nothing outside
/// nextest, in a child a test started from its own binary, or for a test
/// whose limit is not known.
pub fn arm() {
    static ARMED: OnceLock<()> = OnceLock::new();
    ARMED.get_or_init(|| {
        let Some((limit, why)) = limit() else { return };
        let age = process_age().unwrap_or_default();
        let wait = report_at(limit).saturating_sub(age);
        let started = Instant::now();
        // If the thread cannot be started, the test runs without a watch.
        let _ = std::thread::Builder::new()
            .name("cineris-testkit-watch".to_owned())
            .spawn(move || {
                pause(wait);
                // The waits first, which cost nothing to gather, and only
                // then the tree, which on Windows asks PowerShell and can
                // take longer than the test has left.
                let (head, tail) = report(started.elapsed() + age, limit, &why);
                let mut error = std::io::stderr().lock();
                let _ = error.write_all(head.as_bytes());
                let _ = error.flush();
                let _ = error.write_all(tail().as_bytes());
                let _ = error.flush();
            });
    });
}

/// How far into a test's limit the report is written.
fn report_at(limit: Duration) -> Duration {
    limit.mul_f64(SHARE)
}

/// The test's limit and where it came from, or none when the watch should
/// not run.
fn limit() -> Option<(Duration, String)> {
    std::env::var_os("NEXTEST")?;
    // A child that a test started from its own binary: its parent watches.
    if std::env::args().any(|argument| argument.starts_with("cin-child=") || argument == "cin-echo")
    {
        return None;
    }
    if let Some(milliseconds) = std::env::var("CINERIS_WATCH_LIMIT_MS")
        .ok()
        .and_then(|text| text.parse().ok())
    {
        return Some((
            Duration::from_millis(milliseconds),
            "CINERIS_WATCH_LIMIT_MS".to_owned(),
        ));
    }
    let profile = std::env::var("NEXTEST_PROFILE").unwrap_or_default();
    let test = std::env::var("NEXTEST_TEST_NAME")
        .ok()
        .or_else(|| std::thread::current().name().map(str::to_owned))
        .unwrap_or_default();
    let seconds = limit_of(&profile, &test);
    Some((
        Duration::from_secs(seconds),
        format!("nextest's `{profile}` profile, for `{test}`"),
    ))
}

/// The limit in seconds nextest gives a test in a profile, as
/// `.config/nextest.toml` sets it. A test checks the two agree.
fn limit_of(profile: &str, test: &str) -> u64 {
    let system = test.starts_with("sys_") || test.contains("::sys_");
    match (profile, system) {
        ("properties", false) => 300,
        ("perf", _) => 1800,
        (_, true) => 60,
        (_, false) => 10,
    }
}

/// How long ago this process started, where the system says.
pub(crate) fn process_age() -> Option<Duration> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The fields after the command's name, which is in brackets and may
    // hold spaces; the start time is the 22nd field, the 20th after it.
    let after = &stat[stat.rfind(')')? + 2..];
    let ticks: f64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    // Linux counts a process's start in ticks of 1/100 s on every
    // platform this project builds for.
    Some(Duration::from_secs_f64((uptime - ticks / 100.0).max(0.0)))
}

/// Waits for `length` without sleeping a fixed time: parked, and woken
/// early only by chance, so it parks again for what is left.
fn pause(length: Duration) {
    let end = Instant::now() + length;
    loop {
        let now = Instant::now();
        if now >= end {
            return;
        }
        std::thread::park_timeout(end - now);
    }
}

/// The pause `CINERIS_TESTKIT_PAUSE_MS` asks the scripted server for, if
/// any.
pub(crate) fn race_pause() {
    static PAUSE: OnceLock<Option<Duration>> = OnceLock::new();
    let pause_for = PAUSE.get_or_init(|| {
        std::env::var("CINERIS_TESTKIT_PAUSE_MS")
            .ok()
            .and_then(|text| text.parse().ok())
            .map(Duration::from_millis)
    });
    if let Some(length) = pause_for {
        pause(*length);
    }
}

/// The report: what the test's threads wait for, and then, gathered only
/// when it is asked for, its process tree.
fn report(ran: Duration, limit: Duration, why: &str) -> (String, impl FnOnce() -> String) {
    let mut text = format!(
        "\n== cineris-testkit: this test has run {:.1} s of the {:.0} s it is given ({why}). \
         What it is doing:\n",
        ran.as_secs_f64(),
        limit.as_secs_f64()
    );
    text.push_str(&waits());
    let tree = || {
        let children = cineris_base::hook::spawned_children();
        let pids: Vec<u32> = children.iter().map(|child| child.pid).collect();
        let (tree, alive) = tree(std::process::id(), &pids);
        let mut tail = children_text(&children, |pid| there(pid, alive.as_deref()));
        tail.push_str(&tree);
        tail.push_str("== end of cineris-testkit's report\n");
        tail
    };
    (text, tree)
}

/// Whether a child is still there when the report is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum There {
    Running,
    /// Exited, and not yet waited for: a zombie, on Linux.
    Unreaped,
    Gone,
    /// The system was not asked.
    Unknown,
}

/// Where a child is: from `/proc` on Linux, and on Windows from the ids
/// PowerShell found among the system's processes (`alive`).
fn there(pid: u32, alive: Option<&[u32]>) -> There {
    if cfg!(target_os = "linux") {
        linux::there(pid)
    } else if let Some(alive) = alive {
        if alive.contains(&pid) {
            There::Running
        } else {
            There::Gone
        }
    } else {
        There::Unknown
    }
}

/// The test's own children (since the twelfth tooling step): each process
/// the harness or the test kit started for it (`cineris_base::hook::spawned`),
/// with its id, how long ago it started, whether the code that started it
/// saw it exit and whether it is still there, and its command line. A child
/// that is gone while the wait of the code that started it has not
/// returned, beside a thread still waiting, is a wake that was missed; one
/// still running is a late exit. Only a start marked as tracked
/// (`spawned_tracked`: the sandbox's process tree, the runner's commands)
/// marks its wait; of the others the report says only that they are gone.
/// A child is found by its id alone, which the system may give a new
/// process once the child is gone (soon, on Windows).
fn children_text(children: &[cineris_base::hook::Spawned], there: impl Fn(u32) -> There) -> String {
    let mut text = String::from(
        "the test's own children, oldest first: pid, how long ago it started, what became of \
         it, its command line\n",
    );
    if children.is_empty() {
        text.push_str("  none was started through the harness or the test kit\n");
    }
    for child in children {
        let state = match (child.reaped_after, there(child.pid), child.tracked) {
            (Some(after), _, _) => format!(
                "exited after {:.1} s, and its wait returned",
                after.as_secs_f64()
            ),
            (None, There::Running, _) => "still running".to_owned(),
            (None, There::Unreaped, _) => {
                "exited, and nothing has waited for it yet (a zombie)".to_owned()
            }
            (None, There::Gone, true) => {
                "gone, and the wait of the code that started it has not returned: a wake missed, \
                 or a wait not reached yet"
                    .to_owned()
            }
            (None, There::Gone, false) => {
                "gone (the code that started it does not mark its wait, so whether its exit was \
                 seen is not known)"
                    .to_owned()
            }
            (None, There::Unknown, _) => "not asked of the system".to_owned(),
        };
        let _ = writeln!(
            text,
            "  {} started {:.1} s ago, {state}: {}",
            child.pid,
            child.ago.as_secs_f64(),
            child.command
        );
    }
    text
}

/// The whole report at once, for a test.
#[cfg(test)]
fn report_text(ran: Duration, limit: Duration, why: &str) -> String {
    let (head, tail) = report(ran, limit, why);
    head + &tail()
}

/// What each thread said it is doing, longest first.
pub(crate) fn waits() -> String {
    let entries = lock();
    let mut lines: Vec<(Duration, String)> = entries
        .iter()
        .map(|entry| {
            let since = entry.since.elapsed();
            (
                since,
                format!(
                    "  {:>6.1} s  thread `{}`: {}\n",
                    since.as_secs_f64(),
                    entry.thread,
                    entry.what
                ),
            )
        })
        .collect();
    drop(entries);
    // The library's own threads that a test holds at a point: they wait
    // inside code that knows nothing of the test kit.
    lines.extend(cineris_base::hook::stopped().into_iter().map(|held| {
        (
            held.for_how_long,
            format!(
                "  {:>6.1} s  thread `{}`: stopped at the library point `{}`, held by a test until it lets the thread go\n",
                held.for_how_long.as_secs_f64(),
                held.thread,
                held.point
            ),
        )
    }));
    lines.sort_by_key(|line| std::cmp::Reverse(line.0));
    let mut text = String::from("waits, and for how long:\n");
    if lines.is_empty() {
        text.push_str("  none of the test kit's waits is in progress\n");
    }
    for (_, line) in lines {
        text.push_str(&line);
    }
    text
}

/// The process tree under `root`, and on Windows which of `pids` the
/// system still has (asked of the same PowerShell).
fn tree(root: u32, pids: &[u32]) -> (String, Option<Vec<u32>>) {
    if cfg!(target_os = "linux") {
        (linux::tree(root), None)
    } else if cfg!(windows) {
        windows_tree(root, pids)
    } else {
        ("processes: not read on this OS\n".to_owned(), None)
    }
}

mod linux {
    //! The process tree from `/proc`.

    use std::fmt::Write as _;

    /// A process's parent, state and name, from `/proc/<pid>/stat`.
    fn stat(path: &str) -> Option<(u32, char, String)> {
        let stat = std::fs::read_to_string(path).ok()?;
        let open = stat.find('(')?;
        let close = stat.rfind(')')?;
        let name = stat[open + 1..close].to_owned();
        let mut rest = stat[close + 2..].split_whitespace();
        let state = rest.next()?.chars().next()?;
        let parent = rest.next()?.parse().ok()?;
        Some((parent, state, name))
    }

    /// Whether process `pid` is running, has exited and not been waited
    /// for (a zombie), or is gone.
    pub(super) fn there(pid: u32) -> super::There {
        match stat(&format!("/proc/{pid}/stat")) {
            Some((_, 'Z' | 'X', _)) => super::There::Unreaped,
            Some(_) => super::There::Running,
            None => super::There::Gone,
        }
    }

    pub(super) fn tree(root: u32) -> String {
        let mut parents = Vec::new();
        if let Ok(directory) = std::fs::read_dir("/proc") {
            for entry in directory.flatten() {
                let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<u32>().ok())
                else {
                    continue;
                };
                if let Some((parent, _, _)) = stat(&format!("/proc/{pid}/stat")) {
                    parents.push((pid, parent));
                }
            }
        }
        let mut text = String::from(
            "processes: pid, state, command line; then each thread's name, state and wait channel\n",
        );
        let mut stack = vec![(root, 0usize)];
        while let Some((pid, depth)) = stack.pop() {
            let indent = "  ".repeat(depth + 1);
            let (_, state, name) =
                stat(&format!("/proc/{pid}/stat")).unwrap_or((0, '?', String::new()));
            let command = std::fs::read(format!("/proc/{pid}/cmdline"))
                .map(|bytes| {
                    let line = String::from_utf8_lossy(&bytes).replace('\0', " ");
                    let line = line.trim_end().to_owned();
                    if line.is_empty() {
                        format!("[{name}]")
                    } else {
                        line
                    }
                })
                .unwrap_or_else(|_| format!("[{name}]"));
            let command: String = command.chars().take(400).collect();
            let _ = writeln!(text, "{indent}{pid} {state} {command}");
            if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
                let mut tids: Vec<u32> = tasks
                    .flatten()
                    .filter_map(|task| task.file_name().to_str().and_then(|name| name.parse().ok()))
                    .collect();
                tids.sort_unstable();
                for tid in tids {
                    let base = format!("/proc/{pid}/task/{tid}");
                    let (_, state, name) =
                        stat(&format!("{base}/stat")).unwrap_or((0, '?', String::new()));
                    let channel =
                        std::fs::read_to_string(format!("{base}/wchan")).unwrap_or_default();
                    let channel = if channel.is_empty() || channel == "0" {
                        "-"
                    } else {
                        channel.trim()
                    };
                    let _ = writeln!(text, "{indent}  thread {tid} `{name}` {state} in {channel}");
                    // The kernel stack is readable as root only; elsewhere
                    // this is empty and costs one failed open.
                    if let Ok(stack) = std::fs::read_to_string(format!("{base}/stack")) {
                        for frame in stack.lines().take(8) {
                            let _ = writeln!(text, "{indent}      {frame}");
                        }
                    }
                }
            }
            let mut children: Vec<u32> = parents
                .iter()
                .filter(|(_, parent)| *parent == pid)
                .map(|(child, _)| *child)
                .collect();
            children.sort_unstable_by(|a, b| b.cmp(a));
            stack.extend(children.into_iter().map(|child| (child, depth + 1)));
        }
        text
    }
}

/// The process tree under `root` on Windows, from PowerShell: each
/// process's id and command line, and each of its threads' state and, while
/// it waits, its wait reason. Thread stacks are not read.
///
/// The processes come from one query of `Win32_Process`, and their threads
/// from `Get-Process`, which reads one snapshot of the system. A query of
/// `Win32_Thread` for each process walks every thread of the system each
/// time: on a cold nightly runner the tree of a test and one child took
/// longer than the 30 s the watch's own test waited for it (nightly run
/// 43675736382).
fn windows_tree(root: u32, pids: &[u32]) -> (String, Option<Vec<u32>>) {
    let ids: Vec<String> = pids.iter().map(u32::to_string).collect();
    let ids = ids.join(",");
    let script = format!(
        "$all = Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,CommandLine; \
         $todo = [System.Collections.Generic.Queue[object]]::new(); $todo.Enqueue(@({root}, 1)); \
         while ($todo.Count -gt 0) {{ $item = $todo.Dequeue(); $id = $item[0]; $depth = $item[1]; \
           $p = $all | Where-Object {{ $_.ProcessId -eq $id }}; \
           $line = if ($p) {{ $p.CommandLine }} else {{ '?' }}; \
           Write-Output ('  ' * $depth + \"$id $line\"); \
           $process = Get-Process -Id $id -ErrorAction SilentlyContinue; \
           if ($process) {{ $process.Threads | ForEach-Object {{ \
             $reason = if ($_.ThreadState -eq 'Wait') {{ $_.WaitReason }} else {{ '-' }}; \
             Write-Output ('  ' * $depth + \"  thread $($_.Id) state $($_.ThreadState) wait reason $reason\") }} }}; \
           $all | Where-Object {{ $_.ParentProcessId -eq $id -and $_.ProcessId -ne $id }} | ForEach-Object {{ $todo.Enqueue(@($_.ProcessId, ($depth + 1))) }} }}; \
         $known = @($all | ForEach-Object {{ $_.ProcessId }}); \
         Write-Output ('{ALIVE} ' + ((@({ids}) | Where-Object {{ $known -contains $_ }}) -join ' '))"
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();
    let mut text = String::from(
        "processes: pid and command line; then each thread's state and, while it waits, \
         its wait reason. Thread stacks are not read on Windows.\n",
    );
    // None unless PowerShell answered which are there: no answer is not
    // an answer that none is.
    let mut alive = None;
    match output {
        Ok(output) => {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some(ids) = line.strip_prefix(ALIVE) {
                    alive = Some(
                        ids.split_whitespace()
                            .filter_map(|id| id.parse::<u32>().ok())
                            .collect(),
                    );
                } else {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
        Err(error) => {
            let _ = writeln!(text, "  PowerShell could not be asked: {error}");
        }
    }
    (text, alive)
}

/// What the line of the children's ids that the system still has starts
/// with, in PowerShell's answer.
const ALIVE: &str = "cineris-alive:";

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{doing, limit_of, process_age, report_at, report_text as report};

    #[test]
    fn the_report_comes_before_the_limit() {
        assert_eq!(report_at(Duration::from_secs(10)), Duration::from_secs(8));
        assert_eq!(report_at(Duration::from_secs(60)), Duration::from_secs(48));
    }

    #[test]
    fn the_age_of_this_process_is_read_on_linux() {
        let age = process_age();
        if cfg!(target_os = "linux") {
            // This test's process started a moment ago, not when the
            // machine did.
            let age = age.expect("Linux says when a process started");
            assert!(age < Duration::from_secs(120), "{age:?}");
        } else {
            assert_eq!(age, None);
        }
    }
    use crate::{Gate, ServerReply, SseServer};

    #[test]
    fn the_limits_are_those_nextest_is_given() {
        assert_eq!(limit_of("ci", "rules::tests::a_rule"), 10);
        assert_eq!(limit_of("default", "shell::sys_quoting"), 60);
        assert_eq!(limit_of("ci", "sys_version"), 60);
        assert_eq!(limit_of("properties", "cases::a_property"), 300);
        assert_eq!(limit_of("perf", "perf::perf_a_tree"), 1800);
        // The same numbers as the nextest configuration's.
        let configuration = include_str!("../../../.config/nextest.toml");
        for line in [
            r#"slow-timeout = { period = "2s", terminate-after = 5 }"#,
            r#"slow-timeout = { period = "10s", terminate-after = 6 }"#,
            r#"slow-timeout = { period = "60s", terminate-after = 5 }"#,
            r#"slow-timeout = { period = "60s", terminate-after = 30 }"#,
        ] {
            assert!(
                configuration.lines().any(|each| each.trim() == line),
                "the nextest configuration no longer says {line}: change limit_of with it"
            );
        }
    }

    /// Each child with its id, its age, what became of it and its command:
    /// an exit its wait saw, still running, a zombie, gone unseen.
    #[test]
    fn each_child_is_told_running_exited_or_gone() {
        use cineris_base::hook::Spawned;
        let child = |pid, reaped: Option<u64>, tracked| Spawned {
            pid,
            command: format!("program {pid}"),
            ago: Duration::from_millis(7_900),
            reaped_after: reaped.map(Duration::from_millis),
            tracked,
        };
        let children = [
            child(1, Some(300), true),
            child(2, None, true),
            child(3, None, false),
            child(4, None, true),
            child(5, None, false),
        ];
        let text = super::children_text(&children, |pid| match pid {
            2 => super::There::Running,
            3 => super::There::Unreaped,
            _ => super::There::Gone,
        });
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6, "{text}");
        assert_eq!(
            lines[1],
            "  1 started 7.9 s ago, exited after 0.3 s, and its wait returned: program 1"
        );
        assert!(lines[2].contains("still running") && lines[2].ends_with("program 2"));
        assert!(lines[3].contains("a zombie"), "{text}");
        assert!(lines[4].contains("a wake missed"), "{text}");
        assert!(
            lines[5].contains("gone (the code that started it does not mark its wait"),
            "{text}"
        );
        let none = super::children_text(&[], |_| super::There::Unknown);
        assert!(none.contains("none was started"), "{none}");
    }

    #[test]
    fn a_wait_is_reported_while_it_lasts() {
        // The waits alone: the process tree is asked of PowerShell on
        // Windows, which costs a second a time, and is tested by the
        // system test below.
        let waits =
            |text: &str| super::report(Duration::from_secs(8), Duration::from_secs(10), text).0;
        let waiting = doing("waiting for something made up for this test");
        let text = waits("a test");
        assert!(text.contains("has run 8.0 s of the 10 s"), "{text}");
        assert!(
            text.contains("waiting for something made up for this test"),
            "{text}"
        );
        waiting.now("now doing something else");
        let text = waits("a test");
        assert!(text.contains("now doing something else"), "{text}");
        assert!(!text.contains("made up for this test"), "{text}");
        drop(waiting);
        let text = waits("a test");
        assert!(!text.contains("something else"), "{text}");
        // This process is in its own tree, read from /proc on Linux.
        if cfg!(target_os = "linux") {
            let whole = report(Duration::from_secs(8), Duration::from_secs(10), "a test");
            assert!(
                whole.contains(&format!(" {} ", std::process::id())),
                "{whole}"
            );
        }
    }

    /// The test that hangs on purpose: a gate nobody opens, a scripted
    /// server holding a response, and a child process that stays. It does
    /// nothing unless the test below starts it with `CINERIS_WATCH_HANG`.
    #[test]
    #[ignore = "run by sys_a_test_that_runs_out_of_time_says_what_it_was_doing"]
    fn hangs_on_purpose() {
        if std::env::var_os("CINERIS_WATCH_HANG").is_none() {
            return;
        }
        let server = SseServer::start();
        let never = Gate::new();
        server.push(ServerReply::sse().hold_until(&never).send("data: x\n\n"));
        let mut connection =
            TcpStream::connect(server.url("").trim_start_matches("http://")).unwrap();
        connection
            .write_all(b"POST /v1/held HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        // A child that stays until it is killed: this binary, waiting on
        // its standard input, which is never closed.
        let program = std::env::current_exe().unwrap();
        let mut command = Command::new(&program);
        command
            .args(["--exact", "watch::tests::stays", "--ignored"])
            .env("CINERIS_WATCH_STAY", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let described = cineris_base::hook::describe(&command);
        let mut child = command.spawn().unwrap();
        cineris_base::hook::spawned(child.id(), described);
        // And one that is gone before the report, its exit seen by nobody
        // the hooks hear from: the report says so.
        let mut quick = Command::new(&program);
        quick
            .arg("--list")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let described = cineris_base::hook::describe(&quick);
        let mut quick = quick.spawn().unwrap();
        cineris_base::hook::spawned(quick.id(), described);
        let _ = quick.wait();
        let stuck = Gate::new();
        let _ = stuck.wait(Duration::from_secs(600));
        // Not reached: the test that started this one kills it first.
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A scripted exchange of four writes (the head, two pieces, the end),
    /// which prints how long it took. It does nothing unless the test below
    /// starts it with `CINERIS_TESTKIT_PAUSE_MS`.
    #[test]
    #[ignore = "run by sys_the_race_pause_holds_back_each_write"]
    fn paused_exchange() {
        if std::env::var_os("CINERIS_TESTKIT_PAUSE_MS").is_none() {
            return;
        }
        let server = SseServer::start();
        server.push(ServerReply::sse().send("data: a\n\n").send("data: b\n\n"));
        let started = std::time::Instant::now();
        let mut connection =
            TcpStream::connect(server.url("").trim_start_matches("http://")).unwrap();
        connection
            .write_all(b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        std::io::Read::read_to_string(&mut connection, &mut response).unwrap();
        assert!(response.ends_with("0\r\n\r\n"), "{response}");
        println!("elapsed-ms: {}", started.elapsed().as_millis());
    }

    #[test]
    #[ignore = "waits on purpose; the nightly workflow runs it"]
    fn sys_the_race_pause_holds_back_each_write() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "watch::tests::paused_exchange",
                "--ignored",
                "--nocapture",
            ])
            .env("CINERIS_TESTKIT_PAUSE_MS", "100")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}");
        let elapsed: u64 = stdout
            .lines()
            .find_map(|line| line.strip_prefix("elapsed-ms: "))
            .and_then(|number| number.trim().parse().ok())
            .unwrap_or_else(|| panic!("{stdout}"));
        // Four writes, each held back 100 ms.
        assert!(elapsed >= 400, "the exchange took {elapsed} ms: {stdout}");
    }

    /// The child of `hangs_on_purpose`.
    #[test]
    #[ignore = "a child of hangs_on_purpose"]
    fn stays() {
        if std::env::var_os("CINERIS_WATCH_STAY").is_none() {
            return;
        }
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    /// Kills a process and everything under it.
    fn kill_tree(child: &mut std::process::Child) {
        let pid = child.id().to_string();
        if cfg!(windows) {
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid])
                .output();
        } else {
            // The child leads a process group of its own.
            let _ = Command::new("kill")
                .args(["-KILL", &format!("-{pid}")])
                .output();
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Ignored in the blocking checks, where it would cost a second of the
    /// system tier on each OS (more on Windows, where the report asks
    /// PowerShell). The nightly workflow runs it on both OSes, and so does
    /// anyone who changes this module:
    /// `cargo xtask test -p cineris-testkit --run-ignored only watch::tests::sys_`.
    #[test]
    #[ignore = "starts a test that hangs on purpose; the nightly workflow runs it"]
    fn sys_a_test_that_runs_out_of_time_says_what_it_was_doing() {
        let program = std::env::current_exe().unwrap();
        let mut command = Command::new(program);
        command
            .args([
                "--exact",
                "watch::tests::hangs_on_purpose",
                "--ignored",
                "--nocapture",
            ])
            .env("NEXTEST", "1")
            .env("NEXTEST_PROFILE", "ci")
            .env("NEXTEST_TEST_NAME", "watch::tests::hangs_on_purpose")
            .env("CINERIS_WATCH_LIMIT_MS", "1000")
            .env("CINERIS_WATCH_HANG", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (lines, received) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let end = line.starts_with("== end of cineris-testkit's report");
                if lines.send(line).is_err() || end {
                    break;
                }
            }
        });
        let mut text = String::new();
        let (mut head_at, mut ended) = (None, None);
        // A bound on a mistake: the report comes after 0.8 s.
        while let Ok(line) = received.recv_timeout(Duration::from_secs(30)) {
            text.push_str(&line);
            text.push('\n');
            if line.contains("this test has run ") {
                head_at = Some(std::time::Instant::now());
            }
            if line.starts_with("== end of cineris-testkit's report") {
                ended = head_at.map(|at| at.elapsed());
                break;
            }
        }
        kill_tree(&mut child);
        // How long the tree took after the waits, which on Windows is
        // PowerShell's time: seen with `--no-capture`.
        eprintln!("the tree came {ended:?} after the waits");
        assert!(
            ended.is_some(),
            "the report's tree did not come within 30 s of its waits: {text}"
        );

        assert!(
            text.contains("of the 1 s it is given (CINERIS_WATCH_LIMIT_MS)"),
            "{text}"
        );
        // Before the limit, at four fifths of it: counted from the
        // process's start on Linux, from the first wait elsewhere.
        let ran: f64 = text
            .split("this test has run ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|number| number.parse().ok())
            .unwrap_or_else(|| panic!("{text}"));
        assert!(
            (0.75..1.0).contains(&ran),
            "the report came at {ran} s: {text}"
        );
        // The test's own wait, by where the gate was made.
        assert!(
            text.contains("waits for the gate made at crates/cineris-testkit/src/watch.rs:")
                || text
                    .contains("waits for the gate made at crates\\cineris-testkit\\src\\watch.rs:"),
            "{text}"
        );
        // The server's connection, and what it holds for.
        assert!(
            text.contains(
                "connection 1, POST /v1/held: holding the response until the gate made at"
            ),
            "{text}"
        );
        // The test's own children: the one that stays, still running, and
        // the quick one, gone with no wait the hooks heard from.
        assert!(text.contains("the test's own children"), "{text}");
        assert!(
            text.lines()
                .any(|line| line.contains("still running") && line.contains("stays")),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|line| line.contains("gone (the code") && line.contains("--list")),
            "{text}"
        );
        // The tree: the test and its child.
        assert!(text.contains("hangs_on_purpose"), "{text}");
        assert!(text.contains("watch::tests::stays"), "{text}");
        if cfg!(target_os = "linux") {
            assert!(text.contains(" in "), "each thread's wait channel: {text}");
        }
        if cfg!(windows) {
            assert!(text.contains("wait reason"), "{text}");
        }
    }
}
