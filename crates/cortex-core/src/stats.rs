//! The loop's statistics events to the statistics store: each event made a
//! record of `cortex-stats`, with the session's project, and handed to its
//! recorder's queue (`docs/architecture/14-adaptation.md`, "Local
//! statistics").
//!
//! **This is on the loop's thread, so it only converts and pushes:** a few
//! strings cloned and one try-send. Writing, the local day, the aggregates
//! and the limits file are the recorder's writer thread's.

use cortex_agent::{
    ContextStat, PrefixDifference, RequestPurpose, RequestStat, StatEvent, Stats, ToolCallStat,
    ToolStatOutcome, TurnStat,
};
use cortex_protocol::ProjectKey;
use cortex_stats::{
    ContextRecord, Difference, Purpose, Record, Recorder, RequestRecord, Sources, ToolOutcome,
    ToolRecord, ToolShare, TurnRecord, Variants,
};

/// The statistics of one session: its recorder and its project.
pub(crate) struct SessionStats {
    pub(crate) recorder: Recorder,
    pub(crate) project: ProjectKey,
    /// The session's clock: what an event that carries no time of its own
    /// is stamped with.
    pub(crate) clock: cortex_base::SharedClock,
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// The variants as a record holds them.
fn variants(variants: &cortex_llm::Variants, effort: Option<&String>) -> Variants {
    Variants {
        edit_format: variants.edit_format.clone(),
        tool_schema: variants.tool_schema.as_str().to_owned(),
        prompt_style: variants.prompt_style.clone(),
        effort: effort.cloned(),
    }
}

impl SessionStats {
    fn request(&self, stat: RequestStat) -> Record {
        let usage = stat.usage;
        let tokens_per_second = usage.as_ref().and_then(|usage| {
            let (ttft, duration) = (usage.time_to_first_token_ms?, usage.duration_ms?);
            let streaming = duration.saturating_sub(ttft);
            #[expect(
                clippy::cast_precision_loss,
                reason = "a rate: the counts are far below 2^52"
            )]
            let rate =
                (streaming > 0).then(|| usage.tokens.output as f64 * 1000.0 / streaming as f64);
            rate
        });
        Record::Request(RequestRecord {
            at: stat.at.to_timestamp(),
            session: stat.session,
            turn: stat.turn,
            project: Some(self.project.clone()),
            provider: stat.provider,
            model: stat.model,
            role: stat.role,
            purpose: match stat.purpose {
                RequestPurpose::Summary => Purpose::Summary,
                RequestPurpose::Turn => Purpose::Turn,
            },
            speed: usage
                .as_ref()
                .map(|usage| usage.speed.clone())
                .unwrap_or_default(),
            speed_requested: usage
                .as_ref()
                .and_then(|usage| usage.speed_requested.clone()),
            variants: variants(&stat.variants, stat.effort.as_ref()),
            tokens: usage
                .as_ref()
                .map(|usage| usage.tokens.clone())
                .unwrap_or_default(),
            // The cost was fixed when the request's usage was recorded, at
            // the catalog's prices then: it is carried as it is.
            cost: usage.as_ref().and_then(|usage| usage.cost.clone()),
            catalog_version: usage
                .as_ref()
                .and_then(|usage| usage.catalog_version.clone()),
            billing: usage
                .as_ref()
                .map_or(cortex_protocol::usage::Billing::Unspecified, |usage| {
                    usage.billing.clone()
                }),
            ttft_ms: usage
                .as_ref()
                .and_then(|usage| usage.time_to_first_token_ms),
            tokens_per_second,
            time: stat.time,
            stop: stat.stop.as_str().to_owned(),
            error: stat.error.map(|error| error.code),
            retries: stat.retries,
        })
    }

    fn tool(&self, stat: ToolCallStat, at: cortex_protocol::Timestamp) -> Record {
        Record::Tool(ToolRecord {
            at,
            session: stat.session,
            turn: stat.turn,
            project: Some(self.project.clone()),
            tool: stat.tool,
            model: stat.model,
            variants: variants(&stat.variants, None),
            outcome: match stat.outcome {
                ToolStatOutcome::Ok => ToolOutcome::Ok,
                ToolStatOutcome::InvalidArguments => ToolOutcome::InvalidArguments,
                ToolStatOutcome::UnknownTool => ToolOutcome::UnknownTool,
                ToolStatOutcome::Failed => ToolOutcome::Failed,
                ToolStatOutcome::Denied => ToolOutcome::Denied,
                ToolStatOutcome::Interrupted => ToolOutcome::Interrupted,
            },
            running_ms: millis(stat.duration),
            queued_ms: millis(stat.queued),
            approval_ms: millis(stat.approval),
        })
    }

    fn turn(&self, stat: TurnStat, at: cortex_protocol::Timestamp) -> Record {
        let times = stat.times.totals;
        Record::Turn(TurnRecord {
            at,
            session: stat.session,
            turn: stat.turn,
            project: Some(self.project.clone()),
            provider: stat.provider,
            model: stat.model,
            variants: variants(&stat.variants, stat.effort.as_ref()),
            requests: stat.requests,
            tool_calls: stat.tool_calls,
            outcome: stat.outcome,
            verification: stat.verification,
            wall_ms: times.wall_ms,
            model_time: times.model,
            tools_ms: times.tools_ms,
            approval_ms: times.approval_ms,
            verification_ms: times.verification_ms,
            harness_ms: times.harness_ms,
            by_tool: stat
                .times
                .by_tool
                .into_iter()
                .map(|(tool, ms)| ToolShare { tool, ms })
                .collect(),
        })
    }

    fn context(&self, stat: ContextStat, at: cortex_protocol::Timestamp) -> Record {
        let sources = stat.sources;
        Record::Context(ContextRecord {
            at,
            session: stat.session,
            turn: stat.turn,
            project: Some(self.project.clone()),
            model: stat.model,
            epoch: stat.epoch,
            sources: Sources {
                system: sources.system,
                tools: sources.tools,
                summary: sources.summary,
                user: sources.user,
                assistant: sources.assistant,
                tool_results: sources.tool_results,
                notices: sources.notices,
            },
            estimate: stat.estimate,
            limit: stat.limit,
            window: stat.window,
            difference: match stat.difference {
                PrefixDifference::First => Difference::First,
                PrefixDifference::Extends => Difference::None,
                PrefixDifference::Settings => Difference::Settings,
                PrefixDifference::System => Difference::System,
                PrefixDifference::Tools => Difference::Tools,
                PrefixDifference::Message(index) => Difference::Message {
                    index: index as u64,
                },
            },
            tokens: stat.usage,
        })
    }
}

