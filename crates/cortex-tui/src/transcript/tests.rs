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

/// A session of every kind of block, pushed and finished in turn.
fn session(transcript: &mut Transcript) {
    transcript.push(Block::Header(Header {
        version: "0.1.0".to_owned(),
        model: "opus-5.5".to_owned(),
        folder: "~/src/parser".to_owned(),
        ..Header::default()
    }));
    transcript.push(Block::User("Fix scan_number, and add a test.".to_owned()));
    transcript.push(Block::Reasoning(Reasoning {
        text: String::new(),
        elapsed: Duration::from_secs(6),
        summary: Some("The loop breaks on any non-digit.".to_owned()),
        finished: true,
    }));
    let mut answer = Markdown::answer().keeping();
    answer.push(
        "It stops at the first character that is not a digit, so `1_000` ends after the \
         `1`.\n\n~~~rust\nif c == '_' { continue; }\n~~~\n\n- one\n- two\n",
    );
    answer.finish();
    transcript.push(Block::Answer(answer));
    let mut failed = running_call();
    failed.state = CallState::Failed;
    failed.detail = Detail::Output("line one\nline two\ntest result: FAILED".to_owned());
    transcript.push(Block::Round(Round {
        calls: vec![running_call(), failed],
        open: None,
    }));
    if let Some(Block::Round(round)) = transcript.last_mut() {
        round.calls[0].state = CallState::Succeeded;
    }
    transcript.push(Block::Notice("Context compacted.".to_owned()));
    transcript.push(Block::Notice("A retry.".to_owned()));
    transcript.push(Block::Listing {
        title: "Status".to_owned(),
        lines: vec!["model  opus-5.5".to_owned()],
    });
    transcript.push(Block::Error {
        message: "It failed.".to_owned(),
        hint: Some("Press enter.".to_owned()),
    });
    transcript.push(Block::Summary {
        took: Duration::from_secs(12),
        tokens: Some(1_200),
        cost: Some("$0.02".to_owned()),
    });
}

/// The kept blocks, drawn again at the width they committed at, are the
/// lines the transcript committed, blank rows and all (step 25): the
/// full-screen presenter and the viewer draw the session as the scrollback
/// holds it. At another width they are what a transcript committed at that
/// width gives.
#[test]
fn kept_blocks_draw_again_the_lines_that_were_committed() {
    let look = look();
    for width in [40, 79, 99] {
        let mut transcript = Transcript::new();
        transcript.keep_finished();
        session(&mut transcript);
        let committed = transcript.commit(width, &look);
        assert!(transcript.is_empty());
        let mut kept = kept::Kept::new();
        kept.extend(transcript.take_finished());
        assert_eq!(kept.len(), 10);
        assert_eq!(kept.rounds(), 1);
        assert_eq!(kept.round_block(0), Some(4));
        let drawn: Vec<Line> = kept
            .iter()
            .flat_map(|block| block.rows(Placement::Committed, width, &look))
            .collect();
        assert_eq!(texts(&drawn), texts(&committed), "at {width} columns");
        assert_eq!(drawn, committed, "the styles at {width} columns");
        // Drawn again at another width: what committing there gives.
        let mut other = Transcript::new();
        session(&mut other);
        let narrow: Vec<Line> = kept
            .iter()
            .flat_map(|block| block.rows(Placement::Committed, 30, &look))
            .collect();
        assert_eq!(narrow, other.commit(30, &look));
    }
    // A transcript that does not keep its blocks gives none.
    let mut transcript = Transcript::new();
    session(&mut transcript);
    let _ = transcript.commit(79, &look);
    assert!(transcript.take_finished().is_empty());
}

/// A kept round holds at most the end of a long command's output, from a
/// line's start.
#[test]
fn a_kept_round_holds_the_end_of_a_long_output() {
    let mut call = running_call();
    call.state = CallState::Succeeded;
    let long: String = (0..10_000).map(|n| format!("line {n}\n")).collect();
    call.detail = Detail::Output(long.clone());
    let mut kept = kept::Kept::new();
    kept.extend([kept::Finished {
        block: Block::Round(Round {
            calls: vec![call],
            open: None,
        }),
        gap: false,
        after: false,
    }]);
    let Some(Detail::Output(output)) = kept
        .get(0)
        .and_then(kept::KeptBlock::round)
        .map(|round| round.calls[0].detail.clone())
    else {
        panic!("no output kept");
    };
    assert!(output.len() <= kept::OUTPUT_KEPT, "{}", output.len());
    assert!(output.starts_with("line ") && long.ends_with(&output));
    assert!(kept.bytes() < kept::OUTPUT_KEPT + 1_024);
}

