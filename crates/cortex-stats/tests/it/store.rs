use std::time::Duration;

use std::sync::Arc;

use cortex_base::{LocalDate, Zone};
use cortex_protocol::usage::{UsageGroup, UsageRange};
use cortex_stats::{Aggregates, Found, Query, StatsDir, reads, report};

use crate::{clock, recorder, request, scratch, time, tool, turn};

fn date(text: &str) -> LocalDate {
    LocalDate::parse(text).unwrap()
}

fn all_time(by: Vec<UsageGroup>) -> Query {
    Query {
        range: UsageRange::All,
        group_by: by,
        project: None,
        week_start: 0,
        rolling: false,
    }
}

#[test]
fn an_event_goes_to_its_month_with_the_local_day_it_fell_on() {
    let scratch = scratch("an_event_goes_to_its_month");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-12-01T03:00:00Z");
    // Five hours behind UTC: 03:00 UTC on 1 December is 30 November there.
    let recorder = recorder(&dir, Zone::fixed(-5 * 3600), clock, 12);
    recorder.record(request("2026-12-01T03:00:00Z", "p", "opus", 1));
    recorder.flush();
    let text = std::fs::read_to_string(dir.events_file("2026-12")).unwrap();
    assert!(text.contains(r#""day":"2026-11-30""#), "{text}");
    assert!(text.contains(r#""kind":"request""#), "{text}");
    let total = Aggregates::load(&dir).unwrap().total();
    let periods: Vec<&str> = total.usage.keys().map(|key| key.period.as_str()).collect();
    assert_eq!(periods, ["2026-11-30"]);
}

#[test]
fn every_file_is_owner_only() {
    let scratch = scratch("every_file_is_owner_only");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-11-15T10:00:00Z");
    let recorder = recorder(&dir, Zone::utc(), clock.clone(), 12);
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.limits(crate::limits::headers_limits(
        "anthropic",
        "2026-11-15T10:00:00Z",
        5,
    ));
    recorder.flush();
    for path in [
        dir.events_file("2026-11"),
        dir.aggregates_file(),
        dir.limits_file(),
    ] {
        assert!(
            cortex_base::private::is_private(&path).unwrap(),
            "{}",
            path.display()
        );
    }
}

#[test]
fn the_aggregates_rebuild_to_the_same_values_from_the_files() {
    let scratch = scratch("the_aggregates_rebuild");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-09-01T00:00:00Z");
    let recorder = recorder(
        &dir,
        Zone::posix("GMT0BST,M3.5.0/1,M10.5.0").unwrap(),
        clock.clone(),
        1,
    );
    // Three months of events, brought up to date at odd moments in between,
    // with the first month's file pruned on the way.
    let mut n = 0u64;
    for (month, days) in [("2026-09", 30), ("2026-10", 31), ("2026-11", 15)] {
        for day in 1..=days {
            let at = format!("{month}-{day:02}T23:30:00Z");
            clock.set_now(time(&at));
            recorder.record(request(
                &at,
                if day % 2 == 0 { "a" } else { "b" },
                "opus",
                1000 + n,
            ));
            recorder.record(tool(&at, "a", if day % 3 == 0 { "shell" } else { "read" }));
            recorder.record(turn(&at, "b"));
            n += 1;
            if day % 7 == 0 {
                recorder.flush();
            }
        }
    }
    recorder.flush();
    // The September file is past its retention of one month.
    assert!(!dir.events_file("2026-09").exists());
    let kept = Aggregates::load(&dir).unwrap();
    assert!(!kept.kept.is_empty(), "the pruned month's part is kept");
    let rebuilt = kept.rebuild(&dir, date("2026-11-15")).unwrap();
    assert_eq!(kept.total(), rebuilt.total());
    let requests: u64 = kept.total().usage.values().map(|sum| sum.requests).sum();
    assert_eq!(requests, n);
}

#[test]
fn two_writers_that_bring_the_aggregates_up_to_date_out_of_order_lose_nothing() {
    let scratch = scratch("two_writers");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-11-15T10:00:00Z");
    let recorder = recorder(&dir, Zone::utc(), clock.clone(), 12);
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.flush();
    // Another process loaded the aggregates before more was written…
    let stale = Aggregates::load(&dir).unwrap();
    recorder.record(request("2026-11-15T10:01:00Z", "p", "opus", 2));
    recorder.flush();
    // …and saves them after this one did.
    stale.save(&dir, clock.as_ref()).unwrap();
    let mut aggregates = Aggregates::load(&dir).unwrap();
    aggregates
        .sync(&dir, date("2026-11-15"), "2026-11", Some(12))
        .unwrap();
    let requests: u64 = aggregates
        .total()
        .usage
        .values()
        .map(|sum| sum.requests)
        .sum();
    assert_eq!(requests, 2);
    assert_eq!(
        aggregates.total(),
        aggregates
            .rebuild(&dir, date("2026-11-15"))
            .unwrap()
            .total()
    );
}

#[test]
fn all_time_outlives_the_raw_files_and_old_days_fold_into_months() {
    let scratch = scratch("all_time_outlives");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2025-01-10T12:00:00Z");
    // At the default retention.
    let retention = cortex_stats::RecorderOptions::new(dir.clone()).retention;
    assert_eq!(retention, 3);
    let recorder = recorder(&dir, Zone::utc(), clock.clone(), retention);
    recorder.record(request("2025-01-10T12:00:00Z", "p", "opus", 7_000));
    recorder.record(request("2025-01-11T12:00:00Z", "p", "opus", 3_000));
    recorder.flush();
    // Four months on, January's file is past three months of retention and
    // gone, and what it held is still counted.
    clock.set_now(time("2025-05-15T12:00:00Z"));
    recorder.record(request("2025-05-15T12:00:00Z", "p", "opus", 1_000));
    recorder.flush();
    assert!(!dir.events_file("2025-01").exists());
    assert!(dir.events_file("2025-05").exists());
    let part = Aggregates::load(&dir).unwrap().total();
    let all = report(
        &part,
        &all_time(vec![UsageGroup::Model]),
        &Zone::utc(),
        time("2025-05-15T12:00:00Z"),
        &[],
        true,
    );
    assert_eq!(all.totals.requests, 3);
    assert_eq!(all.totals.costs[0].usd_nanos, 11_000);
    // Almost two years later the January file is gone, and its days, more
    // than 400 days back, are one month.
    clock.set_now(time("2026-11-15T12:00:00Z"));
    recorder.record(request("2026-11-15T12:00:00Z", "p", "opus", 5_000));
    recorder.flush();
    assert!(!dir.events_file("2025-05").exists());
    let part = Aggregates::load(&dir).unwrap().total();
    let periods: Vec<&str> = part.usage.keys().map(|key| key.period.as_str()).collect();
    assert_eq!(periods, ["2025-01", "2025-05", "2026-11-15"]);
    let all = report(
        &part,
        &all_time(vec![UsageGroup::Model]),
        &Zone::utc(),
        time("2026-11-15T12:00:00Z"),
        &[],
        true,
    );
    assert_eq!(all.totals.requests, 4);
    assert_eq!(all.totals.costs[0].usd_nanos, 16_000);
}

#[test]
fn nothing_is_read_until_it_is_asked_for() {
    let scratch = scratch("nothing_is_read");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    std::fs::write(dir.aggregates_file(), b"{}").unwrap();
    let clock = clock("2026-11-15T10:00:00Z");
    let mut options = cortex_stats::RecorderOptions::new(dir.clone());
    options.zone = Some(Zone::utc());
    options.sync_every = Duration::from_secs(3600);
    // Making a recorder and recording reads nothing, and nor does starting
    // its writer and writing an event.
    let recorder = cortex_stats::Recorder::new(options, clock);
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.start().unwrap();
    recorder.record(request("2026-11-15T10:00:01Z", "p", "opus", 1));
    assert_eq!(reads(&dir), 0);
    // Bringing the aggregates up to date does.
    recorder.flush();
    assert!(reads(&dir) > 0);
}

fn requests(dir: &StatsDir) -> u64 {
    Aggregates::load(dir)
        .unwrap()
        .total()
        .usage
        .values()
        .map(|sum| sum.requests)
        .sum()
}

/// `cortex stats reset` in another process while this one records: the
/// writer held its month's file open, so on Linux its later lines went to
/// the file the reset had deleted, and were lost (found by the independent
/// review of step 16). It now opens the file the name gives.
#[test]
fn a_reset_while_a_writer_runs_keeps_the_writers_later_lines() {
    let scratch = scratch("a_reset_while_a_writer_runs");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-11-15T10:00:00Z");
    let recorder = recorder(&dir, Zone::utc(), clock, 12);
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.flush();
    assert!(dir.events_file("2026-11").exists());
    dir.reset().unwrap();
    for minute in 1..=3 {
        recorder.record(request(
            &format!("2026-11-15T10:0{minute}:00Z"),
            "p",
            "opus",
            2,
        ));
    }
    recorder.flush();
    assert!(
        dir.events_file("2026-11").exists(),
        "the writer's three later events went to the file the reset deleted"
    );
    assert_eq!(requests(&dir), 3);
}

/// January's request, which only `kept` holds once its file is pruned.
fn january_pruned(dir: &StatsDir) -> Arc<cortex_base::FakeClock> {
    let clock = clock("2026-01-10T12:00:00Z");
    let recorder = recorder(dir, Zone::utc(), clock.clone(), 1);
    recorder.record(request("2026-01-10T12:00:00Z", "p", "opus", 7_000));
    recorder.flush();
    clock.set_now(time("2026-04-10T12:00:00Z"));
    recorder.record(request("2026-04-10T12:00:00Z", "p", "opus", 1_000));
    recorder.flush();
    assert!(!dir.events_file("2026-01").exists(), "January is pruned");
    assert_eq!(requests(dir), 2);
    clock
}

/// A newer build's aggregates file is left for it: this build records its
/// events in the monthly files, which the newer build reads, and does not
/// write the aggregates or prune (found by the independent review of step
/// 16: the file was replaced, January with it).
#[test]
fn a_newer_builds_aggregates_file_is_left_for_it() {
    let scratch = scratch("a_newer_builds_aggregates");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = january_pruned(&dir);
    let text = std::fs::read_to_string(dir.aggregates_file()).unwrap();
    assert!(text.contains(r#""version":1"#), "{text}");
    let newer = text.replace(r#""version":1"#, r#""version":2"#);
    std::fs::write(dir.aggregates_file(), &newer).unwrap();
    // Months later, past April's retention too.
    clock.set_now(time("2026-07-10T12:00:00Z"));
    let recorder = recorder(&dir, Zone::utc(), clock, 1);
    recorder.record(request("2026-07-10T12:05:00Z", "p", "opus", 1_000));
    recorder.flush();
    assert_eq!(
        std::fs::read_to_string(dir.aggregates_file()).unwrap(),
        newer,
        "the newer build's file is as it left it"
    );
    assert!(dir.events_file("2026-07").exists(), "the event is recorded");
    assert!(
        dir.events_file("2026-04").exists(),
        "nothing is pruned that only the newer build's file would keep"
    );
    let loaded = Aggregates::load(&dir).unwrap();
    assert_eq!(loaded.found(), &Found::Newer { version: 2 });
}

/// An aggregates file that does not parse is moved aside, owner-only and
/// byte for byte, under a name with the time, before the writer saves
/// aggregates rebuilt from the monthly files: the months only it held are
/// in the copy, for the user (found by the independent review of step 16).
#[test]
fn an_aggregates_file_that_does_not_parse_is_moved_aside_before_anything_is_saved() {
    let scratch = scratch("an_aggregates_file_that_does_not_parse");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = january_pruned(&dir);
    // A file cut short: what a disk that failed part-way leaves.
    let whole = std::fs::read(dir.aggregates_file()).unwrap();
    let damaged = whole[..whole.len() / 2].to_vec();
    std::fs::write(dir.aggregates_file(), &damaged).unwrap();
    clock.set_now(time("2026-04-10T13:00:00Z"));
    let recorder = recorder(&dir, Zone::utc(), clock, 1);
    recorder.record(request("2026-04-10T13:00:00Z", "p", "opus", 1_000));
    recorder.flush();
    let aside = dir.set_aside().unwrap();
    assert_eq!(aside.len(), 1, "{aside:?}");
    let name = aside[0].file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, "aggregates.json.unreadable-2026-04-10T13-00-00Z");
    assert_eq!(std::fs::read(&aside[0]).unwrap(), damaged);
    assert!(cortex_base::private::is_private(&aside[0]).unwrap());
    // The aggregates are the monthly files' again: April's two requests.
    let loaded = Aggregates::load(&dir).unwrap();
    assert_eq!(loaded.found(), &Found::Read);
    assert_eq!(requests(&dir), 2);
}

/// Step 17's rule, held by the type: aggregates that could not read the
/// file that is there are never saved over it, and prune nothing, whoever
/// loaded them.
#[test]
fn aggregates_that_could_not_read_the_file_are_never_saved_over_it() {
    let scratch = scratch("aggregates_that_could_not_read");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = january_pruned(&dir);
    for (bytes, found) in [
        (b"\0\0\0\0".to_vec(), Found::Unreadable),
        (
            br#"{"version":3,"files":"another shape"}"#.to_vec(),
            Found::Newer { version: 3 },
        ),
    ] {
        std::fs::write(dir.aggregates_file(), &bytes).unwrap();
        let mut loaded = Aggregates::load(&dir).unwrap();
        assert_eq!(loaded.found(), &found);
        assert!(!found.may_save());
        loaded
            .sync(&dir, date("2026-11-15"), "2026-11", Some(1))
            .unwrap();
        assert!(dir.events_file("2026-04").exists(), "nothing was pruned");
        assert!(loaded.save(&dir, clock.as_ref()).is_err());
        assert_eq!(std::fs::read(dir.aggregates_file()).unwrap(), bytes);
        // Nor what a caller makes by hand.
        assert!(Aggregates::default().save(&dir, clock.as_ref()).is_err());
        assert_eq!(std::fs::read(dir.aggregates_file()).unwrap(), bytes);
    }
    assert!(dir.set_aside().unwrap().is_empty(), "reading moves nothing");
}

/// A line cut short (a full disk, a power cut) has no newline. The next
/// writer ends it before its own first line, which it would otherwise have
/// joined and lost (found by the independent review of step 16).
#[test]
fn a_half_written_last_line_does_not_take_the_next_writers_first_line() {
    let scratch = scratch("a_half_written_last_line");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    std::fs::write(
        dir.events_file("2026-11"),
        br#"{"day":"2026-11-15","kind":"req"#,
    )
    .unwrap();
    let clock = clock("2026-11-15T10:00:00Z");
    let recorder = recorder(&dir, Zone::utc(), clock, 12);
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.flush();
    let total = Aggregates::load(&dir).unwrap().total();
    assert_eq!(requests(&dir), 1, "unreadable: {}", total.unreadable);
    assert_eq!(total.unreadable, 1, "the cut line is one unreadable line");
}

/// Two recorders (two processes) appending to one month at once and
/// bringing the aggregates up to date as they go: nothing is lost or
/// counted twice. The independent review of step 16 found this sound.
#[test]
fn two_recorders_writing_at_once_lose_nothing() {
    let scratch = scratch("two_recorders_writing_at_once");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-11-15T10:00:00Z");
    let make = || recorder(&dir, Zone::utc(), clock.clone(), 12);
    let each = 100u64;
    let threads: Vec<_> = [make(), make()]
        .into_iter()
        .map(|recorder| {
            std::thread::spawn(move || {
                for n in 0..each {
                    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", n));
                    if n % 10 == 0 {
                        recorder.flush();
                    }
                }
                recorder.flush();
                recorder.dropped()
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 0);
    }
    let aggregates = Aggregates::load(&dir).unwrap();
    let mut synced = aggregates.clone();
    synced
        .sync(&dir, date("2026-11-15"), "2026-11", None)
        .unwrap();
    let rebuilt = aggregates.rebuild(&dir, date("2026-11-15")).unwrap();
    assert_eq!(synced.total(), rebuilt.total());
    let counted: u64 = synced.total().usage.values().map(|sum| sum.requests).sum();
    assert_eq!(counted, 2 * each);
}

/// A monthly file stays for `stats.retention` whole months after its own,
/// and goes the month after: at a retention of two, in November, September
/// stays and August goes.
#[test]
fn a_monthly_file_stays_for_its_retention_and_goes_the_month_after() {
    let scratch = scratch("a_monthly_file_stays");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    for month in ["2026-08", "2026-09"] {
        std::fs::write(dir.events_file(month), b"").unwrap();
    }
    let mut aggregates = Aggregates::load(&dir).unwrap();
    aggregates
        .sync(&dir, date("2026-11-15"), "2026-11", Some(2))
        .unwrap();
    assert!(dir.events_file("2026-09").exists(), "two months after its own");
    assert!(!dir.events_file("2026-08").exists(), "three months after");
}

/// A line another process is still writing has no newline yet: it is left
/// for later, not counted as unreadable and passed (where its event would be
/// lost), and read whole once it is.
#[test]
fn a_line_still_being_written_is_read_once_it_is_whole() {
    let scratch = scratch("a_line_still_being_written");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    let line = serde_json::to_vec(&cortex_stats::Line {
        day: "2026-11-15".to_owned(),
        writer: None,
        record: request("2026-11-15T10:00:00Z", "p", "opus", 1),
    })
    .unwrap();
    let (first, rest) = line.split_at(line.len() / 2);
    std::fs::write(dir.events_file("2026-11"), first).unwrap();
    let mut aggregates = Aggregates::load(&dir).unwrap();
    aggregates
        .sync(&dir, date("2026-11-15"), "2026-11", None)
        .unwrap();
    assert_eq!(aggregates.total().unreadable, 0);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.events_file("2026-11"))
        .unwrap();
    std::io::Write::write_all(&mut file, rest).unwrap();
    std::io::Write::write_all(&mut file, b"\n").unwrap();
    aggregates
        .sync(&dir, date("2026-11-15"), "2026-11", None)
        .unwrap();
    let total = aggregates.total();
    assert_eq!(total.unreadable, 0);
    assert_eq!(
        total.usage.values().map(|sum| sum.requests).sum::<u64>(),
        1
    );
}

/// Day buckets stay days for 400 days, and fold into their month after.
#[test]
fn days_fold_into_months_after_four_hundred_days() {
    let today = date("2026-11-15");
    let mut part = cortex_stats::Part::default();
    for back in [400, 401] {
        part.add_line(&cortex_stats::Line {
            day: today.add_days(-back).to_string(),
            writer: None,
            record: request("2025-10-11T12:00:00Z", "p", "opus", 1),
        });
    }
    part.fold_days(today);
    let periods: Vec<&str> = part.usage.keys().map(|key| key.period.as_str()).collect();
    // 401 days back is 2025-10-10, folded; 400 is 2025-10-11, kept.
    assert_eq!(periods, ["2025-10", "2025-10-11"]);
}

/// Token counts are the provider's word: sums that would overflow stop at
/// the most a figure holds, and the writer goes on (the independent
/// review of step 16's note: a debug build panicked).
#[test]
fn sums_that_would_overflow_stop_at_the_most_a_figure_holds() {
    let mut part = cortex_stats::Part::default();
    for _ in 0..2 {
        let mut huge = request("2026-11-15T10:00:00Z", "p", "opus", u64::MAX - 1);
        if let cortex_stats::Record::Request(request) = &mut huge {
            request.tokens.input = u64::MAX - 1;
        }
        part.add_line(&cortex_stats::Line {
            day: "2026-11-15".to_owned(),
            writer: None,
            record: huge,
        });
    }
    let mut merged = part.clone();
    merged.merge(&part);
    for counted in [&part, &merged] {
        let sum = counted.usage.values().next().unwrap();
        assert_eq!(sum.input, u64::MAX);
        assert_eq!(sum.metered_nanos, u64::MAX);
    }
}
