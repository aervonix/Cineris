//! The aggregates: counts and sums over the events, by the keys the usage
//! view and the adaptive features ask by
//! (`docs/architecture/18-usage.md`, "Where our own numbers come from").
//!
//! A [`Part`] is what some lines add up to. It is additive: the part of two
//! runs of lines is the [`merge`](Part::merge) of their parts, in either
//! order. That is what lets the aggregates be kept per monthly file,
//! brought up to date from where each file was last read, and rebuilt from
//! the files to the same values.
//!
//! **Periods.** Usage, time and tools are kept by the local day an event
//! fell on (`2026-11-15`). Days more than [`DAYS_KEPT`] before today are
//! folded into their month (`2025-09`), so that all-time figures outlive the
//! raw files; folding is the same whenever it is done
//! ([`fold_days`](Part::fold_days)).

use std::collections::BTreeMap;

use cortex_base::LocalDate;
use cortex_protocol::usage::{CostBasis, ModelTime, TimeTotals, TokenUsage, ToolTime};
use serde::{Deserialize, Serialize};

use crate::record::{Line, Record};

/// Day buckets newer than this many days are kept as days.
pub const DAYS_KEPT: i64 = 400;

/// What usage is grouped by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UsageKey {
    /// `YYYY-MM-DD`, or `YYYY-MM` once folded.
    pub period: String,
    /// The provider.
    pub provider: String,
    /// The model.
    pub model: String,
    /// The speed the requests ran at.
    pub speed: String,
    /// The project's key, empty when there was none.
    pub project: String,
    /// The billing kind.
    pub billing: String,
}

/// Usage added up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSum {
    /// Requests.
    pub requests: u64,
    /// Fresh input tokens.
    pub input: u64,
    /// Cache reads.
    pub cache_read: u64,
    /// Cache writes.
    pub cache_write: u64,
    /// Output tokens.
    pub output: u64,
    /// The part of the output that was reasoning.
    pub reasoning: u64,
    /// Metered cost, in nano-dollars.
    pub metered_nanos: u64,
    /// API-equivalent estimates, in nano-dollars.
    pub estimate_nanos: u64,
    /// Costs a provider stated, in nano-dollars.
    pub recorded_nanos: u64,
    /// Requests with no known price.
    pub unpriced: u64,
}

impl UsageSum {
    fn add(&mut self, other: &Self) {
        self.requests += other.requests;
        self.input += other.input;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.metered_nanos += other.metered_nanos;
        self.estimate_nanos += other.estimate_nanos;
        self.recorded_nanos += other.recorded_nanos;
        self.unpriced += other.unpriced;
    }

    /// The tokens, as the protocol counts them.
    #[must_use]
    pub fn tokens(&self) -> TokenUsage {
        TokenUsage {
            input: self.input,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            cache_write_classes: Vec::new(),
            output: self.output,
            reasoning: (self.reasoning > 0).then_some(self.reasoning),
        }
    }
}

/// What time is grouped by: the turns' main model.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimeKey {
    /// As for usage.
    pub period: String,
    /// The project's key, empty when there was none.
    pub project: String,
    /// The provider.
    pub provider: String,
    /// The model.
    pub model: String,
}

/// What tools' time is grouped by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolKey {
    /// As for usage.
    pub period: String,
    /// The project's key, empty when there was none.
    pub project: String,
    /// The tool.
    pub tool: String,
}

/// What success rates are kept by: a model with its variants, and a tool,
/// or `""` for the turn as a whole.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OutcomeKey {
    /// The model.
    pub model: String,
    /// The variants, as [`Variants::key`](crate::Variants::key) writes them.
    pub variants: String,
    /// The tool, or empty for turns.
    pub tool: String,
}

fn add_time(into: &mut TimeTotals, other: &TimeTotals) {
    into.turns += other.turns;
    into.wall_ms += other.wall_ms;
    add_model_time(&mut into.model, &other.model);
    into.tools_ms += other.tools_ms;
    into.approval_ms += other.approval_ms;
    into.verification_ms += other.verification_ms;
    into.harness_ms += other.harness_ms;
}

