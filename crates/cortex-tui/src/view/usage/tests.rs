use cortex_protocol::model::ModelRole;
use cortex_protocol::usage::{Balance, ModelUsage, RequestUsage, TimeTotals, TokenUsage};
use cortex_protocol::{ModelId, ProjectKey, Speed, Timestamp};
use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{self, DARK};
use crate::theme::Theme;

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

/// 2026-11-16T12:00:00Z.
pub(crate) fn now() -> UtcTime {
    UtcTime::parse("2026-11-16T12:00:00Z").expect("a time")
}

fn metered(dollars_cents: u64) -> Cost {
    Cost {
        usd_nanos: dollars_cents * 10_000_000,
        basis: CostBasis::Metered,
    }
}

fn estimate(dollars_cents: u64) -> Cost {
    Cost {
        usd_nanos: dollars_cents * 10_000_000,
        basis: CostBasis::Estimate,
    }
}

pub(crate) fn totals(
    input: u64,
    read: u64,
    write: u64,
    output: u64,
    costs: Vec<Cost>,
) -> UsageTotals {
    UsageTotals {
        tokens: TokenUsage {
            input,
            cache_read: read,
            cache_write: write,
            output,
            ..TokenUsage::default()
        },
        requests: 10,
        costs,
    }
}

fn window(
    measure: LimitMeasure,
    speed: Option<Speed>,
    left: u64,
    limit: u64,
    resets: &str,
) -> LimitWindow {
    LimitWindow {
        measure,
        speed,
        limit: Some(limit),
        remaining: Some(left),
        resets_at: Some(Timestamp::from(resets)),
    }
}

/// Anthropic's limits seen two minutes ago: tokens and requests well
/// within, and the fast tier's output near its end (screen 4).
pub(crate) fn anthropic_limits() -> ProviderLimits {
    ProviderLimits {
        provider: ProviderId::from("anthropic"),
        windows: vec![
            window(LimitMeasure::Tokens, None, 71, 100, "2026-11-16T14:32:00Z"),
            window(
                LimitMeasure::Requests,
                None,
                96,
                100,
                "2026-11-16T12:01:00Z",
            ),
            window(
                LimitMeasure::OutputTokens,
                Some(Speed::Fast),
                18,
                100,
                "2026-11-16T15:00:00Z",
            ),
        ],
        seen_at: Timestamp::from("2026-11-16T11:58:00Z"),
        source: LimitSource::Headers,
        balances: Vec::new(),
        available: None,
        next_epoch: None,
    }
}

/// Venice's, from the refresh ten seconds ago: a balance, and a limit
/// whose reset has passed.
fn venice_limits() -> ProviderLimits {
    ProviderLimits {
        provider: ProviderId::from("venice"),
        windows: vec![window(
            LimitMeasure::Requests,
            None,
            3,
            500,
            "2026-11-16T11:00:00Z",
        )],
        seen_at: Timestamp::from("2026-11-16T11:59:50Z"),
        source: LimitSource::Refresh,
        balances: vec![Balance {
            currency: "USD".to_owned(),
            remaining: Some("110.00".to_owned()),
            limit: None,
            used: None,
        }],
        available: Some(true),
        next_epoch: None,
    }
}

pub(crate) fn session_usage() -> SessionUsage {
    SessionUsage {
        totals: totals(1_210_000, 38_400_000, 2_900_000, 612_000, vec![metered(42)]),
        by_model: vec![
            ModelUsage {
                provider: ProviderId::from("anthropic"),
                model: ModelId::from("opus-5.5"),
                role: ModelRole::Main,
                speed: Speed::Standard,
                totals: totals(1_200_000, 38_000_000, 2_900_000, 600_000, vec![metered(40)]),
            },
            ModelUsage {
                provider: ProviderId::from("anthropic"),
                model: ModelId::from("haiku-5.5"),
                role: ModelRole::Fast,
                speed: Speed::Fast,
                totals: totals(10_000, 400_000, 0, 12_000, vec![metered(2)]),
            },
        ],
        last_request: Some(RequestUsage {
            provider: ProviderId::from("anthropic"),
            model: ModelId::from("opus-5.5"),
            role: ModelRole::Main,
            speed: Speed::Standard,
            speed_requested: None,
            tokens: TokenUsage::default(),
            cost: None,
            catalog_version: None,
            billing: cortex_protocol::usage::Billing::Metered,
            time_to_first_token_ms: Some(1_200),
            duration_ms: Some(9_000),
            raw: None,
        }),
        cache_hit_rate: Some(0.87),
        tokens_per_second: Some(41.0),
        context_tokens: None,
        context: None,
        limits: vec![anthropic_limits()],
    }
}

