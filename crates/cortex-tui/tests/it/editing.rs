//! The editor's rules: every movement and deletion keeps whole grapheme
//! clusters, at a wrap point too; undo groups typing by word and keeps a
//! paste, a yank and a completion apart; the kill slot; chips.

use cortex_tui::editor::{Chip, Editor, Fragment};

/// One of each kind of cluster that a careless editor splits: a letter
/// with a combining mark, a wide character, a family (ZWJ), a flag, a
/// keycap, a skin tone, Hangul written as jamo, and a plain letter.
const CLUSTERS: [&str; 8] = [
    "e\u{301}",
    "\u{4E2D}",
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
    "\u{1F1EB}\u{1F1F7}",
    "1\u{FE0F}\u{20E3}",
    "\u{1F44D}\u{1F3FD}",
    "\u{1100}\u{1161}\u{11A8}",
    "x",
];

fn editor(text: &str) -> Editor {
    let mut editor = Editor::new();
    editor.insert(Fragment::text(text));
    editor
}

fn boundaries() -> Vec<usize> {
    let mut at = vec![0];
    for cluster in CLUSTERS {
        at.push(at.last().unwrap() + cluster.len());
    }
    at
}

#[test]
fn left_and_right_step_one_whole_cluster() {
    let text = CLUSTERS.concat();
    let stops = boundaries();
    let mut editor = editor(&text);
    for expected in stops.iter().rev().skip(1) {
        assert!(editor.left());
        assert_eq!(editor.cursor(), *expected);
    }
    assert!(!editor.left());
    for expected in stops.iter().skip(1) {
        assert!(editor.right());
        assert_eq!(editor.cursor(), *expected);
    }
}

#[test]
fn backspace_and_delete_take_one_whole_cluster() {
    let text = CLUSTERS.concat();
    for (index, cluster) in CLUSTERS.iter().enumerate() {
        let stops = boundaries();
        // Backspace with the cursor after it.
        let mut back = editor(&text);
        back.set_cursor(stops[index + 1]);
        assert!(back.backspace());
        let mut expected = CLUSTERS.to_vec();
        expected.remove(index);
        assert_eq!(back.text(), expected.concat(), "backspace over {cluster:?}");
        assert_eq!(back.cursor(), stops[index]);
        // Delete with the cursor before it.
        let mut forward = editor(&text);
        forward.set_cursor(stops[index]);
        assert!(forward.delete());
        assert_eq!(forward.text(), expected.concat(), "delete over {cluster:?}");
    }
}

#[test]
fn a_cursor_set_inside_a_cluster_goes_to_its_end() {
    let text = CLUSTERS.concat();
    let stops = boundaries();
    let mut editor = editor(&text);
    editor.set_cursor(stops[2] + 4);
    assert_eq!(editor.cursor(), stops[3]);
}

#[test]
fn words_lines_and_ends_move_by_clusters() {
    let mut editor = editor("na\u{EF}ve \u{1F44D}\u{1F3FD} caf\u{E9}\nsecond line");
    editor.line_start();
    assert_eq!(
        editor.cursor(),
        "na\u{EF}ve \u{1F44D}\u{1F3FD} caf\u{E9}\n".len()
    );
    editor.word_left();
    assert_eq!(editor.cursor(), "na\u{EF}ve \u{1F44D}\u{1F3FD} ".len());
    editor.word_left();
    assert_eq!(editor.cursor(), 0, "the emoji is not a word");
    editor.word_right();
    assert_eq!(editor.cursor(), "na\u{EF}ve".len());
    editor.text_end();
    assert_eq!(editor.cursor(), editor.text().len());
    editor.text_start();
    editor.line_end();
    assert_eq!(
        editor.cursor(),
        "na\u{EF}ve \u{1F44D}\u{1F3FD} caf\u{E9}".len()
    );
}

#[test]
fn deleting_a_word_or_a_line_keeps_clusters_whole() {
    let mut editor = editor("say \u{1F1EB}\u{1F1F7}e\u{301}");
    assert!(editor.kill_word_back());
    assert_eq!(
        editor.text(),
        "say \u{1F1EB}\u{1F1F7}",
        "the accent goes with its letter"
    );
    editor.text_start();
    assert!(editor.kill_word_forward());
    assert_eq!(editor.text(), " \u{1F1EB}\u{1F1F7}");
    assert!(editor.kill_to_line_end());
    assert_eq!(editor.text(), "");
}

