//! The core's events onto the transcript, block by block.

use std::time::{Duration, Instant};

use cineris_protocol::event::{
    Delta, Event, ItemCompleted, ItemDelta, ItemKind, ItemRef, ItemStarted, TextDelta,
    ToolArgsDelta, ToolCallDelta,
};
use cineris_protocol::item::{
    AssistantMessageItem, NoticeItem, NoticeSource, StopReason, SummaryItem, ToolOutcome,
    ToolResultItem,
};
use cineris_protocol::message::{Part, ToolCallPart};
use cineris_protocol::{CallId, Item, ItemId, SessionId};
use cineris_term::{ColorDepth, Line};
use serde_json::json;

use super::*;
use crate::glyphs::Glyphs;
use crate::theme::Theme;
use crate::view::{CallState, Look, Measure};

fn look() -> Look {
    Look::new(Theme::new(ColorDepth::Mono, None), &Glyphs::UNICODE)
}

struct Rig {
    mapper: Mapper,
    transcript: Transcript,
    now: Instant,
    scrollback: Vec<String>,
}

impl Rig {
    fn new() -> Self {
        Self {
            mapper: Mapper::new(Highlighter::off()),
            transcript: Transcript::new(),
            now: Instant::now(),
            scrollback: Vec::new(),
        }
    }

    fn event(&mut self, event: Event) {
        self.mapper.event(
            &mut self.transcript,
            &event,
            &SessionId::from("s_test"),
            self.now,
        );
    }

    fn later(&mut self, by: Duration) {
        self.now += by;
    }

    fn started(&mut self, item: &str) {
        self.event(Event::ItemStarted(ItemStarted {
            item_id: ItemId::from(item),
            kind: ItemKind::AssistantMessage,
            tool: None,
            call_id: None,
        }));
    }

    fn delta(&mut self, item: &str, delta: Delta) {
        self.event(Event::ItemDelta(ItemDelta {
            item_id: ItemId::from(item),
            delta,
        }));
    }

    fn text(&mut self, item: &str, text: &str) {
        self.delta(
            item,
            Delta::Text(TextDelta {
                text: text.to_owned(),
            }),
        );
    }

    fn reasoning(&mut self, item: &str, text: &str) {
        self.delta(
            item,
            Delta::Reasoning(TextDelta {
                text: text.to_owned(),
            }),
        );
    }

    fn call(&mut self, item: &str, call: &str, tool: &str) {
        self.delta(
            item,
            Delta::ToolCall(ToolCallDelta {
                call_id: CallId::from(call),
                name: tool.to_owned(),
            }),
        );
    }

    fn completed(&mut self, item: Item) {
        self.event(Event::ItemCompleted(ItemCompleted { item }));
    }

    fn running(&mut self, result: &str, call: &str, tool: &str) {
        self.event(Event::ItemStarted(ItemStarted {
            item_id: ItemId::from(result),
            kind: ItemKind::ToolResult,
            tool: Some(tool.to_owned()),
            call_id: Some(CallId::from(call)),
        }));
    }

    fn result(&mut self, result: &str, call: &str, tool: &str, outcome: ToolOutcome) {
        self.completed(Item::ToolResult(ToolResultItem {
            id: ItemId::from(result),
            call_id: CallId::from(call),
            tool: tool.to_owned(),
            outcome,
            content: vec![Part::text("what the model saw")],
            payload: Some(json!({"first_line": 1, "last_line": 9, "exit_code": 0})),
            artifacts: Vec::new(),
            duration_ms: Some(1200),
            created_at: None,
            replaced: None,
        }));
    }

    /// The lines committed so far, and the live rows now.
    fn frame(&mut self) -> Vec<String> {
        self.mapper.tick(&mut self.transcript, self.now);
        let look = look();
        let committed: Vec<Line> = self.transcript.commit(80, &look);
        self.scrollback.extend(
            committed
                .iter()
                .map(|line| line.text().trim_end().to_owned()),
        );
        self.transcript
            .live(80, &look, 0)
            .iter()
            .map(|line| line.text().trim_end().to_owned())
            .collect()
    }

    fn calls(&mut self) -> Vec<crate::view::Call> {
        let ids = self.mapper.live_rounds(&mut self.transcript);
        let mut calls = Vec::new();
        for (number, round) in &self.mapper.rounds {
            if ids.contains(number)
                && let Some(Block::Round(round)) = self.transcript.get_mut(round.block)
            {
                calls.extend(round.calls.iter().cloned());
            }
        }
        calls
    }
}

fn assistant(item: &str, parts: Vec<Part>, stop: StopReason) -> Item {
    Item::AssistantMessage(AssistantMessageItem {
        id: ItemId::from(item),
        parts,
        stop_reason: Some(stop),
        raw_stop: None,
        stop_detail: None,
        truncated: false,
        origin: None,
        usage: None,
        error: None,
        created_at: None,
    })
}

