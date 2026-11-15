use cortex_base::{LocalDate, Zone};
use cortex_protocol::usage::{UsageGroup, UsageRange};
use cortex_stats::{Line, Part, Query, days_of, report};

use crate::{request, time, turn};

fn date(text: &str) -> LocalDate {
    LocalDate::parse(text).unwrap()
}

fn query(range: UsageRange) -> Query {
    Query {
        range,
        group_by: vec![UsageGroup::Provider, UsageGroup::Model],
        project: None,
        week_start: 0,
        rolling: false,
    }
}

/// One request of a dollar on each of these local days.
fn part_of(days: &[&str]) -> Part {
    let mut part = Part::default();
    for day in days {
        part.add_line(&Line {
            day: (*day).to_owned(),
            record: request(&format!("{day}T12:00:00Z"), "p", "opus", 1_000_000_000),
        });
    }
    part
}

fn requests(part: &Part, range: UsageRange, zone: &Zone, now: &str) -> u64 {
    report(part, &query(range), zone, time(now), &[], true)
        .totals
        .requests
}

#[test]
fn the_days_of_each_range() {
    // Sunday 15 November 2026.
    let today = date("2026-11-15");
    let range = |range, week_start, rolling| {
        let (first, last) = days_of(&range, today, week_start, rolling).unwrap();
        (first.to_string(), last.to_string())
    };
    let pair = |first: &str, last: &str| (first.to_owned(), last.to_owned());
    assert_eq!(
        range(UsageRange::Today, 0, false),
        pair("2026-11-15", "2026-11-15")
    );
    // A week from Monday, and from Sunday when it starts on Sunday.
    assert_eq!(
        range(UsageRange::Week, 0, false),
        pair("2026-11-09", "2026-11-15")
    );
    assert_eq!(
        range(UsageRange::Week, 6, false),
        pair("2026-11-15", "2026-11-15")
    );
    assert_eq!(
        range(UsageRange::Month, 0, false),
        pair("2026-11-01", "2026-11-15")
    );
    assert_eq!(
        range(UsageRange::Week, 0, true),
        pair("2026-11-09", "2026-11-15")
    );
    assert_eq!(
        range(UsageRange::Month, 0, true),
        pair("2026-10-17", "2026-11-15")
    );
    assert!(days_of(&UsageRange::All, today, 0, false).is_none());
}

#[test]
fn a_range_ends_at_the_local_midnight_on_either_side() {
    let part = part_of(&[
        "2026-10-31",
        "2026-11-01",
        "2026-11-08",
        "2026-11-09",
        "2026-11-15",
    ]);
    let utc = Zone::utc();
    let now = "2026-11-15T23:59:59Z";
    assert_eq!(requests(&part, UsageRange::Today, &utc, now), 1);
    // Monday 9 November begins the week; Sunday the 8th is the week before.
    assert_eq!(requests(&part, UsageRange::Week, &utc, now), 2);
    // The 1st begins the month; 31 October is the month before.
    assert_eq!(requests(&part, UsageRange::Month, &utc, now), 4);
    assert_eq!(requests(&part, UsageRange::All, &utc, now), 5);
    // One second later it is the 16th: today has nothing yet.
    assert_eq!(
        requests(&part, UsageRange::Today, &utc, "2026-11-16T00:00:00Z"),
        0
    );
}

#[test]
fn today_is_the_users_today_not_utcs() {
    let part = part_of(&["2026-11-15", "2026-11-16"]);
    // At 01:00 UTC on the 16th it is still the 15th five hours west, and
    // already the 16th nine hours east.
    let now = "2026-11-16T01:00:00Z";
    let west = Zone::fixed(-5 * 3600);
    let east = Zone::fixed(9 * 3600);
    let today_west = report(
        &part,
        &query(UsageRange::Today),
        &west,
        time(now),
        &[],
        true,
    );
    assert_eq!(today_west.totals.requests, 1);
    assert_eq!(today_west.from.unwrap().as_str(), "2026-11-15T05:00:00Z");
    let today_east = report(
        &part,
        &query(UsageRange::Today),
        &east,
        time(now),
        &[],
        true,
    );
    assert_eq!(today_east.totals.requests, 1);
    assert_eq!(today_east.from.unwrap().as_str(), "2026-11-15T15:00:00Z");
    // Across a change of offset the month begins at its own midnight.
    let london = Zone::posix("GMT0BST,M3.5.0/1,M10.5.0").unwrap();
    let month = report(
        &part_of(&["2026-07-01"]),
        &query(UsageRange::Month),
        &london,
        time("2026-07-01T00:30:00Z"),
        &[],
        true,
    );
    assert_eq!(month.from.unwrap().as_str(), "2026-06-30T23:00:00Z");
    assert_eq!(month.totals.requests, 1);
}

#[test]
fn rows_group_by_what_was_asked_and_time_goes_with_them() {
    let mut part = part_of(&["2026-11-15"]);
    part.add_line(&Line {
        day: "2026-11-15".to_owned(),
        record: request("2026-11-15T12:00:00Z", "q", "haiku", 500),
    });
    part.add_line(&Line {
        day: "2026-11-15".to_owned(),
        record: turn("2026-11-15T12:00:00Z", "q"),
    });
    let mut by_project = query(UsageRange::Today);
    by_project.group_by = vec![UsageGroup::Project];
    let report = report(
        &part,
        &by_project,
        &Zone::utc(),
        time("2026-11-15T13:00:00Z"),
        &[],
        true,
    );
    let projects: Vec<&str> = report
        .rows
        .iter()
        .map(|row| row.project.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(projects, ["p", "q"]);
    let q = &report.rows[1];
    let time = q.time.as_ref().unwrap();
    assert_eq!(time.turns, 1);
    // The parts of a turn's time add up to its wall time.
    assert_eq!(time.parts_ms(), time.wall_ms);
    assert_eq!(report.tools[0].tool, "read");
    assert_eq!(report.tools[0].share_ms, 250);
    // By speed there is no time: a turn has no one speed.
    let mut by_speed = query(UsageRange::Today);
    by_speed.group_by = vec![UsageGroup::Speed];
    let report = cortex_stats::report(
        &part,
        &by_speed,
        &Zone::utc(),
        crate::time("2026-11-15T13:00:00Z"),
        &[],
        true,
    );
    assert!(report.rows.iter().all(|row| row.time.is_none()));
    assert_eq!(report.totals.requests, 2);
}
