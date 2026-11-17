//! The panels' measurements (step 24; `docs/performance-budgets.md`): the
//! usage panel's first frame with a year of history, and a key's frame with
//! all four panels open. Ignored, and run in a release build:
//!
//! ```text
//! cargo xtask scratch-test …   # or, on the desk:
//! cargo test --release -p cortex-tui --features test-kit --lib perf_ -- --ignored --nocapture
//! ```
//!
//! On a hosted runner: `gh workflow run try --ref BRANCH -f os=windows -f
//! command="perf -p cortex-tui"`.
//!
//! **The usage panel's year is the application's half of the path:** the
//! key to its frame (the panel open, its history `reading…`), and the
//! report's answer to the frame with its rows, the report holding a year's
//! rows in the shape the core gives them (five providers, twelve models,
//! twenty projects, two speeds) for each grouping and range. The core's
//! half, the report made from a year's aggregates, is
//! `perf_a_years_report_for_the_usage_panel` in `cortex-stats`.

use cortex_base::{FakeClock, SharedClock};
use cortex_protocol::event::{
    CheckOutcome, CheckReport, CheckSource, Notice, PlanUpdated, UsageUpdated, Verification,
    VerificationNotice, VerificationPhase,
};
use cortex_protocol::item::{Plan, PlanItem, PlanStatus};
use cortex_protocol::methods::{ProjectEntriesResult, UsageReportParams};
use cortex_protocol::usage::{
    Cost, CostBasis, TimeTotals, UsageGroup, UsageRange, UsageReport, UsageRow, UsageTotals,
};
use cortex_protocol::{PlanItemId, ProjectKey};

use super::*;
use crate::view::project::tests::entries;
use crate::view::usage::tests::{session_usage, totals};

const PROVIDERS: [&str; 5] = ["anthropic", "openai", "openrouter", "deepseek", "venice"];

/// Twelve models, each with its provider.
const MODELS: [(&str, &str); 12] = [
    ("anthropic", "claude-opus-5-5"),
    ("anthropic", "claude-fable-5-2"),
    ("anthropic", "claude-sonnet-5-5"),
    ("anthropic", "claude-haiku-5-5"),
    ("openai", "gpt-6-luna"),
    ("openai", "gpt-6-luna-mini"),
    ("openrouter", "z-ai/glm-5.3-flash"),
    ("openrouter", "openai/gpt-6-luna"),
    ("openrouter", "anthropic/claude-haiku-5.5"),
    ("deepseek", "deepseek-flash"),
    ("deepseek", "deepseek-v4-pro"),
    ("venice", "deepseek-v4-flash-0731"),
];

fn year_totals(n: u64) -> UsageTotals {
    let mut totals = totals(
        1_000_000 + n * 7_919,
        30_000_000 + n * 104_729,
        2_000_000 + n * 1_301,
        500_000 + n * 3_571,
        vec![
            Cost {
                usd_nanos: 4_000_000_000 + n * 13_000_000,
                basis: CostBasis::Metered,
            },
            Cost {
                usd_nanos: 200_000_000 + n * 1_000_000,
                basis: CostBasis::Estimate,
            },
        ],
    );
    totals.requests = 1_000 + n;
    totals
}

/// A year's report for `group_by` over `range`, in the shape the core gives
/// it: a row for each provider and model, project, speed, or project and
/// provider; the totals; the time; the zone; and every provider's limits.
fn year_report(range: UsageRange, group_by: Vec<UsageGroup>) -> UsageReport {
    let project = |n: usize| ProjectKey::from(format!("p{n:02}").as_str());
    let label = |n: usize| format!("~/src/project-{n:02}");
    let mut rows = Vec::new();
    match group_by.as_slice() {
        [UsageGroup::Project] => {
            for n in 0..20 {
                rows.push(UsageRow {
                    project: Some(project(n)),
                    project_label: Some(label(n)),
                    totals: year_totals(n as u64),
                    ..UsageRow::default()
                });
            }
        }
        [UsageGroup::Speed] => {
            for (n, speed) in [
                cortex_protocol::Speed::Standard,
                cortex_protocol::Speed::Fast,
            ]
            .into_iter()
            .enumerate()
            {
                rows.push(UsageRow {
                    speed: Some(speed),
                    totals: year_totals(n as u64),
                    ..UsageRow::default()
                });
            }
        }
        [UsageGroup::Project, UsageGroup::Provider] => {
            for n in 0..20 {
                for (at, provider) in PROVIDERS.iter().enumerate() {
                    rows.push(UsageRow {
                        project: Some(project(n)),
                        project_label: Some(label(n)),
                        provider: Some(ProviderId::from(*provider)),
                        totals: year_totals((n * 5 + at) as u64),
                        ..UsageRow::default()
                    });
                }
            }
        }
        _ => {
            for (n, (provider, model)) in MODELS.iter().enumerate() {
                rows.push(UsageRow {
                    provider: Some(ProviderId::from(*provider)),
                    model: Some(ModelId::from(*model)),
                    totals: year_totals(n as u64),
                    ..UsageRow::default()
                });
            }
        }
    }
    let mut limits = Vec::new();
    for provider in PROVIDERS {
        let mut one = crate::view::usage::tests::anthropic_limits();
        one.provider = ProviderId::from(provider);
        limits.push(one);
    }
    UsageReport {
        range,
        from: None,
        to: None,
        group_by,
        rows,
        totals: year_totals(1_000),
        limits,
        sources: vec!["cortex-harness".to_owned()],
        recording: true,
        time: Some(TimeTotals {
            turns: 4_000,
            wall_ms: 900_000_000,
            tools_ms: 200_000_000,
            ..TimeTotals::default()
        }),
        tools: Vec::new(),
        zone: Some("Europe/Paris".to_owned()),
        notes: Vec::new(),
    }
}