#[test]
fn at_a_wrap_point_every_cluster_stays_whole_and_up_and_down_land_on_boundaries() {
    // Each cluster is two columns wide except the letters: at width 3 rows
    // wrap between clusters, never inside one.
    let text = CLUSTERS.concat();
    let editor = editor(&text);
    for width in 1..8 {
        let layout = editor.layout(width);
        let stops = boundaries();
        for row in &layout.rows {
            assert!(stops.contains(&row.range.start), "width {width}: {row:?}");
            assert!(stops.contains(&row.range.end), "width {width}: {row:?}");
        }
        let mut moving = self::editor(&text);
        moving.text_start();
        while moving.down(width) {
            assert!(stops.contains(&moving.cursor()), "width {width}");
        }
        while moving.up(width) {
            assert!(stops.contains(&moving.cursor()), "width {width}");
        }
    }
}

#[test]
fn a_cursor_never_reaches_past_the_last_column_a_row_may_use() {
    for width in 1..12 {
        for length in 0..30 {
            let text = "ab \u{4E2D}".repeat(length);
            let editor = editor(&text);
            let layout = editor.layout(width);
            // A wide character has a row to itself even when the row is
            // one column wide.
            assert!(layout.cursor_col <= width.max(2), "width {width}, {text:?}");
            for row in &layout.rows {
                let row_text = &text[row.range.clone()];
                let columns: usize = cortex_base::text::width(row_text);
                assert!(columns <= width.max(2), "width {width}: {row_text:?}");
            }
        }
    }
}

#[test]
fn no_row_is_wider_than_its_width_whatever_follows_white_space() {
    let pieces = [
        " ",
        "a",
        "ab",
        "\u{4F60}\u{597D}",
        "\u{4F60}",
        "word",
        "\t",
        "e\u{301}",
    ];
    for width in 1..12 {
        for a in pieces {
            for b in pieces {
                for c in pieces {
                    for d in pieces {
                        let text = format!("{a}{b}{c}{d}{a}{c}");
                        let editor = editor(&text);
                        let layout = editor.layout(width);
                        for row in &layout.rows {
                            let row_text = &text[row.range.clone()];
                            let columns: usize = cortex_base::text::graphemes(row_text)
                                .map(|(_, cluster, w)| if cluster == "\t" { 4 } else { w })
                                .sum();
                            let alone = cortex_base::text::graphemes(row_text).count() == 1;
                            assert!(
                                columns <= width || alone,
                                "width {width}: {row_text:?} is {columns} columns"
                            );
                        }
                    }
                }
            }
        }
    }
    // A chip after white space, as the review found.
    let mut editor = Editor::new();
    editor.type_str("look at this:");
    editor.insert(Fragment::chip(Chip::new(
        "x\n".repeat(148),
        "[pasted 148 lines]",
    )));
    let layout = editor.layout(20);
    assert!(layout.cursor_col <= 20, "{layout:?}");
    assert!(layout.rows.len() >= 2);
}

#[test]
fn typing_is_undone_a_word_at_a_time() {
    let mut editor = Editor::new();
    for c in "fix the bug".chars() {
        editor.type_char(c);
    }
    assert!(editor.undo());
    assert_eq!(editor.text(), "fix the ");
    assert!(editor.undo());
    assert_eq!(editor.text(), "fix ");
    assert!(editor.undo());
    assert_eq!(editor.text(), "");
    assert!(!editor.undo());
    assert!(editor.redo());
    assert!(editor.redo());
    assert_eq!(editor.text(), "fix the ");
}