#[test]
fn reasoning_ends_when_the_answer_begins_and_says_how_long_it_took() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.reasoning("i_1", "Look at the scanner first.");
    let live = rig.frame();
    assert!(
        live.iter().any(|row| row.contains("Reasoning")),
        "{live:#?}"
    );
    rig.later(Duration::from_secs(6));
    rig.text("i_1", "The scanner stops early.\n\n");
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("Reasoned for 6s"), "{all}");
    assert!(all.contains("Look at the scanner first."), "{all}");
}

#[test]
fn an_answer_ends_where_a_call_begins_and_text_after_it_is_a_new_answer() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.text("i_1", "I'll read it.");
    rig.call("i_1", "c_1", "read");
    rig.text("i_1", "Then I'll fix it.");
    let _ = rig.frame();
    // The first answer is finished, so it is in the scrollback above the
    // round, which is still live.
    assert!(rig.scrollback.join("\n").contains("I'll read it."));
    let live = rig.frame();
    let round = live
        .iter()
        .position(|row| row.contains("Read"))
        .expect("the round");
    let then = live
        .iter()
        .position(|row| row.contains("Then I'll fix it."))
        .expect("the second answer");
    assert!(round < then, "{live:#?}");
}

#[test]
fn a_call_takes_its_target_from_its_arguments_and_its_measure_from_its_result() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.call("i_1", "c_1", "read");
    rig.delta(
        "i_1",
        Delta::ToolArgs(ToolArgsDelta {
            call_id: CallId::from("c_1"),
            json: "{\"path\": \"src/lex.rs\"}".to_owned(),
        }),
    );
    rig.completed(assistant(
        "i_1",
        vec![Part::ToolCall(ToolCallPart {
            id: CallId::from("c_1"),
            name: "read".to_owned(),
            arguments: json!({"path": "src/lex.rs"}),
            raw: None,
        })],
        StopReason::ToolCalls,
    ));
    let calls = rig.calls();
    assert_eq!(
        calls[0].target,
        crate::view::Target::Path("src/lex.rs".to_owned())
    );
    rig.running("i_2", "c_1", "read");
    rig.later(Duration::from_secs(3));
    let _ = rig.frame();
    let calls = rig.calls();
    assert_eq!(calls[0].state, CallState::Running(Duration::from_secs(3)));
    rig.result("i_2", "c_1", "read", ToolOutcome::Success);
    // The round has ended: it is committed whole.
    assert!(rig.calls().is_empty());
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("Read lex.rs"), "{all}");
    assert!(all.contains("lines 1"), "{all}");
}

#[test]
fn a_running_commands_output_is_kept_to_its_tail() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.call("i_1", "c_1", "shell");
    rig.running("i_2", "c_1", "shell");
    let line = "x".repeat(1023) + "\n";
    for _ in 0..200 {
        rig.delta("i_2", Delta::ToolOutput(TextDelta { text: line.clone() }));
    }
    let calls = rig.calls();
    let crate::view::Detail::Output(output) = &calls[0].detail else {
        panic!("the output");
    };
    assert!(output.len() <= OUTPUT_TAIL, "{} bytes kept", output.len());
    assert!(output.starts_with('x'), "cut at a line's start");
}

#[test]
fn an_answer_whose_deltas_stopped_is_completed_from_the_whole_item() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.text("i_1", "The first part, ");
    rig.event(Event::ItemResync(ItemRef {
        item_id: ItemId::from("i_1"),
    }));
    // A delta after the resync is not to be had: deltas stop until the end.
    rig.text("i_1", "never");
    rig.completed(assistant(
        "i_1",
        vec![Part::text("The first part, and the second.\n")],
        StopReason::End,
    ));
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("The first part, and the second."), "{all}");
    assert!(!all.contains("never"), "{all}");
    assert_eq!(
        rig.mapper.last_answer(),
        "The first part, and the second.\n"
    );
}