fn clock() -> SharedClock {
    Arc::new(FakeClock::starting_at("2026-11-16T12:00:00Z").expect("a time"))
}

/// The `n`th of `times`, sorted, as a share in hundredths.
fn at(times: &[f64], share: usize) -> f64 {
    times[(times.len() * share / 100).min(times.len() - 1)]
}

/// The usage panel's first frame with a year of history, the application's
/// half: `/usage`'s `Enter` to its frame, and the report's answer to the
/// frame with its rows; then each `tab` and `g` the same way, through the
/// four ranges and the four groupings. The budget is 16 ms for the whole
/// path, the core's half included.
#[test]
#[ignore = "a measurement: see the module's documentation"]
fn perf_the_usage_panels_first_frame_with_a_year() {
    let runs: usize = std::env::var("CORTEX_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let start = Instant::now();
    let (mut app, link) = app(start);
    app.set_clock(clock());
    app.resize(119, 60);
    opened(&mut app, &link, start);
    link.give(event(Event::UsageUpdated(UsageUpdated {
        usage: session_usage(),
    })));
    drain(&mut app, start);
    let mut keys = Vec::new();
    let mut answers = Vec::new();
    let mut rows = 0;
    let mut now = start;
    for _ in 0..runs {
        now += Duration::from_secs(5);
        type_text(&mut app, "/usage", now);
        key(&mut app, "esc", now);
        let _ = app.frame(now);
        // The key to its frame.
        let began = Instant::now();
        key(&mut app, "enter", now);
        let _ = app.frame(now);
        keys.push(began.elapsed().as_secs_f64() * 1000.0);
        app.drawn(now);
        // Then each range and grouping, by `tab` and `g`.
        for step in 0..8 {
            let mut asked: Vec<(Ticket, UsageReportParams)> = Vec::new();
            for (ticket, op) in link.take_sent() {
                match op {
                    Op::UsageReport(params) => asked.push((ticket, params)),
                    // `usage.refresh`, asked as the panel opens.
                    Op::ConfigGet(_) => link.give(Inbound::Done(
                        ticket,
                        Ok(OpResult::ConfigGet(
                            cortex_protocol::methods::ConfigGetResult {
                                config: serde_json::Map::new(),
                                origins: Default::default(),
                                ..Default::default()
                            },
                        )),
                    )),
                    _ => {}
                }
            }
            assert_eq!(asked.len(), 1, "one report asked for, at step {step}");
            let (ticket, params) = asked[0].clone();
            let report = year_report(params.range, params.group_by);
            let began = Instant::now();
            link.give(Inbound::Done(ticket, Ok(OpResult::UsageReport(report))));
            drain(&mut app, now);
            let (_, live, _) = app.frame(now);
            answers.push(began.elapsed().as_secs_f64() * 1000.0);
            app.drawn(now);
            rows = rows.max(live.len());
            now += Duration::from_secs(2);
            let _ = app.frame(now);
            app.drawn(now);
            let binding = if step % 2 == 0 { "tab" } else { "g" };
            let began = Instant::now();
            key(&mut app, binding, now);
            let _ = app.frame(now);
            keys.push(began.elapsed().as_secs_f64() * 1000.0);
            app.drawn(now);
        }
        // The last key's report answered too, so that nothing is on its
        // way when the panel opens again.
        for (ticket, op) in link.take_sent() {
            if let Op::UsageReport(params) = op {
                let report = year_report(params.range, params.group_by);
                link.give(Inbound::Done(ticket, Ok(OpResult::UsageReport(report))));
            }
        }
        drain(&mut app, now);
        now += Duration::from_secs(2);
        let _ = app.frame(now);
        app.drawn(now);
        key(&mut app, "x", now);
        let _ = app.frame(now);
    }
    keys.sort_by(f64::total_cmp);
    answers.sort_by(f64::total_cmp);
    println!(
        "the usage panel with a year, {} keys and {} answers, a live region of up to {rows} rows:\n  \
         a key to its frame: median {:.3} ms   p99 {:.3} ms   most {:.3} ms\n  \
         an answer to the frame with its rows: median {:.3} ms   p99 {:.3} ms   most {:.3} ms",
        keys.len(),
        answers.len(),
        at(&keys, 50),
        at(&keys, 99),
        keys[keys.len() - 1],
        at(&answers, 50),
        at(&answers, 99),
        answers[answers.len() - 1],
    );
    cortex_testkit::numbers::report("usage_panel.key_frame_p99", at(&keys, 99), "ms");
    cortex_testkit::numbers::report("usage_panel.answer_frame_p99", at(&answers, 99), "ms");
}

/// A key's frame with all four panels open: the plan and the review, the
/// project's entries waiting, and the usage panel's warning line, the input
/// focused; a character typed and taken back by turns, idle and while an
/// answer streams (a piece of it taken in before each key). The budgets are
/// 2 ms idle and 4 ms while streaming, at the 99th percentile.
#[test]
#[ignore = "a measurement: see the module's documentation"]
fn perf_a_keys_frame_with_the_four_panels_open() {
    let runs: usize = std::env::var("CORTEX_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000);
    let start = Instant::now();
    let (mut app, link) = app(start);
    app.set_clock(clock());
    app.resize(119, 60);
    opened(&mut app, &link, start);
    for (ticket, op) in link.take_sent() {
        if let Op::ProjectList(_) = op {
            link.give(Inbound::Done(
                ticket,
                Ok(OpResult::ProjectList(ProjectEntriesResult {
                    entries: entries(),
                    problems: Vec::new(),
                })),
            ));
        }
    }
    let item = |id: &str, text: &str, status| PlanItem {
        id: PlanItemId::from(id),
        text: text.to_owned(),
        status,
    };
    link.give(event(Event::PlanUpdated(PlanUpdated {
        plan: Plan {
            items: vec![
                item("p1", "allow _ between digits", PlanStatus::Done),
                item("p2", "reject a trailing _", PlanStatus::Active),
                item("p3", "add a test for 1_000", PlanStatus::Pending),
                item("p4", "run clippy", PlanStatus::Pending),
            ],
            revision: 1,
        },
        by: cortex_protocol::event::Actor::Agent,
    })));
    let check = |command: &str, outcome| CheckReport {
        command: command.to_owned(),
        cwd: None,
        source: CheckSource::User,
        entry: None,
        outcome: Some(outcome),
        exit_code: Some(0),
        duration_ms: Some(3_400),
        note: None,
        artifact: None,
    };
    link.give(event(Event::Notice(Notice::Verification(
        VerificationNotice {
            message: "The checks.".to_owned(),
            phase: VerificationPhase::Finished,
            round: 1,
            rounds: 1,
            checks: vec![
                check("cargo test -p parser", CheckOutcome::Passed),
                check("cargo clippy -- -D warnings", CheckOutcome::Passed),
            ],
            verdict: Some(Verification::Passed),
        },
    ))));
    link.give(event(Event::UsageUpdated(UsageUpdated {
        usage: session_usage(),
    })));
    drain(&mut app, start);
    let (_, live, _) = app.frame(start);
    let shown = text(&live);
    for first in [
        "◇ Plan",
        "◇ Checks passed",
        "◇ Project",
        "  anthropic fast tier",
    ] {
        assert!(
            shown.iter().any(|row| row.starts_with(first)),
            "{first}: {shown:#?}"
        );
    }
    let measure = |streaming: bool, app: &mut App| {
        if streaming {
            link.give(event(Event::TurnStarted(TurnStarted {})));
            link.give(event(Event::ItemStarted(ItemStarted {
                item_id: ItemId::from("i_s"),
                kind: ItemKind::AssistantMessage,
                tool: None,
                call_id: None,
            })));
            drain(app, start);
        }
        let mut times = Vec::with_capacity(runs);
        for n in 0..runs {
            if streaming {
                // About what 2,000 tokens a second bring between two keys.
                link.give(event(Event::ItemDelta(ItemDelta {
                    item_id: ItemId::from("i_s"),
                    delta: Delta::Text(TextDelta {
                        text: "The scanner stops at the first character that is not a digit, so \
                               1_000 ends after the 1; "
                            .to_owned(),
                    }),
                })));
                drain(app, start);
            }
            let binding = if n % 2 == 0 { "x" } else { "backspace" };
            let began = Instant::now();
            key(app, binding, start);
            let _ = app.frame(start);
            times.push(began.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        times
    };
    let idle = measure(false, &mut app);
    let streaming = measure(true, &mut app);
    for (name, times) in [("idle", &idle), ("streaming", &streaming)] {
        println!(
            "a key's frame with the four panels open, {name}, {} keys: median {:.3} ms   p99 {:.3} ms   most {:.3} ms",
            times.len(),
            at(times, 50),
            at(times, 99),
            times[times.len() - 1]
        );
        cortex_testkit::numbers::report(
            &format!("four_panels.key_frame_{name}_p99"),
            at(times, 99),
            "ms",
        );
    }
}
