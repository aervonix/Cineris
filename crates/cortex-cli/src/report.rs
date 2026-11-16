//! What a headless run reports: the result record, gathered from the
//! session's events as they pass, and what each output format writes
//! (`docs/architecture/10-headless.md`, "Output formats").
//!
//! Nothing here does I/O. The driver feeds it events and writes the lines it
//! makes, so every format is tested on events written by hand.

use std::collections::BTreeSet;

use cortex_protocol::event::{CheckReport, Event, Notice, TurnOutcome, VerificationPhase};
use cortex_protocol::headless::{
    HEADLESS_SCHEMA, HeadlessRecord, HeadlessResult, HeadlessStart, HeadlessVerification,
    PermissionDenial,
};
use cortex_protocol::item::Item;
use cortex_protocol::message::Part;
use cortex_protocol::usage::TokenUsage;
use cortex_protocol::{ErrorData, ModelId, SessionEvent, SessionId, TurnId};
use serde_json::Value;

/// What a run has shown so far.
#[derive(Debug, Default)]
pub(crate) struct Collector {
    session: Option<SessionId>,
    /// The turn this run started: events of other turns are not its own.
    turn: Option<TurnId>,
    /// The text of the last assistant message that had any.
    result: String,
    tool_calls: u64,
    files: BTreeSet<String>,
    /// The turn's end, once it has come.
    end: Option<cortex_protocol::event::TurnCompleted>,
    /// The rounds of the project's checks that ended, and the last one's
    /// checks.
    rounds: u32,
    checks: Vec<CheckReport>,
}

impl Collector {
    /// A collector for a run in `session`.
    pub(crate) fn new(session: SessionId) -> Self {
        Self {
            session: Some(session),
            ..Self::default()
        }
    }

    /// Takes one event. Says whether it ends the run: the end of the run's
    /// own turn.
    pub(crate) fn observe(&mut self, event: &SessionEvent) -> bool {
        if self.session.as_ref() != Some(&event.session_id) {
            return false;
        }
        if let Event::TurnStarted(_) = &event.event
            && self.turn.is_none()
        {
            self.turn.clone_from(&event.turn_id);
        }
        // Events of a turn this run did not start (another client's, on a
        // resumed session) are not this run's.
        if event.turn_id.is_some() && self.turn.is_some() && event.turn_id != self.turn {
            return false;
        }
        match &event.event {
            Event::ItemCompleted(completed) => match &completed.item {
                Item::AssistantMessage(message) => {
                    let text = text_of(&message.parts);
                    if !text.is_empty() {
                        self.result = text;
                    }
                }
                Item::ToolResult(result) => {
                    self.tool_calls += 1;
                    if let Some(payload) = &result.payload {
                        changed_files(payload, &mut self.files);
                    }
                }
                _ => {}
            },
            Event::Notice(Notice::Verification(notice))
                if notice.phase == VerificationPhase::Finished =>
            {
                self.rounds += 1;
                self.checks.clone_from(&notice.checks);
            }
            Event::TurnCompleted(end) if self.turn.is_some() => {
                self.end = Some(end.clone());
                return true;
            }
            _ => {}
        }
        false
    }

    /// The session the run is in.
    pub(crate) fn session(&self) -> Option<&SessionId> {
        self.session.as_ref()
    }

    /// The result record. `failure` is an error that ended the run outside
    /// the turn (before it started, or while it was being asked to start),
    /// with the outcome it has.
    pub(crate) fn result(
        &self,
        failure: Option<(TurnOutcome, ErrorData)>,
        denials: Vec<PermissionDenial>,
        duration_ms: u64,
    ) -> HeadlessResult {
        let mut result = HeadlessResult {
            schema: HEADLESS_SCHEMA,
            session_id: self.session.clone(),
            outcome: TurnOutcome::Failed,
            result: self.result.clone(),
            stop_reason: None,
            error: None,
            usage: TokenUsage::default(),
            costs: Vec::new(),
            requests: 0,
            tool_calls: self.tool_calls,
            duration_ms,
            files_changed: self.files.iter().cloned().collect(),
            permission_denials: denials,
            verification: None,
        };
        if let Some(end) = &self.end {
            result.outcome = end.outcome.clone();
            result.stop_reason.clone_from(&end.stop_reason);
            result.error.clone_from(&end.error);
            result.usage = end.usage.tokens.clone();
            result.costs.clone_from(&end.usage.costs);
            result.requests = end.usage.requests;
            result.verification = end
                .verification
                .clone()
                .map(|verdict| HeadlessVerification {
                    verdict,
                    rounds: self.rounds,
                    checks: self.checks.clone(),
                });
        }
        if let Some((outcome, error)) = failure {
            result.outcome = outcome;
            result.error = Some(error);
        }
        result
    }
}

/// The first line of `jsonl`.
pub(crate) fn start_line(session: SessionId, model: ModelId, workspace: String) -> String {
    line(serde_json::to_string(&HeadlessRecord::Session(
        HeadlessStart {
            schema: HEADLESS_SCHEMA,
            session_id: session,
            model,
            workspace,
        },
    )))
}

