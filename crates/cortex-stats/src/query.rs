//! The usage query: a range of local days and a grouping, answered from the
//! aggregates alone (`docs/architecture/18-usage.md`, "What it shows").
//!
//! | Range | Days |
//! |---|---|
//! | Today | The local day. |
//! | Week | From the week's first day (Monday, unless `usage.week_start` says otherwise) to today; with rolling ranges, the last 7 days. |
//! | Month | From the 1st to today; with rolling ranges, the last 30 days. |
//! | All | Every day and every month on record. |
//!
//! The session range is the open session's own figures, which the core
//! has; it is not answered here.

use std::collections::BTreeMap;

use cortex_base::{LocalDate, UtcTime, Zone};
use cortex_protocol::usage::{
    Cost, CostBasis, LimitSource, ProviderLimits, TimeTotals, ToolTime, UsageGroup, UsageRange,
    UsageReport, UsageRow, UsageTotals,
};
use cortex_protocol::{ModelId, ProjectKey, ProviderId, Speed};

use crate::aggregate::{Part, UsageSum, add_tool};

/// The days of a range, both ends included. `None` for all time.
#[must_use]
pub fn days_of(
    range: &UsageRange,
    today: LocalDate,
    week_start: u8,
    rolling: bool,
) -> Option<(LocalDate, LocalDate)> {
    let start = match range {
        UsageRange::Today | UsageRange::Session => today,
        UsageRange::Week if rolling => today.add_days(-6),
        UsageRange::Week => {
            let back = (i64::from(today.weekday()) - i64::from(week_start % 7)).rem_euclid(7);
            today.add_days(-back)
        }
        UsageRange::Month if rolling => today.add_days(-29),
        UsageRange::Month => today.first_of_month(),
        _ => return None,
    };
    Some((start, today))
}

/// What a report is asked for.
#[derive(Debug, Clone)]
pub struct Query {
    /// The range.
    pub range: UsageRange,
    /// The grouping, outermost first. Empty: provider, then model.
    pub group_by: Vec<UsageGroup>,
    /// Only this project.
    pub project: Option<ProjectKey>,
    /// The first day of a week, Monday 0 to Sunday 6.
    pub week_start: u8,
    /// Week and month as the last 7 and 30 days.
    pub rolling: bool,
}

/// Whether a bucket's period is in the range.
fn in_range(period: &str, days: Option<(LocalDate, LocalDate)>) -> bool {
    match days {
        None => true,
        Some((first, last)) => {
            LocalDate::parse(period).is_some_and(|day| first <= day && day <= last)
        }
    }
}

/// The fields of a row a grouping fills.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct RowKey {
    provider: Option<String>,
    model: Option<String>,
    project: Option<String>,
    source: Option<String>,
    speed: Option<String>,
}

/// Our own records' source.
pub const SOURCE: &str = "cortex-harness";

fn costs(sum: &UsageSum) -> Vec<Cost> {
    [
        (sum.metered_nanos, CostBasis::Metered),
        (sum.estimate_nanos, CostBasis::Estimate),
        (sum.recorded_nanos, CostBasis::Recorded),
    ]
    .into_iter()
    .filter(|(nanos, _)| *nanos > 0)
    .map(|(usd_nanos, basis)| Cost { usd_nanos, basis })
    .collect()
}

fn totals(sum: &UsageSum) -> UsageTotals {
    UsageTotals {
        tokens: sum.tokens(),
        requests: sum.requests,
        costs: costs(sum),
    }
}

/// The aggregates' own sums, which saturate (found by the review of step
/// 16's fixes: the report had copies of its own that overflowed).
fn add_sum(into: &mut UsageSum, other: &UsageSum) {
    into.add(other);
}

fn add_time(into: &mut TimeTotals, other: &TimeTotals) {
    crate::aggregate::add_time(into, other);
}

