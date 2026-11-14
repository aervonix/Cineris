#![allow(
    clippy::disallowed_methods,
    reason = "the tests read the reference screens"
)]

use cortex_term::{ColorDepth, Event, Line};

use super::*;
use crate::complete::Command;
use crate::editor::{Chip, Fragment};
use crate::glyphs::Glyphs;
use crate::history::MemoryHistory;
use crate::keys::{Action, KeyMap};
use crate::testing::{self, DARK};
use crate::theme::Theme;

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

/// An input holding `text` (with a chip where `\u{FFFC}` is), its cursor
/// at the end, wrapping for a box `width` wide.
fn holding(text: &str, width: usize) -> Input {
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    input.set_width(text_width(width));
    let mut fragment = Fragment::default();
    for (index, piece) in text.split('\u{FFFC}').enumerate() {
        if index > 0 {
            fragment.push(Fragment::chip(Chip::new(
                "x\n".repeat(148),
                "[pasted 148 lines]",
            )));
        }
        fragment.push(Fragment::text(piece));
    }
    input.handle(&Event::Paste(String::new()));
    input.external_result(Some(fragment));
    input
}

/// The states the snapshot shows.
fn states() -> Vec<(&'static str, String)> {
    vec![
        ("empty", String::new()),
        ("one line", "also run clippy when that's done".to_owned()),
        (
            "wrapped, with wide characters and a chip",
            "fix the 漢字 lexer and look at \u{FFFC} then tell me what 1_000 lexes as \
             in every case"
                .to_owned(),
        ),
        ("lines", "first\nsecond\n\tindented\n".to_owned()),
        ("controls", "ansi \u{1b}[31m red \u{7} bell".to_owned()),
        (
            "scrolled",
            (1..=12)
                .map(|n| format!("line {n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    ]
}

#[test]
fn the_input_box_in_every_variant() {
    let mut parts = Vec::new();
    for variant in testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        for (name, text) in states() {
            let input = holding(&text, variant.width());
            let drawn = input_box(&input, &look, variant.width());
            let problems = testing::problems(&drawn.lines, variant.width());
            assert!(problems.is_empty(), "{name}: {problems:?}");
            let mut lines = drawn.lines.clone();
            lines.push(Line::plain(format!(
                "cursor at row {}, column {}",
                drawn.cursor.row, drawn.cursor.col
            )));
            parts.push((format!("{} — {name}", variant.title()), lines));
        }
    }
    testing::assert_snapshot("input", &testing::sections(&parts));
}

fn input_box(input: &Input, look: &Look, width: usize) -> InputBox {
    super::input(input, look, width)
}

#[test]
fn the_reference_screens_input_boxes_are_drawn_cell_for_cell() {
    let look = look();
    for (screen, text, first) in [
        ("0", "", 7),
        ("1", "also run clippy when that's done", 36),
        ("3a", "/model", 2),
        ("3b", "/effort", 2),
    ] {
        let reference = std::fs::read_to_string(format!(
            "{}/../../docs/architecture/tui-look/{screen}.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace("\r\n", "\n");
        let expected: Vec<&str> = reference.lines().skip(first).take(3).collect();
        let input = holding(text, 99);
        let drawn = input_box(&input, &look, 99);
        let got: Vec<String> = drawn.lines.iter().map(|l| l.text().into_owned()).collect();
        assert_eq!(got, expected, "screen {screen}");
    }
}

#[test]
fn the_cursor_is_after_the_text_and_never_on_the_edge() {
    let look = look();
    for width in 8..40 {
        for length in 0..60 {
            let text = "ab 中".repeat(length / 4) + &"x".repeat(length % 4);
            let input = holding(&text, width);
            let drawn = input_box(&input, &look, width);
            assert!(testing::problems(&drawn.lines, width).is_empty());
            assert!(
                drawn.cursor.col < width.saturating_sub(1).max(RoundedBox::TEXT_COLUMN + 2),
                "width {width}, {text:?}: column {}",
                drawn.cursor.col
            );
            assert!(drawn.cursor.row >= 1 && drawn.cursor.row < drawn.lines.len() - 1);
        }
    }
}

#[test]
fn the_box_grows_to_eight_rows_and_scrolls_with_the_cursor() {
    let look = look();
    let text = (1..=20)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut input = holding(&text, 60);
    let drawn = input_box(&input, &look, 60);
    assert_eq!(drawn.lines.len(), MAX_ROWS + 2);
    assert!(drawn.lines[MAX_ROWS].text().contains("line 20"));
    input.act(Action::TextStart);
    let drawn = input_box(&input, &look, 60);
    assert!(drawn.lines[1].text().contains("◆ line 1"));
    assert_eq!(drawn.cursor.row, 1);
}

#[test]
fn a_box_of_any_width_keeps_to_it() {
    let look = look();
    let mut input = holding(
        "the 漢字 \u{FFFC} and e\u{301} 👨\u{200d}👩\u{200d}👧 🇫🇷",
        80,
    );
    input.set_commands(vec![Command {
        name: "model".to_owned(),
        description: "pick".to_owned(),
    }]);
    for width in 1..=120 {
        input.set_width(text_width(width));
        let drawn = input_box(&input, &look, width);
        assert!(
            testing::problems(&drawn.lines, width).is_empty(),
            "width {width}: {:?}",
            testing::problems(&drawn.lines, width)
        );
    }
}
