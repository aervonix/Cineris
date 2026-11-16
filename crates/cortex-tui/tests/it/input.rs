//! The input as a whole: pastes, history from a multi-line prompt, the
//! menus as the filter narrows and widens, and the send modes.

use cortex_term::Event;
use cortex_tui::complete::Empty;
use cortex_tui::history::MemoryHistory;
use cortex_tui::input::{Delivery, Input, Outcome};
use cortex_tui::keys::{Action, KeyMap};

use crate::{HISTORY, input, input_with, press, type_text};

fn paste(input: &mut Input, text: &str) -> Outcome {
    input.handle(&Event::Paste(text.to_owned()))
}

#[test]
fn a_paste_is_text_never_keys() {
    let mut input = input_with("x");
    // Enter, Escape sequences, Ctrl+C, Tab, Backspace: all text.
    let pasted = "a\r\nb\rc\x1b[A\x03\t\x7f\x1b";
    assert_eq!(paste(&mut input, pasted), Outcome::Changed);
    assert_eq!(input.editor().expanded(), "xa\nb\nc\x1b[A\x03\t\x7f\x1b");
    assert!(!input.quit_armed());
    assert!(input.menu().is_none());
}

#[test]
fn a_paste_with_a_box_focused_goes_to_the_input() {
    let mut input = input_with("x");
    input.focus_mut().update(&[7], &[]);
    input.act(Action::FocusNext);
    assert_eq!(
        paste(&mut input, "y"),
        Outcome::Focused(cortex_tui::focus::Target::Input)
    );
    assert!(input.focus().on_input());
    assert_eq!(input.editor().expanded(), "xy");
}

#[test]
fn ctrl_c_with_a_picker_open_closes_it_and_says_so() {
    let mut input = input_with("keep");
    let rows = vec![cortex_tui::complete::Row::new("low", "")];
    input.open_picker(cortex_tui::complete::Menu::picker("effort", rows, 0, None));
    assert_eq!(
        press(&mut input, "ctrl+c"),
        Outcome::PickerClosed("effort".to_owned())
    );
    assert!(input.menu().is_none());
    assert_eq!(input.editor().expanded(), "keep", "the prompt is kept");
}

#[test]
fn a_paste_over_a_limit_is_a_chip_sent_whole_and_one_under_is_text() {
    let mut input = input_with("review ");
    let under = "l\n".repeat(12);
    paste(&mut input, &under);
    assert!(input.editor().chips().is_empty());
    input.act(Action::Undo);
    let over = "l\n".repeat(13);
    paste(&mut input, &over);
    assert_eq!(input.editor().chips().len(), 1);
    assert_eq!(input.editor().chips()[0].label(), "[pasted 13 lines]");
    assert_eq!(
        press(&mut input, "enter"),
        Outcome::Submit {
            text: format!("review {over}"),
            delivery: Delivery::Prompt
        }
    );
}

#[test]
fn a_paste_that_ends_mid_cluster_joins_what_follows() {
    let mut input = input_with("e");
    paste(&mut input, "\u{301}");
    assert_eq!(input.editor().text(), "e\u{301}");
    assert_eq!(input.editor().cursor(), "e\u{301}".len());
    // A paste of half a flag before the other half.
    let mut input = input_with("\u{1F1F7}");
    input.act(Action::TextStart);
    paste(&mut input, "\u{1F1EB}");
    assert_eq!(input.editor().cursor(), input.editor().text().len());
}

