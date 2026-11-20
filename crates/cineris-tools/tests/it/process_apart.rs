//! A command run with its standard output and its standard error apart
//! (`Runner::run_apart`, for an extension's `cineris.exec`): each stream
//! whole and on its own side, each cut at the limit with its true size
//! counted, and the run ended by its time limit, by a cancellation and by a
//! process left holding one stream, as a run of the one pipe is
//! (`docs/architecture/05-tools.md`, "`shell`", "Output").
//!
//! The commands are this test binary run again
//! (`cineris_testkit::children`), started directly with no shell. The time
//! limit and the graces are waits on the test's timer, which end when the
//! test ends them.

use std::time::Duration;

use cineris_testkit::children::{ChildSpec, Roll, Stream, stream_bytes, streams_arguments, until};
use cineris_tools::process::{
    CommandLine, Ending, Exit, FinishedApart, KeptStream, Launch, Start, Stop,
};

use crate::bench::{self, Director, GRACE, LINGER, TestTimer, directed_call};
use crate::{ENTRY, LIMIT, block_on};

/// A time limit no test reaches unless it ends the wait itself.
const TIME_LIMIT: Duration = Duration::from_secs(60);

/// What a test keeps of each stream when it is not about the limit: more
/// than any of these commands writes.
const ROOMY: usize = 1 << 20;

/// What starts `program` with `arguments`, directly, with the test's own
/// environment: a child connects to the test over the loopback interface,
/// which Windows' sockets cannot do in an empty one.
fn start(program: &str, arguments: Vec<String>) -> Start {
    start_line(CommandLine::new(program).args(arguments))
}

/// What starts `command`, as [`start`] does.
fn start_line(command: CommandLine) -> Start {
    Start {
        launch: Launch {
            command,
            stdin: None,
        },
        cwd: std::env::current_dir().expect("the test has a directory"),
        env: std::env::vars_os().collect(),
        timeout: TIME_LIMIT,
        label: "apart".to_owned(),
    }
}

/// Runs `start` with its streams apart, keeping `limit` bytes of each, on
/// this thread, while `direct` runs on another.
fn run_apart<T: Send + 'static>(
    start: Start,
    limit: usize,
    direct: impl FnOnce(&Director) -> T + Send + 'static,
) -> (FinishedApart, T) {
    let timer = TestTimer::default();
    let runner = bench::runner(&timer);
    directed_call(
        &timer,
        |cancel, kill| {
            block_on(runner.run_apart(start, cancel, kill, limit)).expect("the command starts")
        },
        direct,
    )
}

/// Runs a command that must end by itself. If it has not within [`LIMIT`],
/// the director fails, which lets every wait go and cancels the call.
fn run_alone(start: Start, limit: usize) -> FinishedApart {
    run_apart(start, limit, |director| {
        until(LIMIT, "the command to end by itself", || {
            director.call_has_ended()
        });
    })
    .0
}

/// The bytes a command of `streams_arguments` wrote to its standard output,
/// out of what was kept of it: the test runner's own lines come first, and
/// they are a few bytes with nothing of the standard error in them.
fn after_the_runners_lines(kept: &KeptStream, written: usize) -> &[u8] {
    let before = usize::try_from(kept.total_bytes)
        .expect("a size")
        .checked_sub(written)
        .expect("the stream holds what the command wrote to it");
    assert!(
        before < 256,
        "the test runner wrote {before} bytes of its own before the command's"
    );
    assert!(
        !contains(&kept.bytes[..before], b"err "),
        "the standard error is in the standard output"
    );
    &kept.bytes[before..]
}

fn contains(bytes: &[u8], word: &[u8]) -> bool {
    bytes.windows(word.len()).any(|window| window == word)
}

/// Each stream comes through whole and on its own side. Each is more than
/// a pipe holds on either system, and the command writes all of its
/// standard error before any of its standard output: a runner that read
/// one stream to its end before the other would wait for ever.
#[test]
fn sys_standard_output_and_standard_error_are_kept_apart_and_whole() {
    let (out_size, err_size) = (300_000, 200_000);
    let (program, arguments) = streams_arguments(ENTRY, out_size, err_size);
    let finished = run_alone(start(&program, arguments), ROOMY);

    assert_eq!(finished.exit, Some(Exit::Code(0)));
    assert_eq!(finished.ending, Ending::Exited);
    assert_eq!(finished.stopped_with, None);
    assert!(!finished.left_running && !finished.outlived && !finished.escaped);

    // The standard error holds what the command wrote there and nothing
    // else.
    assert!(!finished.stderr.is_cut());
    assert_eq!(finished.stderr.total_bytes, err_size as u64);
    assert!(
        finished.stderr.bytes == stream_bytes(Stream::Stderr, err_size),
        "the standard error is not what the command wrote to it"
    );
    // The standard output too, after the test runner's lines.
    assert!(!finished.stdout.is_cut());
    assert!(
        after_the_runners_lines(&finished.stdout, out_size)
            == stream_bytes(Stream::Stdout, out_size),
        "the standard output is not what the command wrote to it"
    );
}

