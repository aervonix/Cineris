//! The usage panel in the application (step 24): `/usage` and its one
//! round trip, the warning line that opens it by itself, the range, the
//! grouping, the refresh, and hostile text in the frame.

use cortex_base::{FakeClock, SharedClock};
use cortex_protocol::event::{ConfigChanged, UsageUpdated};
use cortex_protocol::methods::{
    ConfigGetParams, ConfigGetResult, RefreshOutcome, UsageRefreshParams, UsageRefreshResult,
    UsageReportParams,
};
use cortex_protocol::usage::{SessionUsage, UsageGroup, UsageRange, UsageReport};

use super::*;
use crate::app::panels::usage::NO_REFRESH;
use crate::app::ready::QUIET;
use crate::focus::Target;
use crate::view::usage::tests::{anthropic_limits, report, session_usage};

/// The time the panel reads: 2026-11-16T12:00:00Z, said in UTC.
fn clock() -> SharedClock {
    Arc::new(FakeClock::starting_at("2026-11-16T12:00:00Z").expect("a time"))
}

fn usage_id(app: &App) -> u64 {
    app.panels().find("usage").expect("the usage panel")
}

fn usage_updated(usage: SessionUsage) -> Inbound {
    event(Event::UsageUpdated(UsageUpdated { usage }))
}

/// A session open long before `at`, `rows` high, its usage given, and a
/// frame drawn.
fn with_usage(at: Instant, rows: usize, usage: SessionUsage) -> (App, Scripted) {
    let long_before = at - Duration::from_secs(10);
    let (mut app, link) = app(long_before);
    app.set_clock(clock());
    app.resize(119, rows);
    opened(&mut app, &link, long_before);
    link.give(usage_updated(usage));
    drain(&mut app, long_before);
    let _ = app.frame(long_before);
    app.drawn(long_before);
    link.take_sent();
    (app, link)
}

fn press(app: &mut App, binding: &str, at: Instant) {
    key(app, binding, at);
    let _ = app.frame(at);
    app.drawn(at);
}

fn live_text(app: &mut App, at: Instant) -> Vec<String> {
    let (_, live, _) = app.frame(at);
    app.drawn(at);
    text(&live)
}

/// What was sent, taken: the reports asked for, the config asked for, the
/// refreshes, and anything else.
#[derive(Debug, Default)]
struct Sent {
    reports: Vec<(Ticket, UsageReportParams)>,
    configs: Vec<(Ticket, ConfigGetParams)>,
    refreshes: Vec<(Ticket, UsageRefreshParams)>,
    other: Vec<Op>,
}

fn sent(link: &Scripted) -> Sent {
    let mut sent = Sent::default();
    for (ticket, op) in link.take_sent() {
        match op {
            Op::UsageReport(params) => sent.reports.push((ticket, params)),
            Op::ConfigGet(params) => sent.configs.push((ticket, params)),
            Op::UsageRefresh(params) => sent.refreshes.push((ticket, params)),
            other => sent.other.push(other),
        }
    }
    sent
}

/// `/usage` typed long before `at` and its `Enter` at `at`, with the frame
/// for it: what it sent. The time the panel is ready.
fn usage_focused(app: &mut App, link: &Scripted, at: Instant) -> (Sent, Instant) {
    type_text(app, "/usage", at - Duration::from_secs(5));
    key(app, "esc", at - Duration::from_secs(5));
    press(app, "enter", at);
    assert_eq!(app.input().focus().focused(), Target::Panel(usage_id(app)));
    (sent(link), at + QUIET)
}

/// Answers the report asked for under `ticket` with `report`, and the
/// config with `refresh` as `usage.refresh`.
fn answer(
    app: &mut App,
    link: &Scripted,
    sent: &Sent,
    report: &UsageReport,
    refresh: &[&str],
    at: Instant,
) {
    for (ticket, params) in &sent.reports {
        let mut answered = report.clone();
        answered.range = params.range.clone();
        answered.group_by.clone_from(&params.group_by);
        link.give(Inbound::Done(*ticket, Ok(OpResult::UsageReport(answered))));
    }
    for (ticket, _) in &sent.configs {
        let mut config = serde_json::Map::new();
        config.insert(
            "usage".to_owned(),
            serde_json::json!({ "refresh": refresh }),
        );
        link.give(Inbound::Done(
            *ticket,
            Ok(OpResult::ConfigGet(ConfigGetResult {
                config,
                origins: Default::default(),
                ..Default::default()
            })),
        ));
    }
    drain(app, at);
}