/// Answers a query from the aggregates. `now` and `zone` place today; the
/// limits are the latest each provider reported, a limit whose reset time
/// has passed shown as not known.
#[must_use]
pub fn report(
    part: &Part,
    query: &Query,
    zone: &Zone,
    now: UtcTime,
    limits: &[ProviderLimits],
    recording: bool,
) -> UsageReport {
    let today = zone.date_of(now);
    let days = days_of(&query.range, today, query.week_start, query.rolling);
    let group_by = if query.group_by.is_empty() {
        vec![UsageGroup::Provider, UsageGroup::Model]
    } else {
        query.group_by.clone()
    };
    let has = |group: &UsageGroup| group_by.contains(group);
    let project_wanted = |project: &str| {
        query
            .project
            .as_ref()
            .is_none_or(|wanted| wanted.as_str() == project)
    };

    let mut rows: BTreeMap<RowKey, UsageSum> = BTreeMap::new();
    let mut all = UsageSum::default();
    for (key, sum) in &part.usage {
        if !in_range(&key.period, days) || !project_wanted(&key.project) {
            continue;
        }
        let row = RowKey {
            provider: has(&UsageGroup::Provider).then(|| key.provider.clone()),
            model: has(&UsageGroup::Model).then(|| key.model.clone()),
            project: has(&UsageGroup::Project).then(|| key.project.clone()),
            source: has(&UsageGroup::Source).then(|| SOURCE.to_owned()),
            speed: has(&UsageGroup::Speed).then(|| key.speed.clone()),
        };
        add_sum(rows.entry(row).or_default(), sum);
        add_sum(&mut all, sum);
    }

    // Time is a turn's: it can be shown by provider, model and project.
    let time_groups = group_by.iter().all(|group| {
        matches!(
            group,
            UsageGroup::Provider | UsageGroup::Model | UsageGroup::Project
        )
    });
    let mut row_time: BTreeMap<RowKey, TimeTotals> = BTreeMap::new();
    let mut all_time = TimeTotals::default();
    for (key, time) in &part.time {
        if !in_range(&key.period, days) || !project_wanted(&key.project) {
            continue;
        }
        add_time(&mut all_time, time);
        if time_groups {
            let row = RowKey {
                provider: has(&UsageGroup::Provider).then(|| key.provider.clone()),
                model: has(&UsageGroup::Model).then(|| key.model.clone()),
                project: has(&UsageGroup::Project).then(|| key.project.clone()),
                source: None,
                speed: None,
            };
            add_time(row_time.entry(row).or_default(), time);
        }
    }

    let mut tools: BTreeMap<String, ToolTime> = BTreeMap::new();
    for (key, tool) in &part.tools {
        if !in_range(&key.period, days) || !project_wanted(&key.project) {
            continue;
        }
        add_tool(
            tools.entry(key.tool.clone()).or_insert_with(|| ToolTime {
                tool: key.tool.clone(),
                ..ToolTime::default()
            }),
            tool,
        );
    }
    let mut tools: Vec<ToolTime> = tools.into_values().collect();
    tools.sort_by(|a, b| {
        b.share_ms
            .cmp(&a.share_ms)
            .then_with(|| a.tool.cmp(&b.tool))
    });

    let mut rows_out = Vec::with_capacity(rows.len().max(row_time.len()));
    // Every row of usage, and a row of time alone where turns ran and no
    // request was counted (none can be, but the files may say so).
    let mut keys: Vec<RowKey> = rows.keys().cloned().collect();
    for key in row_time.keys() {
        if !rows.contains_key(key) {
            keys.push(key.clone());
        }
    }
    keys.sort();
    for key in keys {
        let sum = rows.get(&key).cloned().unwrap_or_default();
        let time = if time_groups {
            row_time.get(&key).cloned()
        } else {
            None
        };
        rows_out.push(UsageRow {
            provider: key.provider.map(ProviderId::new),
            model: key.model.map(ModelId::new),
            project: key
                .project
                .filter(|project| !project.is_empty())
                .map(ProjectKey::new),
            project_label: None,
            source: key.source,
            speed: key.speed.as_deref().map(Speed::from_wire),
            totals: totals(&sum),
            time,
        });
    }

    UsageReport {
        range: query.range.clone(),
        from: days.map(|(first, _)| zone.start_of(first).to_timestamp()),
        to: days.map(|_| now.to_timestamp()),
        group_by,
        rows: rows_out,
        totals: totals(&all),
        limits: current_limits(limits, now),
        sources: vec![SOURCE.to_owned()],
        recording,
        time: (all_time.turns > 0).then_some(all_time),
        tools,
        zone: Some(zone.name().to_owned()),
    }
}

/// The latest limits, as they stand at `now`: a figure whose reset time has
/// passed is not known any more, not full, so what was left of it is
/// dropped and its limit kept. Each provider's figures from its responses
/// come before those from a refresh.
#[must_use]
pub fn current_limits(limits: &[ProviderLimits], now: UtcTime) -> Vec<ProviderLimits> {
    let mut out: Vec<ProviderLimits> = limits
        .iter()
        .cloned()
        .map(|mut limits| {
            for window in &mut limits.windows {
                let reset = window
                    .resets_at
                    .as_ref()
                    .and_then(|at| UtcTime::parse(at.as_str()).ok());
                if reset.is_some_and(|reset| reset <= now) {
                    window.remaining = None;
                }
            }
            limits
        })
        .collect();
    out.sort_by(|a, b| {
        let order = |source: &LimitSource| u8::from(source != &LimitSource::Headers);
        (a.provider.as_str(), order(&a.source)).cmp(&(b.provider.as_str(), order(&b.source)))
    });
    out
}
