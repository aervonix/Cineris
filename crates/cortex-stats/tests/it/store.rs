use std::time::Duration;

use cortex_base::{LocalDate, Zone};
use cortex_protocol::usage::{UsageGroup, UsageRange};
use cortex_stats::{Aggregates, Query, StatsDir, reads, report};

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
    let recorder = recorder(&dir, Zone::utc(), clock.clone(), 12);
    recorder.record(request("2025-01-10T12:00:00Z", "p", "opus", 7_000));
    recorder.record(request("2025-01-11T12:00:00Z", "p", "opus", 3_000));
    recorder.flush();
    // Almost two years later the January file is gone, and its days, more
    // than 400 days back, are one month.
    clock.set_now(time("2026-11-15T12:00:00Z"));
    recorder.record(request("2026-11-15T12:00:00Z", "p", "opus", 5_000));
    recorder.flush();
    assert!(!dir.events_file("2025-01").exists());
    let part = Aggregates::load(&dir).unwrap().total();
    let periods: Vec<&str> = part.usage.keys().map(|key| key.period.as_str()).collect();
    assert_eq!(periods, ["2025-01", "2026-11-15"]);
    let all = report(
        &part,
        &all_time(vec![UsageGroup::Model]),
        &Zone::utc(),
        time("2026-11-15T12:00:00Z"),
        &[],
        true,
    );
    assert_eq!(all.totals.requests, 3);
    assert_eq!(all.totals.costs[0].usd_nanos, 15_000);
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
