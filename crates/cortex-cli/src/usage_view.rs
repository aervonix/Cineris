//! What `cortex usage` and `cortex stats` print: the report as a person
//! reads it (`docs/architecture/18-usage.md`, "What it shows").
//!
//! Nothing here does I/O, so the text is tested on reports written by hand.

use std::fmt::Write as _;

use cortex_base::UtcTime;
use cortex_core::StatsSummary;
use cortex_protocol::usage::{
    CostBasis, LimitMeasure, LimitSource, ProviderLimits, TimeTotals, UsageReport, UsageRow,
    UsageTotals,
};

/// A count as it reads: `612`, `612k`, `1.21M`.
fn count(n: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "for display")]
    let value = n as f64;
    if n >= 1_000_000 {
        format!("{:.2}M", value / 1e6)
    } else if n >= 10_000 {
        format!("{:.0}k", value / 1e3)
    } else {
        n.to_string()
    }
}

/// Nano-dollars as dollars and cents.
fn dollars(nanos: u64) -> String {
    let cents = (nanos + 5_000_000) / 10_000_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// The costs of some usage, each basis apart.
fn costs(totals: &UsageTotals) -> String {
    if totals.costs.is_empty() {
        return "—".to_owned();
    }
    totals
        .costs
        .iter()
        .map(|cost| match cost.basis {
            CostBasis::Metered => dollars(cost.usd_nanos),
            CostBasis::Estimate => format!("{} est.", dollars(cost.usd_nanos)),
            _ => format!("{} stated", dollars(cost.usd_nanos)),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Milliseconds as they read: `850 ms`, `12.4 s`, `3 min 5 s`, `2 h 4 min`.
fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        #[expect(clippy::cast_precision_loss, reason = "for display")]
        let seconds = ms as f64 / 1000.0;
        format!("{seconds:.1} s")
    } else if ms < 3_600_000 {
        format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000)
    } else {
        format!("{} h {} min", ms / 3_600_000, ms % 3_600_000 / 60_000)
    }
}

fn share(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".to_owned();
    }
    format!("{}%", (part * 100 + whole / 2) / whole)
}

/// How long ago a time was.
fn ago(seen: &str, now: UtcTime) -> String {
    let Some(seconds) = UtcTime::parse(seen)
        .ok()
        .and_then(|seen| now.duration_since(seen))
        .map(|since| since.as_secs())
    else {
        return "just now".to_owned();
    };
    if seconds < 60 {
        format!("{seconds} s ago")
    } else if seconds < 3600 {
        format!("{} min ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} h ago", seconds / 3600)
    } else {
        format!("{} days ago", seconds / 86_400)
    }
}

