//! The cap of threads left running (`MAX_ABANDONED`), counted from when a
//! thread is told to stop and looked at where a call starts as well as
//! where a runtime does (the review of step 28's close, finding 9: the cap
//! was looked at only as a runtime started, and an extension disabled only
//! once its grace was over, so a runtime or a call let in during another's
//! grace left one more thread).
//!
//! A thread that will not stop is a thread held at
//! `A_CALL_IS_STARTED` through its call's time and its grace, never one
//! racing a native loop: held there, its call has said it started and its
//! script has not begun. No test waits: time is the fake timer's, and each
//! hold is let go at the end.

use std::future::Future as _;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use cineris_base::path::{AbsPath, PathStyle};
use cineris_base::{CancelToken, FakeClock};
use cineris_ext::capability::Capabilities;
use cineris_ext::engine::{
    A_CALL_IS_STARTED, CallContext, CallError, GRACE, LoadError, MAX_ABANDONED, ScriptRuntime,
    Source, Target, abandoned_threads, is_disabled,
};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_testkit::{FakeTimer, Hold, block_on, join};
use serde_json::{Value as Json, json};

use crate::{FakeHost, extension};

const TIME: Duration = Duration::from_secs(7);

fn timer() -> FakeTimer {
    FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH))
}

/// An extension of one tool, `t`, told apart from others by `mark`.
fn tool(mark: usize) -> Arc<Snapshot> {
    extension(
        "",
        &format!("// {mark}\ncineris.tool({{ name: 't', description: 'a tool', run: () => 1 }});"),
    )
}

fn capabilities(snapshot: &Snapshot) -> Capabilities {
    let manifest = Manifest::of(snapshot).unwrap();
    let workspace = AbsPath::parse("/work/app", PathStyle::Posix).unwrap();
    Capabilities::compile(&manifest.capabilities, &workspace).unwrap()
}

fn start(snapshot: &Arc<Snapshot>, timer: &FakeTimer) -> Result<ScriptRuntime, LoadError> {
    block_on(starting(snapshot, timer))
}

async fn starting(snapshot: &Arc<Snapshot>, timer: &FakeTimer) -> Result<ScriptRuntime, LoadError> {
    ScriptRuntime::start("probe", Source::Snapshot(Arc::clone(snapshot)), None, timer).await
}

async fn call(
    runtime: &ScriptRuntime,
    capabilities: &Capabilities,
    host: &FakeHost,
    timer: &FakeTimer,
) -> Result<Json, CallError> {
    let cancel = CancelToken::new();
    runtime
        .call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host,
                capabilities,
                time: TIME,
                timer,
                clock: timer.clock(),
                cancel: &cancel,
                asked: None,
            },
        )
        .await
}

/// The call's time, once its thread is held at the point, and the start of
/// the grace its runtime then gives the thread.
async fn out_of_time(timer: &FakeTimer) {
    timer.sleeping_for(TIME).await;
    timer.advance(TIME);
    timer.sleeping_for(GRACE).await;
}

/// One session's call runs out of time and its thread is in its grace: not
/// alive, its extension not disabled yet. The session's next use of the
/// extension (a hook of a call planned meanwhile) starts a fresh runtime:
/// that start waits for the first thread to be judged, and, the first left
/// running, is refused. One thread of the extension is left, not two.
#[test]
fn a_runtime_started_in_anothers_grace_waits_and_is_refused_when_that_one_is_left() {
    let timer = timer();
    let snapshot = tool(0);
    let capabilities = capabilities(&snapshot);
    let first = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let started = Hold::new(A_CALL_IS_STARTED);
    let before = abandoned_threads();
    let (first_result, second) =
        block_on(join(call(&first, &capabilities, &host, &timer), async {
            started.reach();
            out_of_time(&timer).await;
            assert!(!first.is_alive());
            assert!(!is_disabled(&snapshot.hash()));
            let (second, ()) = join(starting(&snapshot, &timer), async {
                timer.advance(GRACE);
            })
            .await;
            second
        }));
    assert_eq!(first_result, Err(CallError::TimedOut));
    assert!(first.was_abandoned());
    assert!(
        matches!(second, Err(LoadError::Disabled)),
        "a runtime of the extension started in the first one's grace, before it was judged: \
         {second:?}"
    );
    assert_eq!(abandoned_threads(), before + 1);
    started.release();
    assert_eq!(started.timed_out(), 0, "the held thread went on by itself");
}