/// `/usage` opens the panel focused with its history `reading…`, sends
/// one `usage/report` and one `config/get` and nothing else, and the frame
/// after the answer has the rows: the path the budget is measured on, in
/// a small year (a second round trip, or a read of anything else, fails
/// it).
#[test]
fn slash_usage_asks_once_and_the_frame_after_the_answer_has_the_rows() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 60, session_usage());
    let (sent, ready) = usage_focused(&mut app, &link, now);
    assert_eq!(
        sent.reports
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>(),
        [UsageReportParams {
            range: UsageRange::Today,
            group_by: vec![UsageGroup::Provider, UsageGroup::Model],
            session_id: None,
            project: None,
        }]
    );
    assert_eq!(
        sent.configs
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>(),
        [ConfigGetParams {
            session_id: None,
            keys: vec!["usage.refresh".to_owned()],
        }]
    );
    // Beside the look at the configuration a key makes (the hot reload's),
    // nothing.
    assert!(
        sent.refreshes.is_empty()
            && sent
                .other
                .iter()
                .all(|op| matches!(op, Op::ConfigReload(_))),
        "{sent:?}"
    );
    let live = live_text(&mut app, now);
    assert!(
        live.iter()
            .any(|row| row.starts_with("◇ Usage  this session $0.42 · today · reading…")),
        "{live:#?}"
    );
    answer(
        &mut app,
        &link,
        &sent,
        &report(vec![UsageGroup::Provider, UsageGroup::Model]),
        &["anthropic"],
        ready,
    );
    let live = live_text(&mut app, ready);
    for row in [
        "  Today · by provider, model",
        "    opus-5.5",
        "  openrouter  not available",
    ] {
        assert!(
            live.iter().any(|shown| shown.starts_with(row)),
            "{row}: {live:#?}"
        );
    }
    let more = self::sent(&link);
    assert!(
        more.reports.is_empty() && more.configs.is_empty() && more.refreshes.is_empty(),
        "nothing more is asked: {more:?}"
    );
    let hint = live.last().cloned().unwrap_or_default();
    assert!(
        hint.contains("tab shows this week")
            && hint.contains("g groups by project")
            && hint.contains("r refreshes the limits"),
        "{hint}"
    );
}

#[test]
fn a_limit_near_opens_it_by_itself_as_the_warning_line_and_its_end_closes_it() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 40, SessionUsage::default());
    let usage = usage_id(&app);
    assert!(!app.panels().is_open(usage));
    link.give(usage_updated(session_usage()));
    drain(&mut app, now);
    assert!(app.panels().is_open(usage));
    assert!(app.input().focus().on_input(), "it takes no focus");
    let live = live_text(&mut app, now);
    let at = live
        .iter()
        .position(|row| {
            row == "  anthropic fast tier: 18% of the output token limit left, resets 15:00"
        })
        .unwrap_or_else(|| panic!("{live:#?}"));
    assert_eq!(live[at + 1], "", "a blank row after the block");
    // A fifth or more left: it goes.
    let mut eased = session_usage();
    eased.limits[0].windows[2].remaining = Some(40);
    link.give(usage_updated(eased));
    drain(&mut app, now);
    assert!(!app.panels().is_open(usage));
}

#[test]
fn opened_with_slash_usage_it_stays_when_no_limit_is_near() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 60, session_usage());
    let _ = usage_focused(&mut app, &link, now);
    let mut eased = session_usage();
    eased.limits = vec![anthropic_limits()];
    eased.limits[0].windows[2].remaining = Some(40);
    link.give(usage_updated(eased));
    drain(&mut app, now);
    assert!(app.panels().is_open(usage_id(&app)));
}

#[test]
fn tab_and_g_ask_for_the_next_range_and_grouping() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 60, session_usage());
    let (asked, ready) = usage_focused(&mut app, &link, now);
    answer(
        &mut app,
        &link,
        &asked,
        &report(Vec::new()),
        &["anthropic"],
        now,
    );
    let _ = live_text(&mut app, ready);
    press(&mut app, "tab", ready);
    let tab = sent(&link);
    assert_eq!(
        tab.reports
            .iter()
            .map(|(_, p)| p.range.clone())
            .collect::<Vec<_>>(),
        [UsageRange::Week]
    );
    let live = live_text(&mut app, ready);
    assert!(
        live.iter()
            .any(|row| row.starts_with("  This week · by provider, model")
                && row.ends_with("reading…")),
        "{live:#?}"
    );
    answer(&mut app, &link, &tab, &report(Vec::new()), &[], ready);
    let later = ready + Duration::from_secs(2);
    let _ = live_text(&mut app, later);
    press(&mut app, "g", later);
    let g = sent(&link);
    assert_eq!(
        g.reports
            .iter()
            .map(|(_, p)| (p.range.clone(), p.group_by.clone()))
            .collect::<Vec<_>>(),
        [(UsageRange::Week, vec![UsageGroup::Project])]
    );
}