fn row(
    provider: &str,
    model: Option<&str>,
    project: Option<&str>,
    totals: UsageTotals,
) -> UsageRow {
    UsageRow {
        provider: Some(ProviderId::from(provider)),
        model: model.map(ModelId::from),
        project: project.map(ProjectKey::from),
        project_label: project.map(|label| format!("~/src/{label}")),
        totals,
        ..UsageRow::default()
    }
}

/// Today's report by provider, then model: a metered row, an estimate, a
/// model with no price; a total that mixes them; the time; a note.
pub(crate) fn report(group_by: Vec<UsageGroup>) -> UsageReport {
    let project = group_by.contains(&UsageGroup::Project);
    let at = |name: &'static str| project.then_some(name);
    UsageReport {
        range: UsageRange::Today,
        from: None,
        to: None,
        group_by,
        rows: vec![
            row(
                "anthropic",
                Some("opus-5.5"),
                at("parser"),
                totals(1_210_000, 38_400_000, 2_900_000, 612_000, vec![metered(4_187)]),
            ),
            row(
                "anthropic",
                Some("haiku-5.5"),
                at("parser"),
                totals(440_000, 6_100_000, 520_000, 201_000, vec![estimate(192)]),
            ),
            row(
                "openrouter",
                Some("some-open-model"),
                at("site"),
                totals(800_000, 0, 0, 310_000, Vec::new()),
            ),
        ],
        totals: totals(
            2_450_000,
            44_500_000,
            3_420_000,
            1_123_000,
            vec![metered(4_187), estimate(192)],
        ),
        limits: vec![anthropic_limits()],
        sources: vec!["cortex-harness".to_owned()],
        recording: true,
        time: Some(TimeTotals {
            turns: 12,
            wall_ms: 200_000,
            tools_ms: 50_000,
            approval_ms: 5_000,
            ..TimeTotals::default()
        }),
        tools: Vec::new(),
        zone: Some("Europe/Paris".to_owned()),
        notes: vec![
            "A statistics file that could not be read was set aside, at ~/.local/state/cortex/stats/2026-10.jsonl.bad: the months it held are not counted here."
                .to_owned(),
        ],
    }
}

fn view<'a>(
    session: Option<&'a SessionUsage>,
    report: Option<&'a UsageReport>,
    refreshed: &'a [ProviderLimits],
    said: &'a [String],
    zone: &'a Zone,
) -> UsageView<'a> {
    UsageView {
        session,
        report,
        reading: false,
        range: &UsageRange::Today,
        group: report.map_or(&[][..], |report| &report.group_by),
        refreshed,
        said,
        now: now(),
        zone,
    }
}

#[test]
fn the_usage_in_every_variant() {
    let zone = Zone::utc();
    let session = session_usage();
    let by_model = report(vec![UsageGroup::Provider, UsageGroup::Model]);
    let by_project = report(vec![UsageGroup::Project, UsageGroup::Provider]);
    let refreshed = vec![venice_limits()];
    let said = vec!["openai: the refresh failed: the key was refused (401).".to_owned()];
    let mut quiet = session_usage();
    quiet.limits.clear();
    let states: Vec<(&str, UsageView<'_>, bool)> = vec![
        (
            "opened, its history being read",
            UsageView {
                reading: true,
                ..view(Some(&session), Some(&by_model), &[], &[], &zone)
            },
            true,
        ),
        (
            "today by provider, model; a refresh",
            view(Some(&session), Some(&by_model), &refreshed, &said, &zone),
            true,
        ),
        (
            "today by project, provider",
            view(Some(&session), Some(&by_project), &[], &[], &zone),
            true,
        ),
        (
            "a limit near, opened by itself",
            view(Some(&session), None, &[], &[], &zone),
            false,
        ),
        (
            "nothing near, no history",
            view(Some(&quiet), None, &[], &[], &zone),
            true,
        ),
    ];
    let mut parts = Vec::new();
    for variant in testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        let width = variant.width();
        for (name, state, summoned) in &states {
            let collapsed = usage_collapsed(state, *summoned, width, &look);
            let rows: Vec<Line> = usage_rows(state, Some(1), width, &look)
                .into_iter()
                .map(|row| row.line)
                .collect();
            for (form, lines) in [("collapsed", collapsed), ("focused", rows)] {
                let problems = testing::problems(&lines, width);
                assert!(problems.is_empty(), "{name}, {form}: {problems:?}");
                parts.push((format!("{} — {name}, {form}", variant.title()), lines));
            }
        }
    }
    testing::assert_snapshot("usage", &testing::sections(&parts));
}