pub(crate) fn add_model_time(into: &mut ModelTime, other: &ModelTime) {
    into.connect_ms += other.connect_ms;
    into.first_byte_ms += other.first_byte_ms;
    into.streaming_ms += other.streaming_ms;
    into.retry_ms += other.retry_ms;
    into.summary_ms += other.summary_ms;
}

pub(crate) fn add_tool(into: &mut ToolTime, other: &ToolTime) {
    into.calls += other.calls;
    into.share_ms += other.share_ms;
    into.running_ms += other.running_ms;
    into.queued_ms += other.queued_ms;
    into.approval_ms += other.approval_ms;
}

/// What some lines add up to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    /// Usage by day, provider, model, speed, project and billing.
    #[serde(default, with = "entries")]
    pub usage: BTreeMap<UsageKey, UsageSum>,
    /// Where turns' time went, by day, project and model.
    #[serde(default, with = "entries")]
    pub time: BTreeMap<TimeKey, TimeTotals>,
    /// Tools' time, by day, project and tool.
    #[serde(default, with = "entries")]
    pub tools: BTreeMap<ToolKey, ToolTime>,
    /// Outcomes by model, variants and tool: counts by outcome.
    #[serde(default, with = "entries")]
    pub outcomes: BTreeMap<OutcomeKey, BTreeMap<String, u64>>,
    /// Events the writer dropped because its queue was full.
    #[serde(default)]
    pub dropped: u64,
    /// Lines that could not be read.
    #[serde(default)]
    pub unreadable: u64,
}

impl Part {
    /// Whether nothing is counted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Adds one stored line.
    pub fn add_line(&mut self, line: &Line) {
        let period = line.day.clone();
        let project = |key: &Option<cortex_protocol::ProjectKey>| {
            key.as_ref()
                .map(|key| key.as_str().to_owned())
                .unwrap_or_default()
        };
        match &line.record {
            Record::Request(request) => {
                let key = UsageKey {
                    period,
                    provider: request.provider.as_str().to_owned(),
                    model: request.model.as_str().to_owned(),
                    speed: request.speed.as_str().to_owned(),
                    project: project(&request.project),
                    billing: request.billing.as_str().to_owned(),
                };
                let tokens = &request.tokens;
                let mut sum = UsageSum {
                    requests: 1,
                    input: tokens.input,
                    cache_read: tokens.cache_read,
                    cache_write: tokens.cache_write,
                    output: tokens.output,
                    reasoning: tokens.reasoning.unwrap_or(0),
                    ..UsageSum::default()
                };
                match &request.cost {
                    Some(cost) => match cost.basis {
                        CostBasis::Metered => sum.metered_nanos = cost.usd_nanos,
                        CostBasis::Recorded => sum.recorded_nanos = cost.usd_nanos,
                        _ => sum.estimate_nanos = cost.usd_nanos,
                    },
                    None => sum.unpriced = 1,
                }
                self.usage.entry(key).or_default().add(&sum);
            }
            Record::Tool(tool) => {
                let key = ToolKey {
                    period,
                    project: project(&tool.project),
                    tool: tool.tool.clone(),
                };
                add_tool(
                    self.tools.entry(key).or_insert_with(|| ToolTime {
                        tool: tool.tool.clone(),
                        ..ToolTime::default()
                    }),
                    &ToolTime {
                        tool: tool.tool.clone(),
                        calls: 1,
                        share_ms: 0,
                        running_ms: tool.running_ms,
                        queued_ms: tool.queued_ms,
                        approval_ms: tool.approval_ms,
                    },
                );
                let key = OutcomeKey {
                    model: tool.model.as_str().to_owned(),
                    variants: tool.variants.key(),
                    tool: tool.tool.clone(),
                };
                *self
                    .outcomes
                    .entry(key)
                    .or_default()
                    .entry(tool.outcome.as_str().to_owned())
                    .or_default() += 1;
            }
            Record::Turn(turn) => {
                let key = TimeKey {
                    period: period.clone(),
                    project: project(&turn.project),
                    provider: turn.provider.as_str().to_owned(),
                    model: turn.model.as_str().to_owned(),
                };
                add_time(
                    self.time.entry(key).or_default(),
                    &TimeTotals {
                        turns: 1,
                        wall_ms: turn.wall_ms,
                        model: turn.model_time.clone(),
                        tools_ms: turn.tools_ms,
                        approval_ms: turn.approval_ms,
                        verification_ms: turn.verification_ms,
                        harness_ms: turn.harness_ms,
                    },
                );
                for share in &turn.by_tool {
                    let key = ToolKey {
                        period: period.clone(),
                        project: project(&turn.project),
                        tool: share.tool.clone(),
                    };
                    self.tools
                        .entry(key)
                        .or_insert_with(|| ToolTime {
                            tool: share.tool.clone(),
                            ..ToolTime::default()
                        })
                        .share_ms += share.ms;
                }
                let key = OutcomeKey {
                    model: turn.model.as_str().to_owned(),
                    variants: turn.variants.key(),
                    tool: String::new(),
                };
                *self
                    .outcomes
                    .entry(key)
                    .or_default()
                    .entry(turn.outcome.as_str().to_owned())
                    .or_default() += 1;
            }
            Record::Dropped { count, .. } => self.dropped += count,
            // The context record is kept in the raw files for the context
            // profiler, which reads them; nothing of it is summed here.
            Record::Context(_) => {}
        }
    }