#[test]
fn history_steps_only_from_the_first_and_last_rows() {
    let mut input = input_with("one\ntwo\nthree");
    // On the last row: Up moves within the prompt first.
    press(&mut input, "up");
    press(&mut input, "up");
    assert_eq!(input.editor().expanded(), "one\ntwo\nthree");
    press(&mut input, "up");
    assert_eq!(input.editor().expanded(), HISTORY[2], "from the first row");
    press(&mut input, "up");
    assert_eq!(input.editor().expanded(), HISTORY[1]);
    // The cursor is on the first row of a recalled prompt: Up goes on.
    press(&mut input, "up");
    assert_eq!(input.editor().expanded(), HISTORY[0]);
    assert_eq!(press(&mut input, "up"), Outcome::Nothing, "the oldest");
    // Down shows the newer entry with the cursor at its end; from its
    // first row, Down moves within it before going on.
    press(&mut input, "down");
    assert_eq!(input.editor().expanded(), HISTORY[1]);
    press(&mut input, "up");
    assert_eq!(input.editor().expanded(), HISTORY[1], "up a row inside it");
    press(&mut input, "down");
    assert_eq!(
        input.editor().expanded(),
        HISTORY[1],
        "down a row inside it"
    );
    press(&mut input, "down");
    assert_eq!(input.editor().expanded(), HISTORY[2]);
    press(&mut input, "down");
    assert_eq!(
        input.editor().expanded(),
        "one\ntwo\nthree",
        "the draft again"
    );
}

#[test]
fn history_from_a_middle_row_moves_within_the_prompt() {
    let mut input = input_with("one\ntwo\nthree");
    press(&mut input, "up");
    assert_eq!(input.editor().expanded(), "one\ntwo\nthree");
    press(&mut input, "down");
    assert_eq!(input.editor().expanded(), "one\ntwo\nthree");
}

#[test]
fn a_search_with_no_match_leaves_the_prompt_alone() {
    let mut input = input_with("draft");
    press(&mut input, "ctrl+r");
    type_text(&mut input, "nothing like it");
    assert_eq!(input.search_found(), None);
    press(&mut input, "enter");
    assert!(input.search().is_none());
    assert_eq!(input.editor().expanded(), "draft");
}

#[test]
fn a_menu_narrows_and_widens_as_the_filter_changes() {
    let mut input = input();
    let paths: Vec<String> = (0..20).map(|n| format!("src/part{n:02}.rs")).collect();
    let mut all = paths.clone();
    all.push("README.md".to_owned());
    input.set_files(all);
    type_text(&mut input, "@");
    assert_eq!(input.menu().unwrap().rows.len(), 21);
    assert_eq!(
        input.menu().unwrap().visible().len(),
        8,
        "eight rows in view"
    );
    type_text(&mut input, "part1");
    // part10 to part19, and part01: the characters in order.
    assert_eq!(input.menu().unwrap().rows.len(), 11);
    type_text(&mut input, "9");
    assert_eq!(input.menu().unwrap().rows.len(), 1);
    type_text(&mut input, "x");
    let menu = input.menu().unwrap();
    assert!(menu.rows.is_empty());
    assert_eq!(menu.empty, Empty::NoMatch);
    assert_eq!(menu.selected(), None);
    assert_eq!(
        press(&mut input, "tab"),
        Outcome::Nothing,
        "nothing to accept"
    );
    for _ in 0..2 {
        press(&mut input, "backspace");
    }
    assert_eq!(input.menu().unwrap().rows.len(), 11, "widened again");
    for _ in 0..5 {
        press(&mut input, "backspace");
    }
    assert_eq!(input.menu().unwrap().rows.len(), 21);
    press(&mut input, "backspace");
    assert!(input.menu().is_none(), "the @ is gone");
}

#[test]
fn files_not_listed_yet_say_so() {
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    type_text(&mut input, "@lex");
    assert_eq!(input.menu().unwrap().empty, Empty::Listing);
    input.set_files(vec!["src/lex.rs".to_owned()]);
    assert_eq!(input.menu().unwrap().rows.len(), 1);
}

#[test]
fn a_menu_closed_with_escape_stays_closed_until_asked_again_elsewhere() {
    let mut input = input_with("@le");
    press(&mut input, "esc");
    assert!(input.menu().is_none());
    type_text(&mut input, "x");
    assert!(input.menu().is_none(), "the same @");
    type_text(&mut input, " @");
    assert!(input.menu().is_some(), "a new @");
}