fn row_name(row: &UsageRow) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = &row.provider {
        parts.push(provider.to_string());
    }
    if let Some(model) = &row.model {
        parts.push(model.to_string());
    }
    if let Some(project) = &row.project {
        parts.push(format!("project {project}"));
    }
    if let Some(source) = &row.source {
        parts.push(source.clone());
    }
    if let Some(speed) = &row.speed {
        parts.push(format!("speed {speed}"));
    }
    if parts.is_empty() {
        "(no project)".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn time_line(time: &TimeTotals) -> String {
    let wall = time.wall_ms;
    let model = &time.model;
    let model_ms = model.connect_ms
        + model.first_byte_ms
        + model.streaming_ms
        + model.retry_ms
        + model.summary_ms;
    format!(
        "{} turns, {}: model {} (connect {}, first byte {}, streaming {}, retries {}, \
         summaries {}), tools {}, approvals {}, checks {}, harness {}",
        time.turns,
        duration(wall),
        share(model_ms, wall),
        share(model.connect_ms, wall),
        share(model.first_byte_ms, wall),
        share(model.streaming_ms, wall),
        share(model.retry_ms, wall),
        share(model.summary_ms, wall),
        share(time.tools_ms, wall),
        share(time.approval_ms, wall),
        share(time.verification_ms, wall),
        share(time.harness_ms, wall),
    )
}

fn limit_line(limits: &ProviderLimits, now: UtcTime) -> String {
    let mut parts = Vec::new();
    for window in &limits.windows {
        let what = match &window.measure {
            LimitMeasure::Requests => "requests",
            LimitMeasure::InputTokens => "input tokens",
            LimitMeasure::OutputTokens => "output tokens",
            _ => "tokens",
        };
        let tier = window
            .speed
            .as_ref()
            .map_or_else(String::new, |speed| format!(" ({speed})"));
        let left = match (window.remaining, window.limit) {
            (Some(left), Some(limit)) => format!("{} of {} left", count(left), count(limit)),
            (Some(left), None) => format!("{} left", count(left)),
            // Past its reset time: not known, never full.
            (None, Some(limit))
                if window
                    .resets_at
                    .as_ref()
                    .and_then(|at| UtcTime::parse(at.as_str()).ok())
                    .is_some_and(|reset| reset <= now) =>
            {
                format!(
                    "{} a window; reset since, unknown until the next request",
                    count(limit)
                )
            }
            (None, Some(limit)) => format!("{} a window", count(limit)),
            (None, None) => "not known".to_owned(),
        };
        parts.push(format!("{what}{tier} {left}"));
    }
    for balance in &limits.balances {
        let mut text = format!("balance {}", balance.currency);
        if let Some(left) = &balance.remaining {
            let _ = write!(text, " {left} left");
        }
        if let Some(limit) = &balance.limit {
            let _ = write!(text, " of {limit}");
        }
        if let Some(used) = &balance.used {
            let _ = write!(text, ", {used} used");
        }
        parts.push(text);
    }
    if limits.available == Some(false) {
        parts.push("may not make calls now".to_owned());
    }
    if let Some(epoch) = &limits.next_epoch {
        parts.push(format!("next epoch {epoch}"));
    }
    let source = match &limits.source {
        LimitSource::Refresh => "from the refresh",
        _ => "from responses",
    };
    format!(
        "{:<12} {} · {source}, as of {}",
        limits.provider,
        if parts.is_empty() {
            "nothing reported".to_owned()
        } else {
            parts.join(" · ")
        },
        ago(limits.seen_at.as_str(), now)
    )
}

/// The report, as `cortex usage` prints it.
pub(crate) fn usage(report: &UsageReport, now: UtcTime) -> String {
    let mut out = String::new();
    let from = report.from.as_ref().map_or_else(
        || "everything on record".to_owned(),
        |from| format!("from {from}"),
    );
    let _ = writeln!(
        out,
        "Usage · {} ({from}; days in {})",
        report.range,
        report.zone.as_deref().unwrap_or("UTC")
    );
    if !report.recording {
        out.push_str("Statistics are off (`stats.enabled = false`): nothing is recorded.\n");
    }
    let width = report
        .rows
        .iter()
        .map(|row| row_name(row).chars().count())
        .max()
        .unwrap_or(0)
        .max(5);
    for row in &report.rows {
        let tokens = &row.totals.tokens;
        let _ = writeln!(
            out,
            "  {:<width$}  in {:>7}  cache read {:>7}  cache write {:>7}  out {:>7}  {:>4} requests  {}",
            row_name(row),
            count(tokens.input),
            count(tokens.cache_read),
            count(tokens.cache_write),
            count(tokens.output),
            row.totals.requests,
            costs(&row.totals),
        );
        if let Some(time) = &row.time {
            let _ = writeln!(out, "  {:<width$}  time: {}", "", time_line(time));
        }
    }
    if report.rows.is_empty() {
        out.push_str("  Nothing recorded in this range.\n");
    }
    let _ = writeln!(
        out,
        "Total  {} requests  {}",
        report.totals.requests,
        costs(&report.totals)
    );
    if report
        .totals
        .costs
        .iter()
        .any(|cost| cost.basis == CostBasis::Metered)
    {
        out.push_str(
            "       Our calculation from the providers' published prices, not their invoice.\n",
        );
    }
    if let Some(time) = &report.time {
        let _ = writeln!(out, "Time   {}", time_line(time));
    }
    if !report.tools.is_empty() {
        let tools: Vec<String> = report
            .tools
            .iter()
            .take(8)
            .map(|tool| {
                format!(
                    "{} {} ({} calls, {} running, {} queued)",
                    tool.tool,
                    duration(tool.share_ms),
                    tool.calls,
                    duration(tool.running_ms),
                    duration(tool.queued_ms)
                )
            })
            .collect();
        let _ = writeln!(out, "Tools  {}", tools.join(" · "));
    }
    if report.limits.is_empty() {
        out.push_str("Limits not available\n");
    } else {
        for (at, limits) in report.limits.iter().enumerate() {
            let lead = if at == 0 { "Limits" } else { "      " };
            let _ = writeln!(out, "{lead} {}", limit_line(limits, now));
        }
    }
    out
}

/// What `cortex stats` prints.
pub(crate) fn stats(summary: &StatsSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Statistics are {}.",
        if summary.enabled {
            "recorded"
        } else {
            "off (`stats.enabled = false`)"
        }
    );
    match &summary.dir {
        Some(dir) => {
            let _ = writeln!(out, "Where: {dir}");
        }
        None => out.push_str("Where: nowhere: there is no state directory.\n"),
    }
    let _ = writeln!(
        out,
        "Stored: {} requests, {} turns, {} tool calls, in {} monthly files ({}), {} bytes in all",
        summary.requests,
        summary.turns,
        summary.tool_calls,
        summary.months.len(),
        if summary.months.is_empty() {
            "none".to_owned()
        } else {
            summary.months.join(", ")
        },
        summary.bytes
    );
    let _ = writeln!(
        out,
        "Kept: a monthly file for {} months after its own; what it held stays in the aggregates.",
        summary.retention
    );
    if summary.dropped > 0 {
        let _ = writeln!(
            out,
            "Dropped: {} events and limits updates, when the writer could not keep up.",
            summary.dropped
        );
    }
    if summary.unreadable > 0 {
        let _ = writeln!(
            out,
            "Unreadable: {} lines were skipped.",
            summary.unreadable
        );
    }
    match &summary.aggregates {
        cortex_core::AggregatesFound::Newer { version } => {
            let _ = writeln!(
                out,
                "The aggregates are a newer build's (version {version}): this build neither reads \
                 nor changes them, and the figures above are the monthly files' alone."
            );
        }
        cortex_core::AggregatesFound::Unreadable => out.push_str(
            "The aggregates file could not be read: the figures above are the monthly files' \
             alone.\n",
        ),
        _ => {}
    }
    for path in &summary.set_aside {
        let _ = writeln!(
            out,
            "Set aside: {path}, an aggregates file that could not be read, with the months \
             pruned before it."
        );
    }
    out.push_str("Nothing of it leaves this machine. `cortex stats reset` deletes it.\n");
    out
}

