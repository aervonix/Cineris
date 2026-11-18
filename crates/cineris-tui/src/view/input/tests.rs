#![allow(
    clippy::disallowed_methods,
    reason = "the tests read the reference screens"
)]

use cineris_term::{ColorDepth, Event, Line};

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
    // Every width in the nightly run; every third on a pull request, which
    // still meets every way a row can end.
    let step = if cineris_testkit::cases::exhaustive() {
        1
    } else {
        3
    };
    for width in (8..40).step_by(step) {
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

/// A chip whose label holds what acts unseen is laid out in the columns
/// its label is drawn in, escaped: the cursor after it is after the text.
#[test]
fn a_chip_is_laid_out_as_wide_as_its_label_is_drawn() {
    let look = look();
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    input.set_width(text_width(60));
    let mut fragment = Fragment::text("a ");
    fragment.push(Fragment::chip(Chip::new("x\ny", "[two\u{200b}\u{202e}]")));
    fragment.push(Fragment::text(" end"));
    input.handle(&Event::Paste(String::new()));
    input.external_result(Some(fragment));
    let drawn = input_box(&input, &look, 60);
    let row = drawn.lines[1].text().into_owned();
    assert!(row.contains("a [two\\u{200b}\\u{202e}] end"), "{row:?}");
    let end = row.find(" end").expect("the text") + " end".len();
    assert_eq!(
        drawn.cursor.col,
        cineris_base::text::width(&row[..end]),
        "{row:?}"
    );
}

/// What acts unseen is drawn as its escape, and the editor lays it out in
/// the escape's columns: the cursor stays after the text, and no row is cut
/// (the review of step 23 found a zero-width space counted as no columns,
/// the cursor eight columns inside the text, and the row's end cut off).
#[test]
fn what_acts_unseen_takes_the_columns_of_its_escape() {
    let look = look();
    let text = "\u{fe0f}abc\u{200b}def \u{202e}x a\u{e0041}b \u{ad}z \u{1b}w end\u{26a0}\u{fe0f}";
    let expected: String = cineris_term::sanitize(text)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(expected.contains("abc\\u{200b}def"), "{expected}");
    for width in 22..70 {
        let input = holding(text, width);
        let drawn = input_box(&input, &look, width);
        let found = testing::problems(&drawn.lines, width);
        assert!(found.is_empty(), "width {width}: {found:?}");
        let rows: Vec<String> = drawn.lines[1..drawn.lines.len() - 1]
            .iter()
            .map(|line| {
                let row = line.text().into_owned();
                let inside: String = row
                    .chars()
                    .skip(RoundedBox::TEXT_COLUMN + MARK_WIDTH)
                    .collect();
                inside
                    .trim_end()
                    .trim_end_matches('│')
                    .trim_end()
                    .to_owned()
            })
            .collect();
        let shown: String = rows
            .concat()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(shown, expected, "width {width}: {rows:#?}");
        let last = &rows[drawn.cursor.row - 1];
        assert_eq!(
            drawn.cursor.col,
            RoundedBox::TEXT_COLUMN + MARK_WIDTH + cineris_base::text::width(last),
            "width {width}: {rows:#?}"
        );
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
            drawn.cursor.col < width.saturating_sub(1).max(1),
            "width {width}: the cursor at {} is on or past the border",
            drawn.cursor.col
        );
        assert!(
            testing::problems(&drawn.lines, width).is_empty(),
            "width {width}: {:?}",
            testing::problems(&drawn.lines, width)
        );
    }
}