fn shown(state: &UsageView<'_>, width: usize) -> Vec<String> {
    usage_rows(state, None, width, &look())
        .into_iter()
        .map(|row| row.line.text().into_owned())
        .collect()
}

#[test]
fn costs_say_their_basis_and_an_unknown_price_is_a_dash() {
    assert_eq!(cost_words(&[metered(4_187)]), "$41.87");
    assert_eq!(cost_words(&[estimate(192)]), "$1.92 est.");
    assert_eq!(
        cost_words(&[metered(4_440), estimate(210)]),
        "$44.40 metered + $2.10 est."
    );
    assert_eq!(cost_words(&[]), "—");
    assert_eq!(
        cost_words(&[Cost {
            usd_nanos: 4_200_000,
            basis: CostBasis::Metered
        }]),
        "$0.0042"
    );
    let zone = Zone::utc();
    let by_model = report(vec![UsageGroup::Provider, UsageGroup::Model]);
    let rows = shown(&view(None, Some(&by_model), &[], &[], &zone), 119);
    let row = |name: &str| {
        rows.iter()
            .find(|row| row.contains(name))
            .cloned()
            .unwrap_or_else(|| panic!("{name}: {rows:#?}"))
    };
    assert!(row("haiku-5.5").ends_with("$1.92 est."), "{rows:#?}");
    assert!(row("some-open-model").ends_with("—"), "{rows:#?}");
    assert!(
        row("total").ends_with("$41.87 metered + $1.92 est."),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.trim() == ESTIMATE), "{rows:#?}");
}

#[test]
fn the_columns_drop_as_the_window_narrows() {
    let zone = Zone::utc();
    let by_model = report(vec![UsageGroup::Provider, UsageGroup::Model]);
    let state = view(None, Some(&by_model), &[], &[], &zone);
    let heading = |width: usize| -> String {
        shown(&state, width)
            .into_iter()
            .find(|row| row.starts_with("  Today"))
            .unwrap_or_default()
    };
    let wide = heading(119);
    for column in ["in", "cache read", "cache write", "out", "cost"] {
        assert!(wide.contains(column), "{wide}");
    }
    // Narrower: the cache writes go first, then the reads, then the input.
    let mut dropped = Vec::new();
    for width in (60..119).rev() {
        let row = heading(width);
        for column in ["cache write", "cache read", "  in  "] {
            if !row.contains(column) && !dropped.contains(&column) {
                dropped.push(column);
            }
        }
    }
    // The input stays while the name keeps a third of the row.
    assert_eq!(dropped, ["cache write", "cache read"]);
    // Under 60 columns, the name, the output and the cost.
    let narrow = heading(59);
    assert!(
        narrow.contains("out") && narrow.contains("cost"),
        "{narrow}"
    );
    assert!(!narrow.contains("  in  "), "{narrow}");
}

#[test]
fn limits_say_their_age_a_reset_passed_a_refresh_and_not_available() {
    let zone = Zone::utc();
    let session = session_usage();
    let by_model = report(vec![UsageGroup::Provider, UsageGroup::Model]);
    let refreshed = vec![venice_limits()];
    let rows = shown(
        &view(Some(&session), Some(&by_model), &refreshed, &[], &zone),
        200,
    );
    let has = |text: &str| rows.iter().any(|row| row.contains(text));
    assert!(
        has(
            "anthropic  tokens 71% left · resets 14:32 · requests 96% left · resets 12:01 · as of 2 min ago"
        ),
        "{rows:#?}"
    );
    assert!(
        has("anthropic fast tier  output tokens 18% left · resets 15:00"),
        "{rows:#?}"
    );
    assert!(
        has(
            "venice  requests reset since; unknown until the next request · from the refresh, as of 10 s ago"
        ),
        "{rows:#?}"
    );
    assert!(has("venice  balance 110.00 USD left"), "{rows:#?}");
    assert!(has("openrouter  not available"), "{rows:#?}");
}