#[test]
fn a_paste_a_yank_and_a_completion_are_each_a_step_of_their_own() {
    let mut editor = Editor::new();
    for c in "ab".chars() {
        editor.type_char(c);
    }
    editor.insert(Fragment::text(" PASTE "));
    for c in "cd".chars() {
        editor.type_char(c);
    }
    editor.kill_word_back();
    editor.yank();
    editor.yank();
    let end = editor.text().len();
    editor.replace(end - 2..end, Fragment::text("@src/lex.rs "));
    assert_eq!(editor.text(), "ab PASTE cd@src/lex.rs ");
    let steps = [
        "ab PASTE cdcd",
        "ab PASTE cd",
        "ab PASTE ",
        "ab PASTE cd",
        "ab PASTE ",
        "ab",
        "",
    ];
    // The completion, the second yank, the first, the kill, the typing,
    // the paste, the first typing.
    for expected in steps {
        assert!(editor.undo());
        assert_eq!(editor.text(), expected);
    }
}

#[test]
fn deletes_one_after_another_are_undone_together() {
    let mut editor2 = Editor::new();
    for c in "one two".chars() {
        editor2.type_char(c);
    }
    for _ in 0..7 {
        editor2.backspace();
    }
    assert_eq!(editor2.text(), "");
    assert!(editor2.undo());
    assert_eq!(editor2.text(), "one ");
    assert!(editor2.undo());
    assert_eq!(editor2.text(), "one two");
}

#[test]
fn kills_one_after_another_join_in_the_slot() {
    let mut editor = editor("alpha beta gamma");
    editor.kill_word_back();
    editor.kill_word_back();
    assert_eq!(editor.slot().unwrap().expanded(), "beta gamma");
    editor.yank();
    assert_eq!(editor.text(), "alpha beta gamma");
    // A move in between starts the slot again.
    editor.kill_word_back();
    editor.left();
    editor.kill_word_back();
    assert_eq!(editor.slot().unwrap().expanded(), "beta");
}

#[test]
fn kills_both_ways_join_in_the_slot_in_the_texts_order() {
    let mut editor = editor("abcdef");
    editor.set_cursor(3);
    editor.kill_to_line_start();
    editor.kill_to_line_end();
    assert_eq!(editor.text(), "");
    assert_eq!(editor.slot().unwrap().expanded(), "abcdef");
}

#[test]
fn a_place_inside_a_character_is_never_a_panic() {
    let mut editor = editor("\u{E9}x\u{1F1EB}\u{1F1F7}");
    for at in 0..12 {
        editor.set_cursor(at);
        assert!(editor.text().is_char_boundary(editor.cursor()));
        assert_eq!(editor.chip_at(at), None);
        let mut copy = self::editor(editor.text());
        copy.replace(at..at + 1, Fragment::text("y"));
    }
}

#[test]
fn a_chip_is_one_cluster_sends_its_text_and_expands_in_place_undoably() {
    let mut editor = Editor::new();
    editor.type_str("see ");
    editor.insert(Fragment::chip(Chip::new("A\nB\nC", "[pasted 3 lines]")));
    editor.type_str("\u{301}!");
    assert_eq!(editor.expanded(), "see A\nB\nC\u{301}!");
    editor.left();
    editor.left();
    assert!(editor.left(), "the accent is its own cluster after a chip");
    assert_eq!(editor.cursor(), 4);
    assert!(editor.expand_chip(), "the chip after the cursor");
    assert_eq!(editor.text(), "see A\nB\nC\u{301}!");
    assert!(editor.chips().is_empty());
    // Edited, then undone back to the chip.
    editor.type_str("Z");
    editor.undo();
    editor.undo();
    assert_eq!(editor.chips().len(), 1);
    assert_eq!(editor.expanded(), "see A\nB\nC\u{301}!");
    // Backspace takes the whole chip.
    editor.text_end();
    editor.left();
    editor.left();
    assert!(editor.backspace());
    assert_eq!(editor.expanded(), "see \u{301}!");
}

#[test]
fn an_edit_that_joins_two_clusters_leaves_the_cursor_outside_them() {
    // Two regional indicators typed apart join into one flag.
    let mut editor = Editor::new();
    editor.type_str("\u{1F1F7}");
    editor.text_start();
    editor.type_str("\u{1F1EB}");
    assert_eq!(
        editor.cursor(),
        editor.text().len(),
        "after the flag, not inside"
    );
}

#[test]
fn line_breaks_from_anywhere_are_newlines() {
    let mut editor = Editor::new();
    editor.type_str("a\r\nb\rc");
    assert_eq!(editor.text(), "a\nb\nc");
}
