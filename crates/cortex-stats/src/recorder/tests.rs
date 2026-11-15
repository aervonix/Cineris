#![allow(clippy::unwrap_used, reason = "a test")]

use std::sync::Arc;

use cortex_base::FakeClock;
use cortex_protocol::Timestamp;

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
    let scratch = cortex_base_scratch("stalled");
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
        text.lines().filter(|line| line.contains(r#""count":6"#)).count(),
        1,
        "{text}"
    );
    assert_eq!(recorder.dropped(), 0);
}

fn cortex_base_scratch(test: &str) -> cortex_testkit::Scratch {
    cortex_testkit::Scratch::outside("cortex-stats", test)
}
