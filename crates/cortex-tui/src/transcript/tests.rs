use std::time::Duration;

use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::DARK;
use crate::theme::Theme;
use crate::view::{Call, CallState, Detail, Measure, Target};

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn texts(lines: &[Line]) -> Vec<String> {
    lines.iter().map(|line| line.text().into_owned()).collect()
}

fn running_call() -> Call {
    Call {
        verb: "Run".to_owned(),
        target: Target::Text("cargo test".to_owned()),
        state: CallState::Running(Duration::from_secs(1)),
        measure: Measure::None,
        took: None,
        detail: Detail::None,
    }
}

#[test]
fn blocks_are_set_apart_by_a_blank_row_except_notice_from_notice() {
    let look = look();
    let mut transcript = Transcript::new();
    transcript.push(Block::Notice("AGENTS.md changed.".to_owned()));
    transcript.push(Block::User("Now the JSON lexer.".to_owned()));
    transcript.push(Block::Notice("Context compacted.".to_owned()));
    transcript.push(Block::Notice("A retry.".to_owned()));
    transcript.push(Block::Error {
        message: "It failed.".to_owned(),
        hint: Some("Press enter.".to_owned()),
    });
    assert_eq!(
        texts(&transcript.commit(99, &look)),
        [
            "· AGENTS.md changed.",
            "",
            "◆ Now the JSON lexer.",
            "",
            "· Context compacted.",
            "· A retry.",
            "",
            "✗ It failed.",
            "  Press enter.",
        ]
    );
    assert!(transcript.is_empty());
}

#[test]
fn a_round_commits_only_when_its_last_call_ends_and_what_follows_waits() {
    let look = look();
    let mut transcript = Transcript::new();
    transcript.push(Block::User("Run the tests.".to_owned()));
    let mut round = Round {
        calls: vec![running_call(), running_call()],
        open: None,
    };
    round.calls[0].state = CallState::Succeeded;
    let id = transcript.push(Block::Round(round));
    // The blank row before the round is committed as it begins; the box is
    // live.
    assert_eq!(
        texts(&transcript.commit(99, &look)),
        ["◆ Run the tests.", ""]
    );
    let live = transcript.live(99, &look, 0);
    // The box, and the blank row under it.
    assert_eq!(live.len(), 5, "{:?}", texts(&live));
    // A notice that arrives meanwhile waits for the round, and is drawn live
    // after it.
    transcript.push(Block::Notice("The sandbox stopped a write.".to_owned()));
    assert!(transcript.commit(99, &look).is_empty());
    let live = texts(&transcript.live(99, &look, 0));
    assert_eq!(live[live.len() - 2], "· The sandbox stopped a write.");
    assert_eq!(live[live.len() - 3], "");
    // The last call ends, reached by its id under the notice: the box and
    // the notice commit, in order.
    let Some(Block::Round(round)) = transcript.get_mut(id) else {
        panic!("the round is still open");
    };
    round.calls[1].state = CallState::Succeeded;
    let lines = texts(&transcript.commit(99, &look));
    assert_eq!(lines.len(), 4 + 2, "{lines:?}");
    assert!(lines[0].starts_with('╭'));
    assert_eq!(lines[4], "");
    assert_eq!(lines[5], "· The sandbox stopped a write.");
    assert!(transcript.live(99, &look, 0).is_empty());
    // Committed whole, it can no longer be reached.
    assert!(transcript.get_mut(id).is_none());
}

#[test]
fn reasoning_that_ends_without_a_summary_commits_and_lets_what_follows() {
    let look = look();
    let mut transcript = Transcript::new();
    let id = transcript.push(Block::Reasoning(Reasoning {
        text: "Thinking".to_owned(),
        elapsed: Duration::from_secs(2),
        summary: None,
        finished: false,
    }));
    transcript.push(Block::Notice("A notice after it.".to_owned()));
    assert!(transcript.commit(99, &look).is_empty());
    if let Some(Block::Reasoning(reasoning)) = transcript.get_mut(id) {
        reasoning.finished = true;
    }
    assert_eq!(
        texts(&transcript.commit(99, &look)),
        ["∴ Reasoned for 2s", "", "· A notice after it."]
    );
}

#[test]
fn an_answer_commits_as_it_streams_and_reasoning_when_it_ends() {
    let look = look();
    let mut transcript = Transcript::new();
    transcript.push(Block::Reasoning(Reasoning {
        text: "Thinking about the loop".to_owned(),
        elapsed: Duration::from_secs(3),
        summary: None,
        finished: false,
    }));
    assert!(transcript.commit(99, &look).is_empty());
    assert_eq!(
        texts(&transcript.live(99, &look, 0)),
        ["⣾ Reasoning  3s", "  Thinking about the loop", ""]
    );
    if let Some(Block::Reasoning(reasoning)) = transcript.last_mut() {
        reasoning.summary = Some("The loop breaks early.".to_owned());
        reasoning.elapsed = Duration::from_secs(6);
        reasoning.finished = true;
    }
    transcript.push(Block::Answer(Markdown::answer()));
    if let Some(Block::Answer(answer)) = transcript.last_mut() {
        answer.push("First paragraph.\n\nSecond ");
    }
    assert_eq!(
        texts(&transcript.commit(99, &look)),
        [
            "∴ Reasoned for 6s",
            "  The loop breaks early.",
            "",
            "◇ First paragraph."
        ]
    );
    // The blank row between paragraphs goes with the next one's first row.
    assert_eq!(texts(&transcript.live(99, &look, 0)), ["", "  Second", ""]);
    if let Some(Block::Answer(answer)) = transcript.last_mut() {
        answer.push("paragraph.");
        answer.finish();
    }
    assert_eq!(
        texts(&transcript.commit(99, &look)),
        ["", "  Second paragraph."]
    );
    assert!(transcript.is_empty());
}

#[test]
fn the_header_is_set_apart_at_once_and_compact_density_sets_apart_only_turns() {
    let mut look = look();
    let mut transcript = Transcript::new();
    transcript.push(Block::Header(Header {
        version: "0.1.0".to_owned(),
        model: "opus-5.5".to_owned(),
        ..Header::default()
    }));
    let lines = transcript.commit(99, &look);
    assert_eq!(lines.last().map(Line::width), Some(0));
    transcript.push(Block::User("Hi.".to_owned()));
    let lines = texts(&transcript.commit(99, &look));
    assert_eq!(lines, ["◆ Hi."]);
    look.density = Density::Compact;
    transcript.push(Block::Notice("One.".to_owned()));
    transcript.push(Block::Error {
        message: "Two.".to_owned(),
        hint: None,
    });
    transcript.push(Block::User("Again.".to_owned()));
    assert_eq!(
        texts(&transcript.commit(99, &look)),
        ["· One.", "✗ Two.", "", "◆ Again."]
    );
}