/// Each reasoning block knows where its whole text is: its session, its
/// item and which of the item's reasoning runs it is, as
/// `reasoning_runs` reads them from the item; its words are held once,
/// moved from the stream when it ends; and a response thrown away leaves
/// its reasoning findable by no read (since the fixes of the review of step
/// 25).
#[test]
fn each_reasoning_knows_its_run_of_its_item_and_holds_its_words_once() {
    let mut rig = Rig::new();
    rig.transcript.keep_finished();
    rig.started("i_1");
    rig.reasoning("i_1", "  First, the scanner");
    rig.reasoning("i_1", " and its numbers.\n");
    rig.text("i_1", "I'll read it.");
    rig.reasoning("i_1", "Then the parser.");
    rig.call("i_1", "c_1", "read");
    rig.reasoning("i_1", "Last, the tests.");
    let words = |text: &str| {
        Part::Reasoning(cineris_protocol::message::ReasoningPart {
            text: text.to_owned(),
            raw: None,
        })
    };
    let parts = vec![
        words("  First, the scanner"),
        words(" and its numbers.\n"),
        // A provider's block with no words, which no delta drew.
        words(""),
        Part::text("I'll read it."),
        words("Then the parser."),
        Part::ToolCall(ToolCallPart {
            id: CallId::from("c_1"),
            name: "read".to_owned(),
            arguments: json!({"path": "src/lex.rs"}),
            raw: None,
        }),
        words("Last, the tests."),
    ];
    rig.completed(assistant("i_1", parts.clone(), StopReason::ToolCalls));
    rig.result("i_2", "c_1", "read", ToolOutcome::Success);
    let _ = rig.frame();
    let runs = super::reasoning_runs(&parts);
    assert_eq!(
        runs,
        [
            "First, the scanner and its numbers.",
            "Then the parser.",
            "Last, the tests."
        ]
    );
    let finished = |rig: &mut Rig| -> Vec<Reasoning> {
        rig.transcript
            .take_finished()
            .into_iter()
            .filter_map(|finished| match finished.block {
                Block::Reasoning(reasoning) => Some(reasoning),
                _ => None,
            })
            .collect()
    };
    let kept = finished(&mut rig);
    assert_eq!(kept.len(), 3, "{kept:#?}");
    for (n, reasoning) in kept.iter().enumerate() {
        assert_eq!(
            reasoning.origin,
            Some(Origin {
                session: SessionId::from("s_test"),
                item: ItemId::from("i_1"),
                run: n,
            })
        );
        assert!(reasoning.text.is_empty(), "held twice: {reasoning:#?}");
        assert_eq!(reasoning.summary.as_deref(), Some(runs[n].as_str()));
    }
    // A response thrown away: the reasoning it drew is no read's to find.
    rig.started("i_3");
    rig.reasoning("i_3", "A first try.");
    rig.event(Event::ItemReset(ItemRef {
        item_id: ItemId::from("i_3"),
    }));
    let _ = rig.frame();
    let tried = finished(&mut rig);
    assert_eq!(tried.len(), 1, "{tried:#?}");
    assert_eq!(tried[0].origin, None);
}

/// The mapper's blocks and `reasoning_runs` are decided by one rule, so
/// that a block's run is the one the viewer reads again: a reasoning part
/// with no words (a signature, an encrypted payload) and an answer's text
/// with none neither begin nor end a run, as an item read in progress
/// replays them (the independent review of the fixes of step 25: the
/// mapper opened a block for an empty reasoning and ended one on empty
/// text, so the viewer showed the next run as the block's, and the last
/// could not be read).
#[test]
fn a_part_with_no_words_neither_begins_nor_ends_a_reasoning_run() {
    let mut rig = Rig::new();
    rig.transcript.keep_finished();
    let words = |text: &str| {
        Part::Reasoning(cineris_protocol::message::ReasoningPart {
            text: text.to_owned(),
            raw: None,
        })
    };
    let call = |id: &str| {
        Part::ToolCall(ToolCallPart {
            id: CallId::from(id),
            name: "read".to_owned(),
            arguments: json!({"path": "src/lex.rs"}),
            raw: None,
        })
    };
    let parts = vec![
        words(""),
        Part::text(""),
        words("First"),
        Part::text(""),
        words(", and more."),
        call("c_1"),
        words(""),
        words("Second."),
        call("c_2"),
        words("Third."),
    ];
    let item = assistant("i_1", parts.clone(), StopReason::ToolCalls);
    rig.mapper.in_progress(
        &mut rig.transcript,
        &item,
        &SessionId::from("s_test"),
        rig.now,
    );
    rig.completed(item);
    rig.result("i_2", "c_1", "read", ToolOutcome::Success);
    rig.result("i_3", "c_2", "read", ToolOutcome::Success);
    let _ = rig.frame();
    let runs = super::reasoning_runs(&parts);
    assert_eq!(runs, ["First, and more.", "Second.", "Third."]);
    let kept: Vec<Reasoning> = rig
        .transcript
        .take_finished()
        .into_iter()
        .filter_map(|finished| match finished.block {
            Block::Reasoning(reasoning) => Some(reasoning),
            _ => None,
        })
        .collect();
    let drawn: Vec<(usize, Option<String>)> = kept
        .iter()
        .map(|reasoning| {
            (
                reasoning
                    .origin
                    .as_ref()
                    .map_or(usize::MAX, |origin| origin.run),
                reasoning.summary.clone(),
            )
        })
        .collect();
    assert_eq!(
        drawn,
        runs.iter()
            .enumerate()
            .map(|(n, run)| (n, Some(run.clone())))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_response_thrown_away_ends_where_it_was_and_its_calls_never_ran() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.text("i_1", "A first try");
    rig.call("i_1", "c_1", "edit");
    rig.event(Event::ItemReset(ItemRef {
        item_id: ItemId::from("i_1"),
    }));
    // Its call never ran: the round is over.
    assert!(rig.calls().is_empty());
    // The retried response streams afresh.
    rig.text("i_1", "The second try.\n");
    rig.completed(assistant(
        "i_1",
        vec![Part::text("The second try.\n")],
        StopReason::End,
    ));
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("A first try"), "{all}");
    assert!(all.contains("■ Edit"), "{all}");
    assert!(all.contains("The second try."), "{all}");
}

