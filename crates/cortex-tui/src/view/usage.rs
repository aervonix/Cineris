//! The usage panel's rows (`docs/architecture/09-tui.md`, "Panels";
//! `docs/architecture/18-usage.md`; screen 4's last panel row): what this
//! session used, the history for a range, and each provider's limits.
//!
//! ```text
//! ◇ Usage  this session $0.42 · today
//!   This session                    in  cache read  cache write    out   cost
//!     anthropic opus-5.5          1.2M       38.4M         2.9M   612k  $0.42
//!     cache hit 87% · 41 tokens/s · first token 1.2s
//!   Today · by provider, model · days in Europe/Paris
//!   anthropic
//!     opus-5.5                    1.2M       38.4M         2.9M   612k  $41.87
//!   total                                                $44.40 metered + $2.10 est.
//!   est.: what this would have cost at API prices; a subscription is not billed this way
//!   anthropic  tokens 71% left · resets 14:32 · as of 2 min ago
//!   openrouter  not available
//! ```
//!
//! **The table's columns drop as the window narrows:** cache writes first,
//! then cache reads, then input; output and cost stay, and under 60
//! columns the name, output and cost are all there is.
//!
//! **Each cost says its basis** (18-usage, "Where our own numbers come
//! from"): metered plain, an estimate marked `est.` with the legend once,
//! a recorded cost marked so, a total that mixes them shown apart, and an
//! unknown price as `—`. **Each limit says its age** (`as of 2 min ago`),
//! `reset since; unknown until the next request` once its reset has
//! passed, where it came from when a refresh gave it, and a provider that
//! reports none says `not available`.
//!
//! Collapsed, the panel is its first row when `/usage` opened it, and the
//! warning line when a provider's limit is near: less than a fifth left
//! (`anthropic fast tier: 18% of the output token limit left, resets
//! 15:00`).
//!
//! Everything here came from outside (a provider's and a model's name, a
//! project's label and key, a source, a currency and its amounts, a tool's
//! name, a note, a refresh's error) and is made one line and visible before
//! it is laid out.

use cortex_base::{UtcTime, Zone};
use cortex_protocol::ProviderId;
use cortex_protocol::usage::{
    Cost, CostBasis, LimitMeasure, LimitSource, LimitWindow, ProviderLimits, SessionUsage,
    UsageGroup, UsageRange, UsageReport, UsageRow, UsageTotals,
};
use cortex_term::{Line, Span};

use super::blocks::count;
use super::panel::{self, PanelRow};
use super::{Look, duration, one_line, precise};
use crate::layout::{cut, spread, wrap};
use crate::theme::Token;

/// Under this many columns the table is the name, output and cost alone.
pub const NARROW: usize = 60;

/// What the legend says of an estimate, once.
pub const ESTIMATE: &str =
    "est.: what this would have cost at API prices; a subscription is not billed this way";

/// The usage panel's state, as its view draws it.
#[derive(Debug, Clone, Copy)]
pub struct UsageView<'a> {
    /// The session's live usage (`usage.updated`).
    pub session: Option<&'a SessionUsage>,
    /// The history for the range chosen (`usage/report`).
    pub report: Option<&'a UsageReport>,
    /// Whether the report for the range chosen is being read.
    pub reading: bool,
    /// The range chosen.
    pub range: &'a UsageRange,
    /// The grouping chosen, outermost first.
    pub group: &'a [UsageGroup],
    /// The limits a refresh gave since, newer than the others.
    pub refreshed: &'a [ProviderLimits],
    /// What the refresh said, a sentence each: a failure, or that no
    /// provider has it turned on.
    pub said: &'a [String],
    /// The time now, and the zone a time of day is said in.
    pub now: UtcTime,
    /// The zone a reset's time of day is said in.
    pub zone: &'a Zone,
}