#[test]
fn r_asks_one_refresh_at_a_time_and_with_none_turned_on_says_so_and_sends_nothing() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 60, session_usage());
    let (asked, ready) = usage_focused(&mut app, &link, now);
    answer(
        &mut app,
        &link,
        &asked,
        &report(Vec::new()),
        &["anthropic"],
        now,
    );
    let _ = live_text(&mut app, ready);
    press(&mut app, "r", ready);
    let first = sent(&link);
    assert_eq!(
        first
            .refreshes
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>(),
        [UsageRefreshParams::default()]
    );
    // A second `r` while the first is out sends nothing, and says why.
    let again = ready + Duration::from_secs(2);
    let live = live_text(&mut app, again);
    press(&mut app, "r", again);
    assert!(sent(&link).refreshes.is_empty(), "{live:#?}");
    let hint = live_text(&mut app, again)
        .last()
        .cloned()
        .unwrap_or_default();
    assert!(
        hint.contains("the last change is still being made"),
        "{hint}"
    );
    let (ticket, _) = first.refreshes[0];
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::UsageRefresh(UsageRefreshResult {
            providers: vec![RefreshOutcome {
                provider: ProviderId::from("anthropic"),
                limits: None,
                error: Some(ErrorData::new(
                    ErrorKind::Usage,
                    "refresh_failed",
                    "the key was refused (401)",
                )),
            }],
        })),
    ));
    drain(&mut app, again);
    let live = live_text(&mut app, again);
    assert!(
        live.iter()
            .any(|row| row.contains("anthropic: the refresh failed: the key was refused (401)")),
        "{live:#?}"
    );
    // With no provider in `usage.refresh`: said, and nothing sent.
    let (mut app, link) = with_usage(now, 60, session_usage());
    let (asked, ready) = usage_focused(&mut app, &link, now);
    answer(&mut app, &link, &asked, &report(Vec::new()), &[], now);
    let live = live_text(&mut app, ready);
    let hint = live.last().cloned().unwrap_or_default();
    assert!(!hint.contains("refreshes"), "{hint}");
    key(&mut app, "r", ready);
    assert!(sent(&link).refreshes.is_empty());
    let mut scrollback = Vec::new();
    let _ = screen(&mut app, ready, &mut scrollback);
    let said = scrollback
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(said.contains(NO_REFRESH), "{said}");
    // A reload that changes it: `r` asks the core again.
    link.give(event(Event::ConfigChanged(ConfigChanged {
        applied: vec!["usage.refresh".to_owned()],
        ..ConfigChanged::default()
    })));
    drain(&mut app, ready);
    let later = ready + Duration::from_secs(2);
    let _ = live_text(&mut app, later);
    press(&mut app, "r", later);
    assert_eq!(sent(&link).refreshes.len(), 1);
}

#[test]
fn hostile_usage_is_drawn_visible_in_the_frame() {
    let now = Instant::now() + Duration::from_secs(20);
    let hostile = "\u{1b}[2J\u{202e}gone\u{200b}\u{9b}31m\u{e0041}";
    let mut usage = session_usage();
    usage.by_model[0].model = cortex_protocol::ModelId::from(hostile);
    usage.limits[0].provider = ProviderId::from(hostile);
    let (mut app, link) = with_usage(now, 60, usage);
    let (asked, ready) = usage_focused(&mut app, &link, now);
    let mut history = report(vec![UsageGroup::Project, UsageGroup::Provider]);
    for row in &mut history.rows {
        row.project_label = Some(hostile.to_owned());
    }
    history.notes = vec![hostile.to_owned()];
    answer(&mut app, &link, &asked, &history, &[], ready);
    for columns in [20, 60, 120] {
        app.resize(columns - 1, 60);
        let (_, live, _) = app.frame(ready);
        let problems = crate::testing::problems(&live, columns - 1);
        assert!(problems.is_empty(), "at {columns}: {problems:#?}");
        if columns < 120 {
            continue;
        }
        let shown = text(&live);
        assert!(
            shown.iter().any(|row| row.starts_with("◇ Usage")),
            "the panel is drawn: {shown:#?}"
        );
        let escaped = shown
            .iter()
            .filter(|row| row.contains("\\u{202e}gone\\u{200b}"))
            .count();
        // The warning line, the session's model, the project's heading, the
        // limit's provider and the note.
        assert!(escaped >= 4, "{escaped}: {shown:#?}");
    }
}