/// Past the limit nothing more of a stream is kept, and all of it is
/// counted: the first bytes are what is kept.
#[test]
fn sys_each_stream_is_cut_at_the_limit_and_its_size_counted() {
    let (out_size, err_size) = (150_000, 90_000);
    let limit = 4096;
    let (program, arguments) = streams_arguments(ENTRY, out_size, err_size);
    let finished = run_alone(start(&program, arguments), limit);
    assert_eq!(finished.exit, Some(Exit::Code(0)));

    assert!(finished.stderr.is_cut());
    assert_eq!(finished.stderr.total_bytes, err_size as u64);
    assert_eq!(finished.stderr.bytes, stream_bytes(Stream::Stderr, limit));

    assert!(finished.stdout.is_cut());
    assert_eq!(finished.stdout.bytes.len(), limit);
    let before = usize::try_from(finished.stdout.total_bytes).expect("a size") - out_size;
    assert!(before < 256, "{before} bytes before the command's");
    assert_eq!(
        finished.stdout.bytes[before..],
        stream_bytes(Stream::Stdout, limit - before)
    );
}

/// A command that starts a child, which starts a grandchild that the
/// polite interrupt does not stop.
fn stubborn_family(roll: &Roll) -> (String, Vec<String>) {
    ChildSpec::new(roll, "parent")
        .with_child(ChildSpec::new(roll, "grandchild").stubborn())
        .arguments(ENTRY)
}

#[test]
fn sys_a_time_limit_stops_a_command_run_apart() {
    let roll = Roll::new();
    let (program, arguments) = stubborn_family(&roll);
    let (finished, ()) = run_apart(start(&program, arguments), ROOMY, move |director| {
        let family = roll.all(&["parent", "grandchild"], LIMIT);
        until(LIMIT, "the call's time limit to be armed", || {
            director.timer.running().contains(&TIME_LIMIT)
        });
        assert_eq!(director.timer.end(TIME_LIMIT), 1);
        // The grandchild sits out the polite stages, and each grace is
        // ended as it starts, until the kill has taken everything.
        director.ending_each(GRACE, "the tree to be gone", || {
            !family[0].is_alive() && !family[1].is_alive()
        });
    });
    assert_eq!(finished.ending, Ending::TimedOut);
    assert_eq!(finished.stopped_with, Some(Stop::Kill));
    assert!(!finished.left_running);
}

#[test]
fn sys_cancelling_a_command_run_apart_stops_its_whole_tree() {
    let roll = Roll::new();
    let (program, arguments) = stubborn_family(&roll);
    let (finished, ()) = run_apart(start(&program, arguments), ROOMY, move |director| {
        let family = roll.all(&["parent", "grandchild"], LIMIT);
        director.cancel.cancel();
        director.ending_each(GRACE, "the tree to be gone", || {
            !family[0].is_alive() && !family[1].is_alive()
        });
    });
    assert_eq!(finished.ending, Ending::Interrupted);
    assert_eq!(finished.stopped_with, Some(Stop::Kill));
}

/// "Kill now" alone: the tree is killed with no polite stage and no grace.
#[test]
fn sys_kill_now_kills_a_command_run_apart_without_the_graces() {
    let roll = Roll::new();
    let (program, arguments) = stubborn_family(&roll);
    let (finished, graces) = run_apart(start(&program, arguments), ROOMY, move |director| {
        let family = roll.all(&["parent", "grandchild"], LIMIT);
        director.kill.cancel();
        assert!(
            family[0].gone_within(LIMIT),
            "the command outlived the kill"
        );
        assert!(
            family[1].gone_within(LIMIT),
            "the grandchild outlived the kill"
        );
        until(LIMIT, "the call to end", || director.call_has_ended());
        director
            .timer
            .started()
            .iter()
            .filter(|wait| **wait == GRACE)
            .count()
    });
    assert_eq!(graces, 0, "a grace was started");
    assert_eq!(finished.ending, Ending::Interrupted);
    assert_eq!(finished.stopped_with, Some(Stop::Kill));
}