    /// Adds what another part counted.
    pub fn merge(&mut self, other: &Self) {
        for (key, sum) in &other.usage {
            self.usage.entry(key.clone()).or_default().add(sum);
        }
        for (key, time) in &other.time {
            add_time(self.time.entry(key.clone()).or_default(), time);
        }
        for (key, tool) in &other.tools {
            add_tool(
                self.tools.entry(key.clone()).or_insert_with(|| ToolTime {
                    tool: tool.tool.clone(),
                    ..ToolTime::default()
                }),
                tool,
            );
        }
        for (key, counts) in &other.outcomes {
            let into = self.outcomes.entry(key.clone()).or_default();
            for (outcome, count) in counts {
                *into.entry(outcome.clone()).or_default() += count;
            }
        }
        self.dropped += other.dropped;
        self.unreadable += other.unreadable;
    }

    /// Folds the day buckets more than [`DAYS_KEPT`] days before `today`
    /// into their months. Doing it again, or later, folds nothing twice.
    pub fn fold_days(&mut self, today: LocalDate) {
        let oldest_kept = today.add_days(-DAYS_KEPT);
        let month_of = |period: &str| -> Option<String> {
            let date = LocalDate::parse(period)?;
            (date < oldest_kept).then(|| date.month_text())
        };
        fold_map(
            &mut self.usage,
            |key| {
                month_of(&key.period).map(|period| UsageKey {
                    period,
                    ..key.clone()
                })
            },
            UsageSum::add,
        );
        fold_map(
            &mut self.time,
            |key| {
                month_of(&key.period).map(|period| TimeKey {
                    period,
                    ..key.clone()
                })
            },
            add_time,
        );
        fold_map(
            &mut self.tools,
            |key| {
                month_of(&key.period).map(|period| ToolKey {
                    period,
                    ..key.clone()
                })
            },
            add_tool,
        );
    }
}

/// Moves each entry whose key `refold` gives a new key for to that key,
/// adding it to what is there.
fn fold_map<K: Ord + Clone, V: Default + Clone>(
    map: &mut BTreeMap<K, V>,
    refold: impl Fn(&K) -> Option<K>,
    add: impl Fn(&mut V, &V),
) {
    let moving: Vec<(K, K)> = map
        .keys()
        .filter_map(|key| refold(key).map(|to| (key.clone(), to)))
        .collect();
    for (from, to) in moving {
        if let Some(value) = map.remove(&from) {
            add(map.entry(to).or_default(), &value);
        }
    }
}

/// Maps with structured keys, written as a list of `{"key": …, "value": …}`.
mod entries {
    use std::collections::BTreeMap;

    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Entry<K, V> {
        key: K,
        value: V,
    }

    pub(super) fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        serializer.collect_seq(map.iter().map(|(key, value)| Entry { key, value }))
    }

    pub(super) fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: DeserializeOwned + Ord,
        V: DeserializeOwned,
        D: Deserializer<'de>,
    {
        let entries: Vec<Entry<K, V>> = Vec::deserialize(deserializer)?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.key, entry.value))
            .collect())
    }
}