/// The last line of `jsonl`, and the whole of `json`.
pub(crate) fn result_line(result: HeadlessResult) -> String {
    line(serde_json::to_string(&HeadlessRecord::Result(result)))
}

/// An event as a line of `jsonl`: the shape of a `session/event`
/// notification's parameters.
pub(crate) fn event_line(event: &SessionEvent) -> String {
    line(serde_json::to_string(event))
}

/// What `text` writes: the assistant's last message and a newline, or
/// nothing when it wrote none.
pub(crate) fn text_output(result: &HeadlessResult) -> String {
    if result.result.is_empty() {
        String::new()
    } else if result.result.ends_with('\n') {
        result.result.clone()
    } else {
        format!("{}\n", result.result)
    }
}

/// One JSON object on one line, safe at a terminal: [`crate::json::record`].
fn line(json: Result<String, serde_json::Error>) -> String {
    crate::json::record(json)
}

fn text_of(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// The kinds of an editing tool's payload that say its file was written.
/// `read` names a path too, with kinds of its own, and changes nothing.
const WRITTEN: &[&str] = &["create", "write", "edit"];

/// The files a tool's payload says it changed: `path` for the editing tools
/// when they wrote it, and the shell tool's `changes`.
fn changed_files(payload: &Value, files: &mut BTreeSet<String>) {
    if let Some(path) = payload.get("path").and_then(Value::as_str)
        && payload
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| WRITTEN.contains(&kind))
    {
        files.insert(path.to_owned());
    }
    if let Some(changes) = payload.get("changes") {
        for list in ["modified", "created", "deleted"] {
            for path in changes
                .get(list)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                files.insert(path.to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(seq: u64, turn: Option<&str>, event: Value) -> SessionEvent {
        let mut value = json!({"session_id": "s_1", "seq": seq});
        if let Some(turn) = turn {
            value["turn_id"] = json!(turn);
        }
        for (key, field) in event.as_object().unwrap() {
            value[key] = field.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    fn assistant(id: &str, text: &str) -> Value {
        json!({"type": "item.completed", "item": {"kind": "assistant_message", "id": id,
            "parts": [{"kind": "text", "text": text}], "stop_reason": "end"}})
    }

    fn tool(id: &str, payload: Value) -> Value {
        json!({"type": "item.completed", "item": {"kind": "tool_result", "id": id,
            "call_id": "c", "tool": "write", "outcome": "success", "content": [],
            "payload": payload}})
    }

    fn end(outcome: &str) -> Value {
        json!({"type": "turn.completed", "outcome": outcome, "stop_reason": "end",
            "usage": {"tokens": {"input": 10, "cache_read": 2, "cache_write": 1, "output": 5},
                "requests": 2, "costs": [{"usd_nanos": 1500, "basis": "metered"}]}})
    }

    fn round(round: u32, outcome: &str) -> Value {
        json!({"type": "notice", "kind": "verification", "message": "The checks ended.",
            "phase": "finished", "round": round, "rounds": 2,
            "checks": [{"command": "cargo test", "source": "user", "outcome": outcome}],
            "verdict": if outcome == "passed" { "passed" } else { "failed" }})
    }

    #[test]
    fn the_result_says_what_the_checks_said_of_the_turn() {
        let mut collector = Collector::new(SessionId::from("s_1"));
        let mut ended = end("completed");
        ended["verification"] = json!("passed");
        let events = [
            event(1, Some("t_1"), json!({"type": "turn.started"})),
            event(
                2,
                Some("t_1"),
                json!({"type": "notice", "kind": "verification",
                "message": "Running the checks.", "phase": "started", "round": 1, "rounds": 2,
                "checks": [{"command": "cargo test", "source": "user"}]}),
            ),
            event(3, Some("t_1"), round(1, "failed")),
            event(4, Some("t_1"), round(2, "passed")),
            event(5, Some("t_1"), ended),
        ];
        for one in &events {
            collector.observe(one);
        }
        let result = collector.result(None, Vec::new(), 10);
        let verification = result.verification.expect("the verdict is in the result");
        assert_eq!(verification.verdict.as_str(), "passed");
        assert_eq!(verification.rounds, 2, "the rounds that ended");
        assert_eq!(verification.checks.len(), 1);
        assert_eq!(
            verification.checks[0]
                .outcome
                .as_ref()
                .map(|outcome| outcome.as_str()),
            Some("passed"),
            "the last round's checks"
        );
        // A turn with no verdict (interrupted) says nothing of it.
        let mut quiet = Collector::new(SessionId::from("s_1"));
        quiet.observe(&event(1, Some("t_1"), json!({"type": "turn.started"})));
        quiet.observe(&event(2, Some("t_1"), end("interrupted")));
        assert_eq!(quiet.result(None, Vec::new(), 10).verification, None);
    }

    #[test]
    fn the_result_is_the_last_text_and_the_turns_own_end() {
        let mut collector = Collector::new(SessionId::from("s_1"));
        let events = [
            event(1, Some("t_1"), json!({"type": "turn.started"})),
            event(2, Some("t_1"), assistant("i_1", "Looking.")),
            event(
                3,
                Some("t_1"),
                tool("i_2", json!({"path": "src/a.rs", "kind": "write"})),
            ),
            event(
                4,
                Some("t_1"),
                tool("i_3", json!({"path": "src/b.rs", "kind": "unchanged"})),
            ),
            event(
                4,
                Some("t_1"),
                tool("i_3", json!({"path": "src/c.rs", "kind": "content"})),
            ),
            event(
                4,
                Some("t_1"),
                tool("i_3", json!({"path": "src/d.rs", "kind": "edit"})),
            ),
            event(
                5,
                Some("t_1"),
                tool(
                    "i_4",
                    json!({"changes": {"modified": ["z.txt"], "created": ["a.txt"], "deleted": []}}),
                ),
            ),
            event(6, Some("t_1"), assistant("i_5", "Done.")),
            // A message with no text does not replace the last one that had.
            event(
                7,
                Some("t_1"),
                json!({"type": "item.completed", "item": {
                "kind": "assistant_message", "id": "i_6", "parts": [], "stop_reason": "end"}}),
            ),
        ];
        for event in &events {
            assert!(!collector.observe(event));
        }
        assert!(collector.observe(&event(8, Some("t_1"), end("completed"))));
        let result = collector.result(None, Vec::new(), 42);
        assert_eq!(result.outcome, TurnOutcome::Completed);
        assert_eq!(result.result, "Done.");
        assert_eq!(result.tool_calls, 5);
        assert_eq!(
            result.files_changed,
            ["a.txt", "src/a.rs", "src/d.rs", "z.txt"]
        );
        assert_eq!(result.requests, 2);
        assert_eq!(result.usage.input, 10);
        assert_eq!(result.costs[0].usd_nanos, 1500);
        assert_eq!(result.duration_ms, 42);
        assert_eq!(text_output(&result), "Done.\n");
    }

    #[test]
    fn another_turns_events_are_not_the_runs() {
        let mut collector = Collector::new(SessionId::from("s_1"));
        assert!(!collector.observe(&event(1, Some("t_1"), json!({"type": "turn.started"}))));
        assert!(!collector.observe(&event(2, Some("t_0"), assistant("i_1", "Old."))));
        assert!(!collector.observe(&event(3, Some("t_0"), end("completed"))));
        assert!(!collector.observe(&SessionEvent {
            session_id: SessionId::from("s_2"),
            ..event(4, Some("t_1"), end("completed"))
        }));
        assert_eq!(collector.result(None, Vec::new(), 0).result, "");
        assert!(collector.observe(&event(5, Some("t_1"), end("failed"))));
    }

    #[test]
    fn a_failure_outside_the_turn_is_the_results_outcome_and_error() {
        let collector = Collector::default();
        let error = ErrorData::new(cortex_protocol::ErrorKind::Auth, "no_key", "No key.");
        let result = collector.result(Some((TurnOutcome::Failed, error.clone())), Vec::new(), 1);
        assert_eq!(result.session_id, None);
        assert_eq!(result.error, Some(error));
        assert_eq!(text_output(&result), "");
    }

    #[test]
    fn every_line_is_one_line_of_json_with_its_type() {
        let start = start_line(
            SessionId::from("s_1"),
            ModelId::from("m"),
            "/w\nx".to_owned(),
        );
        let result = result_line(Collector::default().result(None, Vec::new(), 0));
        for (line, kind) in [(start, "session"), (result, "result")] {
            assert!(line.ends_with('\n'));
            assert_eq!(line.matches('\n').count(), 1, "{line}");
            let value: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(value["type"], kind);
            assert_eq!(value["schema"], 1);
        }
    }

    /// Whether `text` is a string somewhere in `value`.
    fn holds(value: &Value, text: &str) -> bool {
        match value {
            Value::String(found) => found == text,
            Value::Array(items) => items.iter().any(|item| holds(item, text)),
            Value::Object(fields) => fields.values().any(|field| holds(field, text)),
            _ => false,
        }
    }

    /// Headless mode's `json` and `jsonl` are JSON with nothing in them
    /// that acts unseen at a terminal, whatever a workspace's name or a
    /// model's text holds (the follow-ups of step 23, which found the
    /// subcommands' `--json` printing the terminal's escape, and these
    /// printing the characters themselves).
    #[test]
    fn every_line_shows_nothing_that_acts_unseen() {
        for name in cortex_testkit::hostile::all() {
            let text = format!("a{}b", name.text);
            let start = start_line(SessionId::from("s_1"), ModelId::from("m"), text.clone());
            let event = event_line(&event(1, Some("t_1"), assistant("i_1", &text)));
            for line in [start, event] {
                let back = crate::json::tests::parsed(&line);
                assert!(holds(&back[0], &text), "{}: {line}", name.label);
            }
        }
    }
}