/// The start in another's grace waits, and is not refused for the wait:
/// once the first thread comes back, within its grace, the new runtime
/// starts, and nothing is left running.
#[test]
fn a_runtime_started_in_anothers_grace_goes_on_once_that_thread_comes_back() {
    let timer = timer();
    let snapshot = tool(0);
    let capabilities = capabilities(&snapshot);
    let first = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let started = Hold::new(A_CALL_IS_STARTED);
    let before = abandoned_threads();
    let (first_result, second) =
        block_on(join(call(&first, &capabilities, &host, &timer), async {
            started.reach();
            out_of_time(&timer).await;
            let (second, ()) = join(starting(&snapshot, &timer), async {
                // Let go, the thread finds its runtime stopped and ends.
                started.release();
            })
            .await;
            second
        }));
    assert_eq!(first_result, Err(CallError::TimedOut));
    assert!(!first.was_abandoned());
    let second = second.expect("the start goes on once the first thread has ended");
    assert!(second.is_alive());
    assert_eq!(abandoned_threads(), before);
    assert!(!is_disabled(&snapshot.hash()));
}

/// One extension with a runtime in each of five sessions of one process (a
/// server's, a Coordinator's employees). The first session's call runs out
/// of time; the others' calls, made during its grace, wait for it to be
/// judged, and, once it is left running, are refused: one thread left, not
/// one a session past the cap.
#[test]
fn calls_of_other_sessions_made_in_a_grace_are_refused_once_that_thread_is_left() {
    let sessions = MAX_ABANDONED + 1;
    let timer = timer();
    let snapshot = tool(0);
    let capabilities = capabilities(&snapshot);
    let runtimes: Vec<ScriptRuntime> = (0..sessions)
        .map(|_| start(&snapshot, &timer).unwrap())
        .collect();
    let hosts: Vec<FakeHost> = (0..sessions).map(|_| FakeHost::new()).collect();
    let started = Hold::new(A_CALL_IS_STARTED);
    let before = abandoned_threads();
    let (first, others) = block_on(join(
        call(&runtimes[0], &capabilities, &hosts[0], &timer),
        async {
            started.reach();
            out_of_time(&timer).await;
            let others = runtimes[1..]
                .iter()
                .zip(&hosts[1..])
                .map(|(runtime, host)| Box::pin(call(runtime, &capabilities, host, &timer)));
            let mut others: Vec<_> = others.collect();
            // Each is polled once, in the grace: let in, it would be sent.
            let mut context = Context::from_waker(Waker::noop());
            for other in &mut others {
                assert!(other.as_mut().poll(&mut context).is_pending());
            }
            timer.advance(GRACE);
            let mut results = Vec::new();
            for other in others {
                results.push(other.await);
            }
            results
        },
    ));
    assert_eq!(first, Err(CallError::TimedOut));
    assert!(runtimes[0].was_abandoned());
    for result in &others {
        assert_eq!(
            result,
            &Err(CallError::Disabled),
            "a call made in another runtime's grace was let in"
        );
    }
    assert_eq!(abandoned_threads(), before + 1);
    assert!(runtimes[1..].iter().all(|runtime| !runtime.was_abandoned()));
    started.release();
    assert_eq!(started.timed_out(), 0, "a held thread went on by itself");
}