#[test]
fn commands_complete_at_the_start_only() {
    let mut input = input_with("/mo");
    let labels: Vec<String> = input
        .menu()
        .unwrap()
        .rows
        .iter()
        .map(|r| r.label.clone())
        .collect();
    assert_eq!(labels, ["mode", "model"]);
    press(&mut input, "down");
    press(&mut input, "enter");
    assert_eq!(input.editor().expanded(), "/model ");
    assert!(input.menu().is_none());
}

#[test]
fn tab_switches_steer_and_queue_only_while_a_turn_runs() {
    let mut input = input_with("more");
    assert_eq!(
        press(&mut input, "tab"),
        Outcome::Ignored(Action::SwitchSendMode)
    );
    input.set_running(true);
    press(&mut input, "tab");
    press(&mut input, "tab");
    assert_eq!(
        press(&mut input, "enter"),
        Outcome::Submit {
            text: "more".to_owned(),
            delivery: Delivery::Steer
        }
    );
}

#[test]
fn a_rebound_key_is_the_only_way_to_its_action() {
    let keys = vec!["ctrl+s".to_owned()];
    let (map, errors) = KeyMap::with_table([("editor.submit", keys.as_slice())]);
    assert!(errors.is_empty());
    let mut input = Input::new(map, Box::new(MemoryHistory::new()));
    type_text(&mut input, "hi");
    assert_eq!(press(&mut input, "enter"), Outcome::Nothing);
    assert!(matches!(
        press(&mut input, "ctrl+s"),
        Outcome::Submit { .. }
    ));
}

#[test]
fn the_input_box_scrolls_to_keep_the_cursor_in_view() {
    let mut input = input_with(&"line\n".repeat(20));
    assert_eq!(input.scroll(), 21 - 8);
    input.act(Action::TextStart);
    assert_eq!(input.scroll(), 0);
    // A narrower box wraps more and keeps the cursor in view.
    input.act(Action::TextEnd);
    input.set_width(2);
    assert!(input.scroll() > 0);
}

/// With the `@` filtering handed out, rows for any query but the latest
/// are dropped: an answer that comes late never replaces a newer one, and
/// one that comes before the newer query's is never shown over it.
#[test]
fn rows_for_an_older_file_query_are_never_shown_over_a_newer_one() {
    use cortex_tui::complete::Row;
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    input.set_width(80);
    input.filter_files_elsewhere();
    let typed = |input: &mut Input, text: &str| {
        for c in text.chars() {
            input.handle(&Event::Key(cortex_term::Key::plain(
                cortex_term::KeyCode::Char(c),
            )));
        }
    };
    typed(&mut input, "@le");
    let older = input.take_file_query().expect("a query");
    assert_eq!(older.query, "le");
    typed(&mut input, "x");
    let newer = input.take_file_query().expect("a newer query");
    assert_eq!(newer.query, "lex");
    assert!(newer.generation > older.generation);
    assert_eq!(input.take_file_query(), None, "handed out once");

    // The older answer, before the newer one: not shown.
    input.file_rows(older.generation, vec![Row::new("src/left.rs", "")]);
    assert!(input.menu().expect("the menu").rows.is_empty());
    // The newer one is.
    input.file_rows(newer.generation, vec![Row::new("src/lex.rs", "")]);
    let labels = |input: &Input| -> Vec<String> {
        input
            .menu()
            .expect("the menu")
            .rows
            .iter()
            .map(|row| row.label.clone())
            .collect()
    };
    assert_eq!(labels(&input), ["src/lex.rs"]);
    // The older one late, after it: still not shown.
    input.file_rows(older.generation, vec![Row::new("src/left.rs", "")]);
    assert_eq!(labels(&input), ["src/lex.rs"]);
}