#[test]
fn a_turn_that_ends_interrupts_whatever_still_runs() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.call("i_1", "c_1", "shell");
    rig.running("i_2", "c_1", "shell");
    rig.mapper.turn_done(&mut rig.transcript, rig.now);
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("■ Run"), "{all}");
}

#[test]
fn a_stopped_response_runs_none_of_its_calls() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.call("i_1", "c_1", "write");
    rig.completed(assistant("i_1", Vec::new(), StopReason::Interrupted));
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("■ Write"), "{all}");
}

#[test]
fn the_map_is_shown_folded_and_a_summary_as_a_notice() {
    let mut rig = Rig::new();
    rig.completed(Item::Notice(NoticeItem {
        id: ItemId::from("i_1"),
        source: NoticeSource::Map,
        text: "src/\n  lex.rs\n  main.rs".to_owned(),
        created_at: None,
    }));
    // The model's own notices are not drawn: the events tell the person.
    rig.completed(Item::Notice(NoticeItem {
        id: ItemId::from("i_2"),
        source: NoticeSource::Plan,
        text: "The user edited the plan.".to_owned(),
        created_at: None,
    }));
    rig.completed(Item::Summary(SummaryItem {
        id: ItemId::from("i_3"),
        text: "What happened so far.".to_owned(),
        covers_through: None,
        usage: None,
        created_at: None,
    }));
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(
        all.contains("The repository map was given to the model (3 lines)."),
        "{all}"
    );
    assert!(!all.contains("src/"), "the map itself is not drawn: {all}");
    assert!(!all.contains("edited the plan"), "{all}");
    assert!(all.contains("summarised"), "{all}");
}

#[test]
fn a_result_for_a_call_never_seen_gets_a_box_of_its_own() {
    let mut rig = Rig::new();
    rig.result("i_9", "c_9", "read", ToolOutcome::Error);
    let _ = rig.frame();
    let all = rig.scrollback.join("\n");
    assert!(all.contains("✗ Read"), "{all}");
}

#[test]
fn ctrl_t_opens_and_closes_the_running_rounds_box() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.call("i_1", "c_1", "shell");
    rig.running("i_2", "c_1", "shell");
    rig.delta(
        "i_2",
        Delta::ToolOutput(TextDelta {
            text: "compiling\n".to_owned(),
        }),
    );
    let before = rig.frame();
    rig.mapper.toggle_round(&mut rig.transcript, None, &look());
    let after = rig.frame();
    assert_ne!(before, after);
    rig.mapper.toggle_round(&mut rig.transcript, None, &look());
    assert_eq!(rig.frame(), before);
    let measure = rig.calls()[0].measure.clone();
    assert_eq!(measure, Measure::None);
}

/// Output that streamed in part, its deltas then stopped (`item.resync`):
/// the box shows the result's own text, which is whole.
#[test]
fn a_commands_output_whose_deltas_stopped_is_taken_from_its_result() {
    let mut rig = Rig::new();
    rig.started("i_1");
    rig.call("i_1", "c_1", "shell");
    // A second call keeps the round open, so that its box can be read.
    rig.call("i_1", "c_2", "shell");
    rig.running("i_2", "c_1", "shell");
    rig.delta(
        "i_2",
        Delta::ToolOutput(TextDelta {
            text: "the part that streamed\n".to_owned(),
        }),
    );
    rig.event(Event::ItemResync(ItemRef {
        item_id: ItemId::from("i_2"),
    }));
    rig.result("i_2", "c_1", "shell", ToolOutcome::Success);
    let calls = rig.calls();
    let crate::view::Detail::Output(output) = &calls[0].detail else {
        panic!("the output: {:?}", calls[0].detail);
    };
    assert!(output.contains("what the model saw"), "{output}");
    assert!(!output.contains("streamed"), "{output}");
}