/// Past the cap of threads left running, a runtime alive before it was
/// reached takes no call either, as no runtime starts: four extensions each
/// leave a thread, and a fifth's call is refused at once, saying why.
#[test]
fn past_the_cap_a_runtime_alive_before_it_takes_no_call() {
    let timer = timer();
    let snapshots: Vec<Arc<Snapshot>> = (0..=MAX_ABANDONED).map(tool).collect();
    let runtimes: Vec<ScriptRuntime> = snapshots
        .iter()
        .map(|snapshot| start(snapshot, &timer).unwrap())
        .collect();
    let host = FakeHost::new();
    let started = Hold::new(A_CALL_IS_STARTED);
    assert_eq!(abandoned_threads(), 0, "a test of its own process");
    for (index, (runtime, snapshot)) in runtimes
        .iter()
        .zip(&snapshots)
        .enumerate()
        .take(MAX_ABANDONED)
    {
        let capabilities = capabilities(snapshot);
        let (result, ()) = block_on(join(call(runtime, &capabilities, &host, &timer), async {
            started.reach_times(index + 1);
            out_of_time(&timer).await;
            timer.advance(GRACE);
        }));
        assert_eq!(result, Err(CallError::TimedOut));
        assert!(runtime.was_abandoned());
    }
    assert_eq!(abandoned_threads(), MAX_ABANDONED);
    let last = &runtimes[MAX_ABANDONED];
    assert!(last.is_alive());
    let capabilities = capabilities(&snapshots[MAX_ABANDONED]);
    let mut refused = Box::pin(call(last, &capabilities, &host, &timer));
    let polled = refused
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    // Refused at once, saying how many and how it clears (matched by its
    // text, so that the test also runs where the refusal is not there).
    match polled {
        Poll::Ready(Err(error)) => {
            let said = error.to_string();
            assert!(
                said.contains(&format!("{MAX_ABANDONED} extension threads"))
                    && said.contains("restarts"),
                "{said}"
            );
        }
        other => panic!("a call was let in past the cap of threads left running: {other:?}"),
    }
    drop(refused);
    started.release();
    assert_eq!(started.timed_out(), 0, "a held thread went on by itself");
}

/// A start given up while its main module will not stop (its future
/// dropped, the core's runtime gone) tells its thread to stop, and the
/// thread counts from then: the extension's next start waits for it, judges
/// it left running once a grace has passed, and is refused. The module is
/// held as it registers its tool.
#[test]
fn a_thread_whose_start_was_given_up_is_judged_by_the_next_start() {
    let timer = timer();
    let snapshot = tool(0);
    let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
    let before = abandoned_threads();
    {
        let mut given_up = Box::pin(starting(&snapshot, &timer));
        assert!(
            given_up
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        registers.reach();
    }
    let (next, ()) = block_on(join(starting(&snapshot, &timer), async {
        timer.advance(GRACE);
    }));
    assert!(
        matches!(next, Err(LoadError::Disabled)),
        "the extension started again while its start that would not stop ran on: {next:?}"
    );
    assert_eq!(abandoned_threads(), before + 1);
    registers.release();
    assert_eq!(
        registers.timed_out(),
        0,
        "the held thread went on by itself"
    );
}

/// A call whose future is given up while its thread will not stop (its
/// task dropped) has nobody to judge its thread: the thread counts from
/// when it is told to stop, and the extension's next start judges it once
/// a grace has passed, and is refused, its extension disabled.
#[test]
fn a_thread_whose_call_was_given_up_is_judged_by_the_next_start() {
    let timer = timer();
    let snapshot = tool(0);
    let capabilities = capabilities(&snapshot);
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let started = Hold::new(A_CALL_IS_STARTED);
    let before = abandoned_threads();
    {
        let mut given_up = Box::pin(call(&runtime, &capabilities, &host, &timer));
        assert!(
            given_up
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        started.reach();
    }
    assert!(!runtime.is_alive());
    // The start is polled first: waiting, it sleeps for a grace, which
    // then passes.
    let (next, ()) = block_on(join(starting(&snapshot, &timer), async {
        timer.advance(GRACE);
    }));
    assert!(
        matches!(next, Err(LoadError::Disabled)),
        "the extension started again while its thread that would not stop ran on: {next:?}"
    );
    assert!(runtime.was_abandoned());
    assert_eq!(abandoned_threads(), before + 1);
    started.release();
    assert_eq!(started.timed_out(), 0, "the held thread went on by itself");
}