#[cfg(test)]
mod tests {
    use cortex_protocol::usage::{Cost, LimitWindow, ModelTime, TokenUsage};
    use cortex_protocol::{ModelId, ProviderId, Timestamp};

    use super::*;

    fn now() -> UtcTime {
        UtcTime::parse("2026-11-15T12:02:00Z").unwrap()
    }

    #[test]
    fn a_report_reads_as_the_view_sketches_it() {
        let report = UsageReport {
            range: cortex_protocol::usage::UsageRange::Month,
            from: Some(Timestamp::new("2026-11-01T00:00:00Z")),
            to: Some(Timestamp::new("2026-11-15T12:02:00Z")),
            group_by: Vec::new(),
            rows: vec![UsageRow {
                provider: Some(ProviderId::new("anthropic")),
                model: Some(ModelId::new("claude-opus-5-5")),
                totals: UsageTotals {
                    tokens: TokenUsage {
                        input: 1_210_000,
                        cache_read: 38_400_000,
                        cache_write: 2_900_000,
                        output: 612_000,
                        ..TokenUsage::default()
                    },
                    requests: 940,
                    costs: vec![Cost {
                        usd_nanos: 41_870_000_000,
                        basis: CostBasis::Metered,
                    }],
                },
                time: Some(TimeTotals {
                    turns: 4,
                    wall_ms: 10_000,
                    model: ModelTime {
                        streaming_ms: 6_000,
                        ..ModelTime::default()
                    },
                    tools_ms: 4_000,
                    ..TimeTotals::default()
                }),
                ..UsageRow::default()
            }],
            totals: UsageTotals {
                requests: 940,
                costs: vec![Cost {
                    usd_nanos: 41_870_000_000,
                    basis: CostBasis::Metered,
                }],
                ..UsageTotals::default()
            },
            limits: vec![ProviderLimits {
                provider: ProviderId::new("anthropic"),
                windows: vec![LimitWindow {
                    measure: LimitMeasure::Requests,
                    speed: None,
                    limit: Some(100),
                    remaining: None,
                    resets_at: Some(Timestamp::new("2026-11-15T12:01:00Z")),
                }],
                seen_at: Timestamp::new("2026-11-15T12:00:00Z"),
                source: LimitSource::Headers,
                balances: Vec::new(),
                available: None,
                next_epoch: None,
            }],
            sources: Vec::new(),
            recording: true,
            time: None,
            tools: Vec::new(),
            zone: Some("UTC".to_owned()),
            notes: Vec::new(),
        };
        let text = usage(&report, now());
        assert!(text.contains("anthropic · claude-opus-5-5"), "{text}");
        assert!(text.contains("in   1.21M"), "{text}");
        assert!(text.contains("$41.87"), "{text}");
        assert!(text.contains("model 60%"), "{text}");
        assert!(text.contains("tools 40%"), "{text}");
        // A limit past its reset time is never shown as full.
        assert!(
            text.contains("reset since, unknown until the next request"),
            "{text}"
        );
        assert!(!text.contains("100 of 100"), "{text}");
        assert!(text.contains("as of 2 min ago"), "{text}");
    }
}