/// A report the core refuses (statistics a newer build wrote) is said in
/// the panel where its rows would be, as `cortex stats` says it, and the
/// panel stops `reading…`.
#[test]
fn a_refused_report_is_said_in_the_panel_and_reading_ends() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 60, session_usage());
    let (asked, ready) = usage_focused(&mut app, &link, now);
    let sentence = "The statistics' aggregates were written by a newer build of Cortex-Harness \
                    (version 9), which this one does not read or change: the newer build shows them.";
    for (ticket, _) in &asked.reports {
        link.give(Inbound::Done(
            *ticket,
            Err(ErrorData::new(ErrorKind::Usage, "stats_newer", sentence)),
        ));
    }
    drain(&mut app, ready);
    let live = live_text(&mut app, ready);
    let joined = live
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains(sentence), "{live:#?}");
    assert!(!joined.contains("reading…"), "{live:#?}");
}

/// A limit that opened the panel, whose reset has passed: with the next
/// update it goes, and with no update at all it goes with the next frame,
/// the focus ring not reaching it.
#[test]
fn a_limit_past_its_reset_closes_the_panel_it_opened() {
    let now = Instant::now() + Duration::from_secs(20);
    let wall = FakeClock::starting_at("2026-11-16T12:00:00Z").expect("a time");
    let (mut app, link) = with_usage(now, 40, SessionUsage::default());
    app.set_clock(Arc::new(wall.clone()));
    let usage = usage_id(&app);
    link.give(usage_updated(session_usage()));
    drain(&mut app, now);
    assert!(app.panels().is_open(usage), "the fast tier is near");
    // The fast tier resets at 15:00; at 15:01 nothing is known of it.
    wall.advance(Duration::from_secs(3 * 3_600 + 60));
    link.give(usage_updated(session_usage()));
    drain(&mut app, now);
    assert!(!app.panels().is_open(usage), "a reset passed is not near");
    // Near again, and then the reset passes with no update: the next
    // frame closes it, and `Ctrl+P` does not land on it.
    let wall = FakeClock::starting_at("2026-11-16T12:00:00Z").expect("a time");
    let (mut app, link) = with_usage(now, 40, SessionUsage::default());
    app.set_clock(Arc::new(wall.clone()));
    link.give(usage_updated(session_usage()));
    drain(&mut app, now);
    let _ = live_text(&mut app, now);
    assert!(app.panels().is_open(usage));
    wall.advance(Duration::from_secs(3 * 3_600 + 60));
    let live = live_text(&mut app, now);
    assert!(!app.panels().is_open(usage), "{live:#?}");
    key(&mut app, "ctrl+p", now);
    assert_ne!(app.input().focus().focused(), Target::Panel(usage));
}

/// `tab` and `g` change the panel's own view: a `g` straight after the
/// range's answer is taken, not typed into the prompt.
#[test]
fn a_key_straight_after_the_views_change_is_taken() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_usage(now, 60, session_usage());
    let (asked, ready) = usage_focused(&mut app, &link, now);
    answer(
        &mut app,
        &link,
        &asked,
        &report(Vec::new()),
        &["anthropic"],
        now,
    );
    // The focus on a row of the history, which `tab` changes.
    for _ in 0..6 {
        key(&mut app, "down", ready);
    }
    let _ = live_text(&mut app, ready);
    press(&mut app, "tab", ready);
    let tab = sent(&link);
    answer(&mut app, &link, &tab, &report(Vec::new()), &[], ready);
    let _ = live_text(&mut app, ready);
    let soon = ready + Duration::from_millis(50);
    press(&mut app, "g", soon);
    assert_eq!(
        sent(&link).reports.len(),
        1,
        "the grouping asked for at once"
    );
    assert_eq!(app.input().focus().focused(), Target::Panel(usage_id(&app)));
}
