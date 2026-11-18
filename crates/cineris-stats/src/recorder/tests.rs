#![allow(clippy::unwrap_used, reason = "a test")]

use std::sync::Arc;

use cineris_base::FakeClock;
use cineris_protocol::Timestamp;

use super::*;

fn dropped_at(at: &str, count: u64) -> Record {
    Record::Dropped {
        at: Timestamp::new(at),
        count,
    }
}

/// The writer is never started, so nothing takes from the queue: it is the
/// stalled writer, as a disk that does not answer would leave it.
#[test]
fn a_stalled_writer_drops_and_counts_and_never_waits() {
    let scratch = cineris_base_scratch("stalled");
    let clock = Arc::new(FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap());
    let mut options = RecorderOptions::new(StatsDir::new(scratch.join("stats")));
    options.queue = 4;
    options.zone = Some(Zone::utc());
    let recorder = Recorder::new(options, clock);
    // Each of these returns at once, whether there is room or not. With a
    // recorder that waited for room, the fifth would never return.
    for _ in 0..10 {
        recorder.record(dropped_at("2026-11-15T10:00:00Z", 0));
    }
    assert_eq!(recorder.dropped(), 6);
    // Nothing was written, or even made.
    assert!(!scratch.join("stats").exists());

    // Once the writer runs, it writes what was queued and the count.
    recorder.start().unwrap();
    recorder.flush();
    let text = std::fs::read_to_string(scratch.join("stats").join("events-2026-11.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 5, "{text}");
    assert_eq!(
        text.lines()
            .filter(|line| line.contains(r#""count":6"#))
            .count(),
        1,
        "{text}"
    );
    assert_eq!(recorder.dropped(), 0);
}

#[test]
fn raw_events_are_kept_three_months_by_default() {
    let options = RecorderOptions::new(StatsDir::new("stats"));
    assert_eq!(options.retention, 3);
    assert_eq!(options.retention, DEFAULT_RETENTION);
}

fn cineris_base_scratch(test: &str) -> cineris_testkit::Scratch {
    cineris_testkit::Scratch::outside("cineris-stats", test)
}

/// A flush that its owner gives up returns at once, with what was left: the
/// end of a process waits for the writer only so long (found by the
/// independent review of step 16, where `Core::shutdown` waited with no
/// limit).
#[test]
fn a_flush_given_up_returns_at_once_with_what_was_left() {
    let scratch = cineris_base_scratch("given_up");
    let clock = Arc::new(FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap());
    let mut options = RecorderOptions::new(StatsDir::new(scratch.join("stats")));
    options.zone = Some(Zone::utc());
    let recorder = Recorder::new(options, clock);
    // Never started: nothing is waited for, and the queue's event is told.
    recorder.record(dropped_at("2026-11-15T10:00:00Z", 0));
    assert_eq!(recorder.flush(), Flushed::NotStarted { waiting: 1 });
    // Started, and given up before it began: the answer is "given up",
    // whatever the writer then does.
    recorder.start().unwrap();
    let give_up = GiveUp::new();
    give_up.now();
    assert!(matches!(
        recorder.flush_until(&give_up),
        Flushed::GaveUp { .. }
    ));
    assert_eq!(recorder.flush(), Flushed::Done);
}

/// A writer whose thread has ended takes nothing more: what is recorded
/// after is dropped and counted, and a flush says it stopped rather than
/// returning as if all were written (the independent review of step 16's
/// note: such a writer was invisible).
#[test]
fn a_writer_that_stopped_is_counted_not_hidden() {
    let clock = Arc::new(FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap());
    let recorder = Recorder::new(RecorderOptions::new(StatsDir::new("unused")), clock);
    // What a thread that panicked leaves: its end of the queue is gone.
    drop(recorder.unstarted.lock().unwrap().take());
    for _ in 0..3 {
        recorder.record(dropped_at("2026-11-15T10:00:00Z", 0));
    }
    assert_eq!(recorder.dropped(), 3);
    assert_eq!(recorder.flush(), Flushed::Stopped { waiting: 0 });
}

/// A writer that ends while a flush waits for it answers the flush as
/// stopped: the answer it drops says so.
#[test]
fn a_flush_the_writer_drops_unanswered_says_it_stopped() {
    let state = GiveUp::new();
    drop(Answer(Arc::clone(&state.state)));
    assert!(matches!(*state.state.lock(), Some(Ended::Stopped)));
    // An answer given is not taken back by its drop.
    let state = GiveUp::new();
    let answer = Answer(Arc::clone(&state.state));
    answer.written();
    drop(answer);
    assert!(matches!(*state.state.lock(), Some(Ended::Written)));
}