#[test]
fn a_limit_with_less_than_a_fifth_left_is_the_warning_line() {
    let zone = Zone::utc();
    let session = session_usage();
    let state = view(Some(&session), None, &[], &[], &zone);
    assert_eq!(
        near(&state).as_deref(),
        Some("anthropic fast tier: 18% of the output token limit left, resets 15:00")
    );
    assert_eq!(
        usage_collapsed(&state, false, 80, &look())
            .iter()
            .map(|line| line.text().into_owned())
            .collect::<Vec<_>>(),
        ["  anthropic fast tier: 18% of the output token limit left, resets 15:00"]
    );
    // A fifth exactly is not near; past its reset, nothing is known.
    let mut fifth = session_usage();
    fifth.limits[0].windows[2].remaining = Some(20);
    assert_eq!(near(&view(Some(&fifth), None, &[], &[], &zone)), None);
    let mut passed = session_usage();
    passed.limits[0].windows[2].resets_at = Some(Timestamp::from("2026-11-16T11:00:00Z"));
    assert_eq!(near(&view(Some(&passed), None, &[], &[], &zone)), None);
    // In another zone, the reset is said in its time of day.
    let paris = Zone::fixed(3_600);
    assert!(
        near(&view(Some(&session), None, &[], &[], &paris))
            .is_some_and(|line| line.ends_with("resets 16:00"))
    );
}

#[test]
fn an_age_is_said_in_words() {
    let now = now();
    assert_eq!(ago("2026-11-16T11:59:58Z", now), "just now");
    assert_eq!(ago("2026-11-16T11:59:50Z", now), "10 s ago");
    assert_eq!(ago("2026-11-16T11:58:00Z", now), "2 min ago");
    assert_eq!(ago("2026-11-16T09:00:00Z", now), "3 h ago");
    assert_eq!(ago("2026-11-14T09:00:00Z", now), "2 d ago");
    assert_eq!(ago("soon", now), "an unknown time");
}

#[test]
fn hostile_names_labels_currencies_notes_and_errors_are_drawn_visible() {
    const HOSTILE: [&str; 5] = [
        "\u{1b}]8;;http://x\u{7}evil\u{1b}]8;;\u{7}",
        "a\u{202e}b",
        "zero\u{200b}width",
        "line\nbreak",
        "\u{9b}2J\u{e0041}",
    ];
    let look = look();
    let zone = Zone::utc();
    for hostile in HOSTILE {
        let mut session = session_usage();
        session.by_model[0].provider = ProviderId::from(hostile);
        session.by_model[0].model = ModelId::from(hostile);
        session.by_model[1].speed = Speed::from_wire(hostile);
        session.limits[0].provider = ProviderId::from(hostile);
        session.limits[0].windows[0].speed = Some(Speed::from_wire(hostile));
        session.limits[0].windows[1].measure = LimitMeasure::from_wire(hostile);
        session.limits[0].seen_at = Timestamp::from(hostile);
        session.limits[0].balances = vec![Balance {
            currency: hostile.to_owned(),
            remaining: Some(hostile.to_owned()),
            limit: Some(hostile.to_owned()),
            used: Some(hostile.to_owned()),
        }];
        let mut report = report(vec![UsageGroup::Project, UsageGroup::Provider]);
        for row in &mut report.rows {
            row.provider = Some(ProviderId::from(hostile));
            row.project_label = Some(hostile.to_owned());
            row.source = Some(hostile.to_owned());
        }
        report.zone = Some(hostile.to_owned());
        report.notes = vec![hostile.to_owned()];
        let said = vec![hostile.to_owned()];
        for width in [1, 10, 40, 59, 120] {
            let state = view(Some(&session), Some(&report), &[], &said, &zone);
            let mut lines: Vec<Line> = usage_rows(&state, Some(0), width, &look)
                .into_iter()
                .map(|row| row.line)
                .collect();
            lines.extend(usage_collapsed(&state, true, width, &look));
            let problems = testing::problems(&lines, width);
            assert!(problems.is_empty(), "{hostile:?} at {width}: {problems:#?}");
        }
    }
}