/// A command that exits and leaves a process holding its standard error:
/// its standard output ends with it, and the run waits out the grace for
/// what it left running, kills the process, and ends with both readers
/// done. On Windows the process may hold the standard output as well (a
/// child inherits every handle its parent may pass on), and the run ends
/// the same way.
#[test]
fn sys_a_process_left_holding_standard_error_is_stopped_and_the_run_ends() {
    let roll = Roll::new();
    let (program, arguments) = ChildSpec::new(&roll, "parent")
        .saying("parent-out")
        .complaining("parent-err")
        .exiting(0)
        .with_child(ChildSpec::new(&roll, "holder").without_stdout())
        .arguments(ENTRY);
    let (finished, ()) = run_apart(start(&program, arguments), ROOMY, move |director| {
        let family = roll.all(&["parent", "holder"], LIMIT);
        // The parent says its words and exits; the holder stays.
        assert!(family[0].gone_within(LIMIT), "the command did not exit");
        assert!(family[1].is_alive());
        // The grace starts when the command itself has exited.
        director.timer.end_when_running(LINGER);
        assert!(family[1].gone_within(LIMIT), "it was left running");
        until(LIMIT, "the call to end", || director.call_has_ended());
    });
    assert_eq!(finished.exit, Some(Exit::Code(0)));
    assert_eq!(finished.ending, Ending::Exited);
    assert_eq!(finished.stopped_with, None);
    assert!(finished.left_running, "{finished:?}");
    assert!(!finished.outlived, "{finished:?}");
    // What the command wrote before it exited is there, each word on its
    // own side.
    assert!(contains(&finished.stdout.bytes, b"parent-out"));
    assert!(!contains(&finished.stdout.bytes, b"parent-err"));
    assert!(contains(&finished.stderr.bytes, b"parent-err"));
    assert!(!contains(&finished.stderr.bytes, b"parent-out"));
}

/// A process that left the command's session holds the standard error
/// after the command exits: nothing can kill it, so the runner stops
/// listening a moment after the tree is gone. Both readers are let go then,
/// and the run ends by itself, without a second moment: the reader of the
/// standard error was stopped. Only Linux has a way out of the tree; on
/// Windows nothing leaves a Job Object, and a stopped read of either stream
/// is the process tree's test.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_process_that_left_the_tree_holding_standard_error_has_its_reader_stopped() {
    if !std::path::Path::new("/usr/bin/setsid").exists() {
        crate::skipped("there is no setsid program here");
        return;
    }
    let roll = Roll::new();
    let (program, arguments) = ChildSpec::new(&roll, "escaped").arguments(ENTRY);
    // The shell starts the child in a session of its own, with its standard
    // output thrown away, says a word on each stream and exits.
    let command = CommandLine::new("/bin/sh")
        .arg("-c")
        .arg("/usr/bin/setsid \"$0\" \"$@\" >/dev/null & echo shell-out; echo shell-err >&2")
        .arg(program)
        .args(arguments);
    let (finished, escaped) = run_apart(start_line(command), ROOMY, move |director| {
        let escaped = roll.next(LIMIT);
        // The command exits and nothing of its group is left: the grace,
        // then the moment the runner gives the standard error to end.
        director.timer.end_when_running(LINGER);
        director
            .timer
            .end_when_running(cineris_tools::process::SETTLE);
        // Not the second moment: the readers end the run.
        until(LIMIT, "the run to end with its readers stopped", || {
            director.call_has_ended()
        });
        escaped
    });
    assert_eq!(finished.exit, Some(Exit::Code(0)));
    assert!(finished.outlived, "{finished:?}");
    assert!(!finished.left_running, "{finished:?}");
    assert_eq!(finished.stdout.bytes, b"shell-out\n");
    assert_eq!(finished.stderr.bytes, b"shell-err\n");

    // It is still running, and the test ends it.
    assert!(escaped.is_alive());
    let killed = std::process::Command::new("kill")
        .args(["-KILL", &escaped.pid().to_string()])
        .status()
        .expect("run kill");
    assert!(killed.success());
    assert!(escaped.gone_within(LIMIT));
}