/// One part of the panel: an item, which `Up` and `Down` move over, drawn
/// in one row or more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// `This session` and the columns' names.
    SessionHeading,
    /// One of the session's models.
    SessionRow(usize),
    /// The cache hit rate, the speed of the last request.
    SessionStats,
    /// The range, the grouping and the zone, and the columns' names.
    HistoryHeading,
    /// The outer group's name, over its rows.
    Group(usize),
    /// One row of the report.
    HistoryRow(usize),
    /// The report's total.
    Total,
    /// Where the range's turns' time went.
    Time,
    /// What a cost's basis means.
    Legend(&'static str),
    /// A provider's limits, by its place in [`limits`].
    Limits(usize),
    /// A provider that reports none.
    NotAvailable(ProviderId),
    /// A sentence from the report or the refresh.
    Said(String),
}

/// The ranges `tab` goes through, in order.
pub const RANGES: [UsageRange; 4] = [
    UsageRange::Today,
    UsageRange::Week,
    UsageRange::Month,
    UsageRange::All,
];

/// A range in words.
#[must_use]
pub fn range_words(range: &UsageRange) -> String {
    match range {
        UsageRange::Session => "this session".to_owned(),
        UsageRange::Today => "today".to_owned(),
        UsageRange::Week => "this week".to_owned(),
        UsageRange::Month => "this month".to_owned(),
        UsageRange::All => "all time".to_owned(),
        other => one_line(other.as_str()),
    }
}

