//! The statistics store against its promises: where events go, that the
//! aggregates rebuild to the same values, that all-time figures outlive the
//! raw files, that ranges are right across day, week, month and time-zone
//! boundaries, and that nothing is read until asked.

#![allow(
    clippy::unwrap_used,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stdout,
    reason = "tests, which read and write the files they check, and say when they skip"
)]

mod day;
mod limits;
mod ranges;
mod store;

use std::sync::Arc;

use cortex_base::{FakeClock, SharedClock, UtcTime, Zone};
use cortex_protocol::event::TurnOutcome;
use cortex_protocol::model::ModelRole;
use cortex_protocol::usage::{Billing, Cost, CostBasis, ModelTime, TokenUsage};
use cortex_protocol::{ModelId, ProjectKey, ProviderId, SessionId, Speed, Timestamp, TurnId};
use cortex_stats::{
    Purpose, Record, Recorder, RecorderOptions, RequestRecord, StatsDir, ToolOutcome, ToolRecord,
    ToolShare, TurnRecord, Variants,
};
use cortex_testkit::Scratch;

/// The entry test: a child in a role when its arguments say so (a second
/// process for the tests of what two processes do), and nothing otherwise.
#[test]
fn child_entry() {
    if let Some(dir) = day::WRITER.options() {
        day::write_one(&dir);
        #[expect(clippy::exit, reason = "a child ends when its part is played")]
        std::process::exit(0);
    }
}

pub(crate) fn scratch(test: &str) -> Scratch {
    cortex_testkit::scratch!(test)
}

pub(crate) fn clock(at: &str) -> Arc<FakeClock> {
    Arc::new(FakeClock::starting_at(at).unwrap())
}

pub(crate) fn time(at: &str) -> UtcTime {
    UtcTime::parse(at).unwrap()
}

pub(crate) fn variants() -> Variants {
    Variants {
        edit_format: "replace".to_owned(),
        tool_schema: "strict".to_owned(),
        prompt_style: "concise".to_owned(),
        effort: Some("high".to_owned()),
    }
}

/// A request at `at` that cost `nanos`, of a project and a model.
pub(crate) fn request(at: &str, project: &str, model: &str, nanos: u64) -> Record {
    Record::Request(RequestRecord {
        at: Timestamp::new(at),
        session: SessionId::new("s_1"),
        turn: TurnId::new("t_1"),
        started_in: None,
        project: Some(ProjectKey::new(project)),
        provider: ProviderId::new("anthropic"),
        model: ModelId::new(model),
        role: ModelRole::Main,
        purpose: Purpose::Turn,
        speed: Speed::Standard,
        speed_requested: None,
        variants: variants(),
        tokens: TokenUsage {
            input: 100,
            cache_read: 1000,
            cache_write: 10,
            output: 50,
            reasoning: Some(5),
            ..TokenUsage::default()
        },
        cost: Some(Cost {
            usd_nanos: nanos,
            basis: CostBasis::Metered,
        }),
        catalog_version: Some("2026-11-12".to_owned()),
        billing: Billing::Metered,
        ttft_ms: Some(400),
        tokens_per_second: Some(60.0),
        time: ModelTime::default(),
        stop: "end".to_owned(),
        error: None,
        retries: 0,
        charged_to: None,
    })
}

pub(crate) fn tool(at: &str, project: &str, name: &str) -> Record {
    Record::Tool(ToolRecord {
        at: Timestamp::new(at),
        session: SessionId::new("s_1"),
        turn: TurnId::new("t_1"),
        project: Some(ProjectKey::new(project)),
        tool: name.to_owned(),
        model: ModelId::new("opus"),
        variants: variants(),
        outcome: ToolOutcome::Ok,
        running_ms: 300,
        queued_ms: 20,
        approval_ms: 0,
    })
}

pub(crate) fn turn(at: &str, project: &str) -> Record {
    Record::Turn(TurnRecord {
        at: Timestamp::new(at),
        session: SessionId::new("s_1"),
        turn: TurnId::new("t_1"),
        project: Some(ProjectKey::new(project)),
        provider: ProviderId::new("anthropic"),
        model: ModelId::new("opus"),
        variants: variants(),
        requests: 2,
        tool_calls: 1,
        outcome: TurnOutcome::Completed,
        verification: None,
        wall_ms: 1000,
        model_time: ModelTime {
            connect_ms: 100,
            first_byte_ms: 200,
            streaming_ms: 300,
            retry_ms: 0,
            summary_ms: 0,
        },
        tools_ms: 250,
        approval_ms: 50,
        verification_ms: 0,
        harness_ms: 100,
        by_tool: vec![ToolShare {
            tool: "read".to_owned(),
            ms: 250,
        }],
    })
}

/// A recorder writing to `dir`, its writer started, counting days in
/// `zone`.
pub(crate) fn recorder(dir: &StatsDir, zone: Zone, clock: SharedClock, retention: u32) -> Recorder {
    let mut options = RecorderOptions::new(dir.clone());
    options.zone = Some(zone);
    options.retention = retention;
    let recorder = Recorder::new(options, clock);
    recorder.start().unwrap();
    recorder
}