impl Stats for SessionStats {
    fn record(&self, event: StatEvent) {
        // The events that carry no time of their own are stamped now: the
        // moment the loop hands them over, which is when they ended.
        let now = || self.clock.now().to_timestamp();
        let record = match event {
            StatEvent::Request(stat) => self.request(stat),
            StatEvent::ToolCall(stat) => self.tool(stat, now()),
            StatEvent::Turn(stat) => self.turn(stat, now()),
            StatEvent::Context(stat) => self.context(stat, now()),
            StatEvent::Limits(limits) => {
                self.recorder.limits(limits);
                return;
            }
            _ => return,
        };
        self.recorder.record(record);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use cortex_agent::{RequestPurpose, RequestStat, StatEvent, Stats};
    use cortex_base::{FakeClock, UtcTime};
    use cortex_protocol::model::ModelRole;
    use cortex_protocol::usage::{Billing, Cost, CostBasis, ModelTime, RequestUsage, TokenUsage};
    use cortex_protocol::{ModelId, ProjectKey, ProviderId, SessionId, Speed, StopReason, TurnId};
    use cortex_stats::{Recorder, RecorderOptions, StatsDir};

    use super::SessionStats;

    /// What one request's event costs the loop's thread: the conversion
    /// and the push, with the writer running. Run by hand: `cargo test
    /// --release -p cortex-core --lib perf_recording -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    #[expect(
        clippy::disallowed_methods,
        reason = "a measurement that removes its own directory"
    )]
    fn perf_recording_an_event_on_the_loop() {
        let dir = std::env::temp_dir().join(format!("cortex-perf-stats-{}", std::process::id()));
        let clock = Arc::new(FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap());
        let mut options = RecorderOptions::new(StatsDir::new(dir.clone()));
        options.zone = Some(cortex_base::Zone::utc());
        let recorder = Recorder::new(options, clock.clone());
        recorder.start().unwrap();
        let stats = SessionStats {
            recorder: recorder.clone(),
            project: ProjectKey::new("pk_perf"),
            clock,
        };
        let event = StatEvent::Request(RequestStat {
            session: SessionId::new("s_01JCX4T9Q2"),
            turn: TurnId::new("t_7"),
            at: UtcTime::parse("2026-11-15T10:00:00Z").unwrap(),
            provider: ProviderId::new("anthropic"),
            model: ModelId::new("claude-opus-5-5"),
            role: ModelRole::Main,
            purpose: RequestPurpose::Turn,
            started_in: None,
            variants: cortex_testkit::fixtures::model().variants.clone(),
            effort: Some("high".to_owned()),
            usage: Some(RequestUsage {
                provider: ProviderId::new("anthropic"),
                model: ModelId::new("claude-opus-5-5"),
                role: ModelRole::Main,
                speed: Speed::Standard,
                speed_requested: None,
                tokens: TokenUsage {
                    input: 1200,
                    cache_read: 40_000,
                    cache_write: 900,
                    output: 600,
                    ..TokenUsage::default()
                },
                cost: Some(Cost {
                    usd_nanos: 47_000_000,
                    basis: CostBasis::Metered,
                }),
                catalog_version: Some("2026-11-13".to_owned()),
                billing: Billing::Metered,
                time_to_first_token_ms: Some(900),
                duration_ms: Some(10_000),
                raw: None,
            }),
            time: ModelTime::default(),
            stop: StopReason::ToolCalls,
            error: None,
            retries: 0,
        });
        let mut times: Vec<Duration> = Vec::with_capacity(20_000);
        for round in 0..20_000 {
            let event = event.clone();
            let started = Instant::now();
            stats.record(event);
            times.push(started.elapsed());
            if round % 2_000 == 0 {
                // The writer keeps up, as it does at a loop's pace.
                recorder.flush();
            }
        }
        times.sort();
        println!(
            "recording one request's event: median {:?}, 99th {:?}, most {:?}; dropped {}",
            times[times.len() / 2],
            times[times.len() * 99 / 100],
            times[times.len() - 1],
            recorder.dropped()
        );
        recorder.flush();
        let _ = std::fs::remove_dir_all(dir);
    }
}