/// What the full-screen presenter draws of a transcript: its kept blocks,
/// the rows committed of the block still open, and the live rows.
fn full_screen_rows(
    transcript: &mut Transcript,
    kept: &mut kept::Kept,
    width: usize,
    look: &Look,
) -> Vec<Line> {
    let _ = transcript.commit(width, look);
    kept.extend(transcript.take_finished());
    let mut rows: Vec<Line> = kept
        .iter()
        .flat_map(|block| block.rows(Placement::Committed, width, look))
        .collect();
    rows.extend(transcript.open_rows().iter().cloned());
    rows.extend(transcript.live(width, look, 0));
    rows
}

/// At every token of an answer that streams, the full-screen presenter's
/// rows are the inline presenter's scrollback and live rows (step 25): the
/// rows an answer commits while it is still open are kept beside it until
/// it finishes.
#[test]
fn every_frame_of_a_stream_is_drawn_the_same_by_both_presenters() {
    let look = look();
    let width = 50;
    let text = "It stops at the first character that is not a digit, so `1_000` ends \
                after the `1`.\n\n~~~rust\nif c == '_' { continue; }\n~~~\n\n- one\n- two \
                and a long item that wraps over the row's end\n\nThe end.\n";
    let mut inline = Transcript::new();
    let mut full = Transcript::new();
    full.keep_finished();
    let mut kept = kept::Kept::new();
    let mut scrollback = Vec::new();
    for transcript in [&mut inline, &mut full] {
        transcript.push(Block::User("Fix it.".to_owned()));
        transcript.push(Block::Answer(Markdown::answer().keeping()));
    }
    let mut pieces: Vec<&str> = text.split_inclusive(' ').collect();
    pieces.push("");
    for (index, piece) in pieces.iter().enumerate() {
        for transcript in [&mut inline, &mut full] {
            if let Some(Block::Answer(answer)) = transcript.last_mut() {
                answer.push(piece);
                if index == pieces.len() - 1 {
                    answer.finish();
                }
            }
        }
        scrollback.extend(inline.commit(width, &look));
        let mut shown = scrollback.clone();
        shown.extend(inline.live(width, &look, 0));
        let drawn = full_screen_rows(&mut full, &mut kept, width, &look);
        assert_eq!(texts(&drawn), texts(&shown), "after {index} pieces");
        assert_eq!(drawn, shown, "the styles after {index} pieces");
    }
    assert!(full.open_rows().is_empty());
    assert_eq!(kept.len(), 2);
}

/// A change of width in the middle of an answer lays its committed rows out
/// again at the new width, as if it had streamed at that width all along.
#[test]
fn an_answer_resized_while_it_streams_is_laid_out_again() {
    let look = look();
    let text = "A paragraph that is long enough to wrap over several rows at forty \
                columns, and then some more words after it.\n\nAnd a second one.\n";
    let half = text.len() / 2;
    let mut full = Transcript::new();
    full.keep_finished();
    full.push(Block::Answer(Markdown::answer().keeping()));
    if let Some(Block::Answer(answer)) = full.last_mut() {
        answer.push(&text[..half]);
    }
    let _ = full.commit(40, &look);
    assert!(!full.open_rows().is_empty());
    full.rewrap_open(70, &look);
    // What a stream at seventy columns all along committed of the same text.
    let mut wide = Transcript::new();
    wide.keep_finished();
    wide.push(Block::Answer(Markdown::answer().keeping()));
    if let Some(Block::Answer(answer)) = wide.last_mut() {
        answer.push(&text[..half]);
    }
    let _ = wide.commit(70, &look);
    assert_eq!(full.open_rows(), wide.open_rows());
    assert_eq!(full.live(70, &look, 0), wide.live(70, &look, 0));
    // The rest streams on at the new width, and the answer is kept whole.
    for transcript in [&mut full, &mut wide] {
        if let Some(Block::Answer(answer)) = transcript.last_mut() {
            answer.push(&text[half..]);
            answer.finish();
        }
        let _ = transcript.commit(70, &look);
    }
    let rows = |transcript: &mut Transcript| {
        let mut kept = kept::Kept::new();
        kept.extend(transcript.take_finished());
        kept.iter()
            .flat_map(|block| block.rows(Placement::Committed, 70, &look))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&mut full), rows(&mut wide));
}