/// A grouping in words: `provider, model`.
#[must_use]
pub fn group_words(group: &[UsageGroup]) -> String {
    if group.is_empty() {
        return "provider, model".to_owned();
    }
    group
        .iter()
        .map(|group| one_line(group.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The panel's parts, in order.
#[must_use]
pub fn parts(view: &UsageView<'_>) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(session) = view.session.filter(|session| !session.by_model.is_empty()) {
        parts.push(Part::SessionHeading);
        parts.extend((0..session.by_model.len()).map(Part::SessionRow));
        if session.cache_hit_rate.is_some()
            || session.tokens_per_second.is_some()
            || session
                .last_request
                .as_ref()
                .is_some_and(|last| last.time_to_first_token_ms.is_some())
        {
            parts.push(Part::SessionStats);
        }
    }
    parts.push(Part::HistoryHeading);
    if let Some(report) = view.report.filter(|_| !view.reading) {
        if !report.recording {
            parts.push(Part::Said(
                "Statistics are off: only this session has figures.".to_owned(),
            ));
        }
        let outer = report.group_by.len() > 1;
        let mut last: Option<String> = None;
        for (at, row) in report.rows.iter().enumerate() {
            if outer {
                let name = cell_name(row, &report.group_by[..1]);
                if last.as_ref() != Some(&name) {
                    parts.push(Part::Group(at));
                    last = Some(name);
                }
            }
            parts.push(Part::HistoryRow(at));
        }
        parts.push(Part::Total);
        if report.time.as_ref().is_some_and(|time| time.turns > 0) {
            parts.push(Part::Time);
        }
        for note in &report.notes {
            parts.push(Part::Said(note.clone()));
        }
    }
    if uses(view, &CostBasis::Estimate) {
        parts.push(Part::Legend(ESTIMATE));
    }
    let limits = limits(view);
    parts.extend((0..limits.len()).map(Part::Limits));
    for provider in providers(view) {
        if !limits.iter().any(|limits| limits.provider == provider) {
            parts.push(Part::NotAvailable(provider));
        }
    }
    parts.extend(view.said.iter().cloned().map(Part::Said));
    parts
}

/// Whether any cost shown has `basis`.
fn uses(view: &UsageView<'_>, basis: &CostBasis) -> bool {
    let has = |totals: &UsageTotals| totals.costs.iter().any(|cost| &cost.basis == basis);
    view.session
        .is_some_and(|session| session.by_model.iter().any(|model| has(&model.totals)))
        || view.report.filter(|_| !view.reading).is_some_and(|report| {
            has(&report.totals) || report.rows.iter().any(|row| has(&row.totals))
        })
}

/// Each provider's latest limits: the refresh's, the report's and the
/// session's, the one seen last of each provider.
#[must_use]
pub fn limits(view: &UsageView<'_>) -> Vec<ProviderLimits> {
    let mut latest: Vec<ProviderLimits> = Vec::new();
    let all = view
        .refreshed
        .iter()
        .chain(view.report.map_or(&[][..], |report| &report.limits))
        .chain(view.session.map_or(&[][..], |session| &session.limits));
    for limits in all {
        let seen = |limits: &ProviderLimits| UtcTime::parse(limits.seen_at.as_str()).ok();
        match latest
            .iter_mut()
            .find(|had| had.provider == limits.provider)
        {
            Some(had) if seen(limits) > seen(had) => *had = limits.clone(),
            Some(_) => {}
            None => latest.push(limits.clone()),
        }
    }
    latest
}

/// The providers the session or the report used.
fn providers(view: &UsageView<'_>) -> Vec<ProviderId> {
    let mut providers: Vec<ProviderId> = Vec::new();
    let session = view
        .session
        .into_iter()
        .flat_map(|session| session.by_model.iter().map(|model| &model.provider));
    let report = view
        .report
        .filter(|_| !view.reading)
        .into_iter()
        .flat_map(|report| report.rows.iter().filter_map(|row| row.provider.as_ref()));
    for provider in session.chain(report) {
        if !providers.contains(provider) {
            providers.push(provider.clone());
        }
    }
    providers
}

/// The panel's first row: `◇ Usage  this session $0.42 · today`.
#[must_use]
pub fn usage_header(view: &UsageView<'_>, width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let mut rest = vec![Span::plain("  ")];
    if let Some(cost) = view
        .session
        .map(|session| cost_words(&session.totals.costs))
    {
        rest.push(Span::new(format!("this session {cost}"), muted));
        rest.push(Span::new(format!(" {} ", look.glyphs.separator), muted));
    }
    rest.push(Span::new(range_words(view.range), muted));
    if view.reading {
        rest.push(Span::new(
            format!(" {} reading{}", look.glyphs.separator, look.glyphs.ellipsis),
            muted,
        ));
    }
    panel::header("Usage", &rest, width, look)
}

/// The panel collapsed: its first row when `summoned`, and the warning
/// line when a limit is near.
#[must_use]
pub fn usage_collapsed(
    view: &UsageView<'_>,
    summoned: bool,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let mut lines = Vec::new();
    if summoned {
        lines.push(usage_header(view, width, look));
    }
    if let Some(near) = near(view) {
        lines.push(cut(
            &Line::plain("  ").with(Span::new(near, look.theme.style(Token::Warning))),
            width,
            look.glyphs.ellipsis,
        ));
    }
    lines
}

/// The panel drawn whole: its first row, then each part, the `focused`
/// one drawn as the selection.
#[must_use]
pub fn usage_rows(
    view: &UsageView<'_>,
    focused: Option<usize>,
    width: usize,
    look: &Look,
) -> Vec<PanelRow> {
    let mut rows = vec![PanelRow::plain(usage_header(view, width, look))];
    if let Some(near) = near(view) {
        rows.push(PanelRow::plain(cut(
            &Line::plain("  ").with(Span::new(near, look.theme.style(Token::Warning))),
            width,
            look.glyphs.ellipsis,
        )));
    }
    let table = Table::new(view, width);
    let limits = limits(view);
    for (index, part) in parts(view).iter().enumerate() {
        let focus = focused == Some(index);
        for line in part_lines(view, part, &table, &limits, focus, width, look) {
            rows.push(PanelRow::of(index, line));
        }
    }
    rows
}

/// The rows of one part.
fn part_lines(
    view: &UsageView<'_>,
    part: &Part,
    table: &Table,
    limits: &[ProviderLimits],
    focused: bool,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let name_token = if focused {
        Token::Selected
    } else {
        Token::Text
    };
    let indented = |text: String, token: Token| {
        cut(
            &Line::plain("  ").with(Span::new(
                text,
                theme.style(if focused { Token::Selected } else { token }),
            )),
            width,
            look.glyphs.ellipsis,
        )
    };
    match part {
        Part::SessionHeading => vec![table.heading("This session", focused, look)],
        Part::SessionRow(at) => {
            let Some(model) = view.session.and_then(|session| session.by_model.get(*at)) else {
                return Vec::new();
            };
            let mut name = format!(
                "{} {}",
                one_line(model.provider.as_str()),
                one_line(model.model.as_str())
            );
            if !model.speed.is_standard() {
                name.push_str(&format!(" {}", one_line(model.speed.as_str())));
            }
            vec![table.row(&name, 1, &model.totals, name_token, look)]
        }
        Part::SessionStats => {
            let Some(session) = view.session else {
                return Vec::new();
            };
            let mut said = Vec::new();
            if let Some(rate) = session.cache_hit_rate {
                said.push(format!(
                    "cache hit {:.0}%",
                    (rate * 100.0).clamp(0.0, 100.0)
                ));
            }
            if let Some(speed) = session.tokens_per_second {
                said.push(format!("{speed:.0} tokens/s"));
            }
            if let Some(last) = &session.last_request {
                if let Some(ms) = last.time_to_first_token_ms {
                    said.push(format!(
                        "first token {}",
                        precise(std::time::Duration::from_millis(ms))
                    ));
                }
                if !last.speed.is_standard() {
                    said.push(format!("{} speed", one_line(last.speed.as_str())));
                }
            }
            vec![indented(
                format!("  {}", said.join(&format!(" {} ", look.glyphs.separator))),
                Token::Muted,
            )]
        }
        Part::HistoryHeading => {
            let mut title = format!(
                "{} {} by {}",
                capitalised(&range_words(view.range)),
                look.glyphs.separator,
                group_words(view.group)
            );
            if let Some(zone) = view.report.and_then(|report| report.zone.as_deref()) {
                title.push_str(&format!(
                    " {} days in {}",
                    look.glyphs.separator,
                    one_line(zone)
                ));
            }
            if view.reading {
                return vec![indented(
                    format!(
                        "{title} {} reading{}",
                        look.glyphs.separator, look.glyphs.ellipsis
                    ),
                    Token::Title,
                )];
            }
            vec![table.heading(&title, focused, look)]
        }
        Part::Group(at) => {
            let Some(row) = view.report.and_then(|report| report.rows.get(*at)) else {
                return Vec::new();
            };
            let group = view.report.map_or(&[][..], |report| &report.group_by[..1]);
            vec![indented(cell_name(row, group), Token::Title)]
        }
        Part::HistoryRow(at) => {
            let Some(report) = view.report else {
                return Vec::new();
            };
            let Some(row) = report.rows.get(*at) else {
                return Vec::new();
            };
            let (group, level) = if report.group_by.len() > 1 {
                (&report.group_by[1..], 1)
            } else {
                (&report.group_by[..], 0)
            };
            let name = if report.group_by.is_empty() {
                // The default grouping: by provider, then model.
                cell_name(row, &[UsageGroup::Provider, UsageGroup::Model])
            } else {
                cell_name(row, group)
            };
            vec![table.row(&name, level, &row.totals, name_token, look)]
        }
        Part::Total => {
            let Some(report) = view.report else {
                return Vec::new();
            };
            let cost = if width < NARROW {
                cost_words_narrow(&report.totals.costs)
            } else {
                cost_words(&report.totals.costs)
            };
            let left = Line::plain("  ").with(Span::new("total", theme.style(name_token)));
            let right = Line::styled(cost, theme.style(Token::Text));
            // Where the cost does not fit at the row's end, it follows the
            // word, cut at its end: its first basis stays.
            if left.width() + 2 + right.width() > width {
                let line = left.with(Span::plain("  ")).with(Span::new(
                    right.text().into_owned(),
                    theme.style(Token::Text),
                ));
                return vec![cut(&line, width, look.glyphs.ellipsis)];
            }
            vec![spread(&left, &right, width, 2, look.glyphs.ellipsis)]
        }
        Part::Time => {
            let Some(time) = view.report.and_then(|report| report.time.as_ref()) else {
                return Vec::new();
            };
            let ms = |ms: u64| duration(std::time::Duration::from_millis(ms));
            let model = &time.model;
            let waiting = model.connect_ms
                + model.first_byte_ms
                + model.streaming_ms
                + model.retry_ms
                + model.summary_ms;
            let turns = if time.turns == 1 { "turn" } else { "turns" };
            let mut text = format!(
                "{} {turns}, {}: the model {}, tools {}",
                time.turns,
                ms(time.wall_ms),
                ms(waiting),
                ms(time.tools_ms)
            );
            if time.approval_ms > 0 {
                text.push_str(&format!(", waiting for you {}", ms(time.approval_ms)));
            }
            if time.verification_ms > 0 {
                text.push_str(&format!(", checks {}", ms(time.verification_ms)));
            }
            wrapped(&text, muted, width)
        }
        Part::Legend(text) => wrapped(text, muted, width),
        Part::Limits(at) => {
            let Some(limits) = limits.get(*at) else {
                return Vec::new();
            };
            limit_lines(limits, view.now, view.zone, focused, width, look)
        }
        Part::NotAvailable(provider) => {
            let line = Line::plain("  ")
                .with(Span::new(
                    one_line(provider.as_str()),
                    theme.style(name_token),
                ))
                .with(Span::new("  not available", muted));
            vec![cut(&line, width, look.glyphs.ellipsis)]
        }
        Part::Said(text) => wrapped(&one_line(text), theme.style(Token::Warning), width),
    }
}

/// `text` two columns in, wrapped at the width, never cut.
fn wrapped(text: &str, style: cortex_term::Style, width: usize) -> Vec<Line> {
    let room = width.saturating_sub(2).max(1);
    wrap(&[Span::new(text, style)], room)
        .into_iter()
        .map(|line| {
            let mut out = Line::plain(if width > 2 { "  " } else { "" });
            for span in line.spans() {
                out.push(span.clone());
            }
            out
        })
        .collect()
}

/// A row's name, from the fields its grouping sets.
fn cell_name(row: &UsageRow, group: &[UsageGroup]) -> String {
    let mut names = Vec::new();
    for group in group {
        let name = match group {
            UsageGroup::Provider => row.provider.as_ref().map(|p| one_line(p.as_str())),
            UsageGroup::Model => row.model.as_ref().map(|m| one_line(m.as_str())),
            UsageGroup::Project => row
                .project_label
                .as_deref()
                .map(one_line)
                .or_else(|| row.project.as_ref().map(|p| one_line(p.as_str()))),
            UsageGroup::Source => row.source.as_deref().map(one_line),
            UsageGroup::Speed => row.speed.as_ref().map(|s| one_line(s.as_str())),
            _ => None,
        };
        names.push(name.unwrap_or_else(|| "—".to_owned()));
    }
    names.join(" ")
}

/// Costs with their bases, apart: `$0.42`, `$1.92 est.`, `$44.40 metered
/// + $2.10 est.`, or `—` with no price known.
#[must_use]
pub fn cost_words(costs: &[Cost]) -> String {
    if costs.is_empty() {
        return "—".to_owned();
    }
    let mixed = costs.len() > 1;
    costs
        .iter()
        .map(|cost| {
            let amount = dollars(cost.usd_nanos);
            match &cost.basis {
                CostBasis::Metered if mixed => format!("{amount} metered"),
                CostBasis::Metered => amount,
                CostBasis::Estimate => format!("{amount} est."),
                CostBasis::Recorded => format!("{amount} recorded"),
                other => format!("{amount} {}", one_line(other.as_str())),
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Costs with their bases in a narrow window, where a row's end may be
/// cut: the estimates and the recorded costs first, each marked, then the
/// metered one plain (`$1.92 est. + $41.87`), so that what is cut is never
/// the mark.
#[must_use]
pub fn cost_words_narrow(costs: &[Cost]) -> String {
    if costs.len() < 2 {
        return cost_words(costs);
    }
    let marked = costs.iter().filter(|cost| cost.basis != CostBasis::Metered);
    let metered = costs.iter().filter(|cost| cost.basis == CostBasis::Metered);
    marked
        .chain(metered)
        .map(|cost| {
            let amount = dollars(cost.usd_nanos);
            match &cost.basis {
                CostBasis::Metered => amount,
                CostBasis::Estimate => format!("{amount} est."),
                CostBasis::Recorded => format!("{amount} recorded"),
                other => format!("{amount} {}", one_line(other.as_str())),
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Nano-dollars as dollars: `$41.87`, `$0.0042` under a cent.
fn dollars(nanos: u64) -> String {
    let value = nanos as f64 / 1e9;
    if nanos > 0 && value < 0.01 {
        format!("${value:.4}")
    } else {
        format!("${value:.2}")
    }
}

/// One provider's limits: a line for its ordinary limits, one for each
/// speed tier with its own, then its balances; each says its age.
fn limit_lines(
    limits: &ProviderLimits,
    now: UtcTime,
    zone: &Zone,
    focused: bool,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let theme = &look.theme;
    let muted = theme.style(Token::Muted);
    let separator = format!(" {} ", look.glyphs.separator);
    let provider = one_line(limits.provider.as_str());
    let mut tiers: Vec<Option<String>> = Vec::new();
    for window in &limits.windows {
        let tier = window.speed.as_ref().map(|s| one_line(s.as_str()));
        if !tiers.contains(&tier) {
            tiers.push(tier);
        }
    }
    let mut age = match &limits.source {
        LimitSource::Refresh => "from the refresh, as of ".to_owned(),
        _ => "as of ".to_owned(),
    };
    age.push_str(&ago(limits.seen_at.as_str(), now));
    let mut said: Vec<(String, Vec<String>)> = Vec::new();
    for tier in &tiers {
        let name = match tier {
            Some(tier) => format!("{provider} {tier} tier"),
            None => provider.clone(),
        };
        let parts: Vec<String> = limits
            .windows
            .iter()
            .filter(|window| window.speed.as_ref().map(|s| one_line(s.as_str())) == *tier)
            .map(|window| window_words(window, now, zone))
            .collect();
        said.push((name, parts));
    }
    if said.is_empty() {
        said.push((provider.clone(), Vec::new()));
    }
    for balance in &limits.balances {
        let currency = one_line(&balance.currency);
        let text = match (&balance.remaining, &balance.used, &balance.limit) {
            (Some(left), _, _) => format!("balance {} {currency} left", one_line(left)),
            (None, Some(used), Some(limit)) => {
                format!("{} of {} {currency} used", one_line(used), one_line(limit))
            }
            (None, Some(used), None) => format!("{} {currency} used", one_line(used)),
            _ => format!("balance in {currency} not said"),
        };
        said.push((provider.clone(), vec![text]));
    }
    if limits.available == Some(false) {
        said.push((provider.clone(), vec!["calls not allowed now".to_owned()]));
    }
    let room = width.saturating_sub(2).max(1);
    let mut lines = Vec::new();
    for (at, (name, parts)) in said.into_iter().enumerate() {
        let mut text = parts.join(&separator);
        if at == 0 {
            if !text.is_empty() {
                text.push_str(&separator);
            }
            text.push_str(&age);
        }
        let spans = [
            Span::new(
                name,
                theme.style(if focused && at == 0 {
                    Token::Selected
                } else {
                    Token::Text
                }),
            ),
            Span::new(format!("  {text}"), muted),
        ];
        for line in wrap(&spans, room) {
            let mut out = Line::plain(if width > 2 { "  " } else { "" });
            for span in line.spans() {
                out.push(span.clone());
            }
            lines.push(out);
        }
    }
    lines
}

/// One limit in words: `tokens 71% left · resets 14:32`.
fn window_words(window: &LimitWindow, now: UtcTime, zone: &Zone) -> String {
    let measure = measure_words(&window.measure);
    let reset = window
        .resets_at
        .as_ref()
        .and_then(|at| UtcTime::parse(at.as_str()).ok());
    if reset.is_some_and(|reset| reset <= now) {
        return format!("{measure} reset since; unknown until the next request");
    }
    let mut text = match (window.remaining, window.limit) {
        (Some(left), Some(limit)) if limit > 0 => {
            format!("{measure} {}% left", left.saturating_mul(100) / limit)
        }
        (Some(left), _) => format!("{measure} {} left", count(left)),
        _ => format!("{measure} unknown"),
    };
    if let Some(reset) = reset {
        text.push_str(&format!(" · resets {}", time_of_day(reset, now, zone)));
    }
    text
}

/// What a limit counts, in words.
fn measure_words(measure: &LimitMeasure) -> String {
    match measure {
        LimitMeasure::Requests => "requests".to_owned(),
        LimitMeasure::Tokens => "tokens".to_owned(),
        LimitMeasure::InputTokens => "input tokens".to_owned(),
        LimitMeasure::OutputTokens => "output tokens".to_owned(),
        other => one_line(other.as_str()),
    }
}

/// The warning line for the limit nearest its end, when one has less than a
/// fifth left: `anthropic fast tier: 18% of the output token limit left,
/// resets 15:00`.
#[must_use]
pub fn near(view: &UsageView<'_>) -> Option<String> {
    let mut nearest: Option<(u64, String)> = None;
    for limits in limits(view) {
        for window in &limits.windows {
            let (Some(left), Some(limit)) = (window.remaining, window.limit) else {
                continue;
            };
            let reset = window
                .resets_at
                .as_ref()
                .and_then(|at| UtcTime::parse(at.as_str()).ok());
            // Past its reset, what was left is not known.
            if limit == 0 || reset.is_some_and(|reset| reset <= view.now) {
                continue;
            }
            if left.saturating_mul(5) >= limit {
                continue;
            }
            let percent = left.saturating_mul(100) / limit;
            if nearest.as_ref().is_some_and(|(had, _)| *had <= percent) {
                continue;
            }
            let mut who = one_line(limits.provider.as_str());
            if let Some(speed) = &window.speed {
                who.push_str(&format!(" {} tier", one_line(speed.as_str())));
            }
            let measure = match &window.measure {
                LimitMeasure::Requests => "request".to_owned(),
                LimitMeasure::Tokens => "token".to_owned(),
                LimitMeasure::InputTokens => "input token".to_owned(),
                LimitMeasure::OutputTokens => "output token".to_owned(),
                other => one_line(other.as_str()),
            };
            let mut text = format!("{who}: {percent}% of the {measure} limit left");
            if let Some(reset) = reset {
                text.push_str(&format!(
                    ", resets {}",
                    time_of_day(reset, view.now, view.zone)
                ));
            }
            nearest = Some((percent, text));
        }
    }
    nearest.map(|(_, text)| text)
}

/// `at` in the local time of `zone`: `15:00` within a day of `now`, else
/// with its date.
fn time_of_day(at: UtcTime, now: UtcTime, zone: &Zone) -> String {
    let local = at.unix_secs() + i64::from(zone.offset_at(at));
    let minutes = local.div_euclid(60);
    let clock = format!(
        "{:02}:{:02}",
        minutes.div_euclid(60).rem_euclid(24),
        minutes.rem_euclid(60)
    );
    if at.unix_secs() - now.unix_secs() < 86_400 {
        clock
    } else {
        format!("{} {clock}", zone.date_of(at))
    }
}

/// How long ago `at` was: `just now`, `40 s ago`, `2 min ago`, `3 h ago`,
/// `2 d ago`; `at an unknown time` for one that cannot be read.
#[must_use]
pub fn ago(at: &str, now: UtcTime) -> String {
    let Ok(at) = UtcTime::parse(at) else {
        return "an unknown time".to_owned();
    };
    let secs = now.unix_secs().saturating_sub(at.unix_secs()).max(0);
    match secs {
        0..5 => "just now".to_owned(),
        5..60 => format!("{secs} s ago"),
        60..3_600 => format!("{} min ago", secs / 60),
        3_600..86_400 => format!("{} h ago", secs / 3_600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// `text` with its first letter in capitals.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// A column of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Input,
    CacheRead,
    CacheWrite,
    Output,
    Cost,
}

impl Column {
    fn name(self) -> &'static str {
        match self {
            Self::Input => "in",
            Self::CacheRead => "cache read",
            Self::CacheWrite => "cache write",
            Self::Output => "out",
            Self::Cost => "cost",
        }
    }

    fn value(self, totals: &UsageTotals, narrow: bool) -> String {
        let tokens = &totals.tokens;
        match self {
            Self::Input => count(tokens.input),
            Self::CacheRead => count(tokens.cache_read),
            Self::CacheWrite => count(tokens.cache_write),
            Self::Output => count(tokens.output),
            Self::Cost if narrow => cost_words_narrow(&totals.costs),
            Self::Cost => cost_words(&totals.costs),
        }
    }
}

/// The table's columns at a width, and how wide each is: the session's and
/// the history's rows line up.
#[derive(Debug, Clone)]
struct Table {
    columns: Vec<(Column, usize)>,
    width: usize,
    /// Under [`NARROW`] columns: the costs put their marks first.
    narrow: bool,
}

impl Table {
    /// The columns that fit `width`, the ones that drop first dropped.
    fn new(view: &UsageView<'_>, width: usize) -> Self {
        let mut all: Vec<&UsageTotals> = Vec::new();
        if let Some(session) = view.session {
            all.extend(session.by_model.iter().map(|model| &model.totals));
        }
        if let Some(report) = view.report.filter(|_| !view.reading) {
            all.extend(report.rows.iter().map(|row| &row.totals));
        }
        let narrow = width < NARROW;
        let wide = |column: Column| {
            all.iter()
                .map(|totals| cortex_base::text::width(&column.value(totals, narrow)))
                .chain([column.name().len()])
                .max()
                .unwrap_or(0)
        };
        let mut columns: Vec<Column> = if width < NARROW {
            vec![Column::Output, Column::Cost]
        } else {
            vec![
                Column::Input,
                Column::CacheRead,
                Column::CacheWrite,
                Column::Output,
                Column::Cost,
            ]
        };
        let mut sized: Vec<(Column, usize)> = columns.iter().map(|c| (*c, wide(*c))).collect();
        // The name keeps at least a third of the row.
        let least_name = width / 3;
        for drop in [Column::CacheWrite, Column::CacheRead, Column::Input] {
            let used: usize = sized.iter().map(|(_, wide)| wide + 2).sum();
            if used + least_name + 2 <= width {
                break;
            }
            columns.retain(|column| *column != drop);
            sized.retain(|(column, _)| *column != drop);
        }
        // In a narrow window the output goes too when the name would have
        // no room beside the cost, whose basis must show.
        let used: usize = sized.iter().map(|(_, wide)| wide + 2).sum();
        if narrow && used + 4 + 4 > width {
            sized.retain(|(column, _)| *column != Column::Output);
        }
        Self {
            columns: sized,
            width,
            narrow,
        }
    }

    /// The columns' room: what is left for the name.
    fn name_room(&self, indent: usize) -> usize {
        let used: usize = self.columns.iter().map(|(_, wide)| wide + 2).sum();
        self.width.saturating_sub(used + indent)
    }

    /// The heading row: `title` where the names go, then each column's name.
    fn heading(&self, title: &str, focused: bool, look: &Look) -> Line {
        let theme = &look.theme;
        let token = if focused {
            Token::Selected
        } else {
            Token::Title
        };
        let mut line = Line::plain("  ");
        let room = self.name_room(2);
        line.push(Span::new(fit(title, room, look), theme.style(token)));
        for (column, wide) in &self.columns {
            line.push(Span::new(
                format!("  {:>wide$}", column.name()),
                theme.style(Token::Muted),
            ));
        }
        cut(&line, self.width, look.glyphs.ellipsis)
    }

    /// A row: its name `level` steps in, then each column's value.
    fn row(
        &self,
        name: &str,
        level: usize,
        totals: &UsageTotals,
        token: Token,
        look: &Look,
    ) -> Line {
        let theme = &look.theme;
        let indent = 2 + 2 * level;
        let mut line = Line::plain(" ".repeat(indent));
        let room = self.name_room(indent);
        line.push(Span::new(fit(name, room, look), theme.style(token)));
        for (column, wide) in &self.columns {
            let value = column.value(totals, self.narrow);
            let token = if *column == Column::Cost && totals.costs.is_empty() {
                Token::Muted
            } else {
                Token::Text
            };
            let pad = wide.saturating_sub(cortex_base::text::width(&value));
            line.push(Span::new(
                format!("  {}{value}", " ".repeat(pad)),
                theme.style(token),
            ));
        }
        cut(&line, self.width, look.glyphs.ellipsis)
    }
}

/// `text` cut and padded to exactly `room` columns.
fn fit(text: &str, room: usize, look: &Look) -> String {
    let line = cut(&Line::plain(text), room, look.glyphs.ellipsis);
    let text = line.text().into_owned();
    let pad = room.saturating_sub(cortex_base::text::width(&text));
    format!("{text}{}", " ".repeat(pad))
}

#[cfg(test)]
pub(crate) mod tests;
