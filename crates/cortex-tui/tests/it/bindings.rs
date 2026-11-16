//! Every default binding, from key to action to state: each key of each
//! action in `ACTIONS` is pressed in a state where its action shows, and
//! the state after it is checked.

use cortex_term::Event;
use cortex_tui::complete::{Menu, Row, Toggle};
use cortex_tui::focus::Target;
use cortex_tui::editor::Fragment;
use cortex_tui::input::{Delivery, Input, Outcome, SendMode};
use cortex_tui::keys::{ACTIONS, Action, parse};

use crate::{HISTORY, input, input_with, press, type_text};

/// The picker `/effort`, with its fast-mode toggle when `toggle`.
fn effort(toggle: bool) -> Menu {
    let rows = ["low", "medium", "high", "max"].map(|label| Row::new(label, ""));
    let toggle = toggle.then(|| Toggle {
        label: "fast mode".to_owned(),
        on: false,
        note: String::new(),
    });
    Menu::picker("effort", rows.to_vec(), 2, toggle)
}

fn text(input: &Input) -> String {
    input.editor().expanded()
}

fn cursor(input: &Input) -> usize {
    input.editor().cursor()
}

/// Presses `key`, which is bound to `action` by default, where `action`
/// shows, and checks what it did.
#[allow(
    clippy::too_many_lines,
    reason = "one arm an action: the whole table of what each key does, in one place"
)]
fn check(action: Action, key: &str) {
    let context = format!("{key} as {action}");
    match action {
        Action::Interrupt => {
            let mut input = input_with("draft");
            input.set_running(true);
            assert_eq!(press(&mut input, key), Outcome::Interrupt, "{context}");
            input.set_running(false);
            press(&mut input, key);
            assert_eq!(text(&input), "", "{context}: idle, it clears");
            assert_eq!(press(&mut input, key), Outcome::QuitArmed, "{context}");
            assert_eq!(press(&mut input, key), Outcome::Quit, "{context}");
        }
        Action::Redraw => assert_eq!(press(&mut input(), key), Outcome::Redraw, "{context}"),
        Action::OpenViewer => {
            assert_eq!(press(&mut input(), key), Outcome::OpenViewer, "{context}");
        }
        Action::ToggleRound => {
            assert_eq!(press(&mut input(), key), Outcome::ToggleRound, "{context}");
        }
        Action::FocusNext => {
            let mut input = input();
            input.focus_mut().update(&[7], &[3]);
            assert_eq!(press(&mut input, key), Outcome::Focused(Target::ToolBox(7)));
            assert_eq!(press(&mut input, key), Outcome::Focused(Target::Panel(3)));
            assert_eq!(press(&mut input, key), Outcome::Focused(Target::Input));
        }
        Action::Submit => {
            let mut input = input_with("hello");
            assert_eq!(
                press(&mut input, key),
                Outcome::Submit {
                    text: "hello".to_owned(),
                    delivery: Delivery::Prompt
                },
                "{context}"
            );
            assert_eq!(text(&input), "", "{context}");
        }
        Action::NewLine => {
            let mut input = input_with("ab");
            press(&mut input, key);
            assert_eq!(text(&input), "ab\n", "{context}");
        }
        Action::Stop => {
            let mut input = input_with("x");
            assert_eq!(press(&mut input, key), Outcome::Nothing, "{context}: idle");
            input.set_running(true);
            assert_eq!(press(&mut input, key), Outcome::Interrupt, "{context}");
        }
        Action::Quit => {
            assert_eq!(
                press(&mut input_with("x"), key),
                Outcome::Nothing,
                "{context}"
            );
            assert_eq!(press(&mut input(), key), Outcome::Quit, "{context}");
        }
        Action::Left => {
            let mut input = input_with("ab");
            press(&mut input, key);
            assert_eq!(cursor(&input), 1, "{context}");
        }
        Action::Right => {
            let mut input = input_with("ab");
            input.act(Action::LineStart);
            press(&mut input, key);
            assert_eq!(cursor(&input), 1, "{context}");
        }
        Action::WordLeft => {
            let mut input = input_with("fix the");
            press(&mut input, key);
            assert_eq!(cursor(&input), 4, "{context}");
        }
        Action::WordRight => {
            let mut input = input_with("fix the");
            input.act(Action::LineStart);
            press(&mut input, key);
            assert_eq!(cursor(&input), 3, "{context}");
        }
        Action::LineStart => {
            let mut input = input_with("a\nbc");
            press(&mut input, key);
            assert_eq!(cursor(&input), 2, "{context}");
        }
        Action::LineEnd => {
            let mut input = input_with("a\nbc");
            input.act(Action::LineStart);
            press(&mut input, key);
            assert_eq!(cursor(&input), 4, "{context}");
        }
        Action::TextStart => {
            let mut input = input_with("a\nbc");
            press(&mut input, key);
            assert_eq!(cursor(&input), 0, "{context}");
        }
        Action::TextEnd => {
            let mut input = input_with("a\nbc");
            input.act(Action::TextStart);
            press(&mut input, key);
            assert_eq!(cursor(&input), 4, "{context}");
        }
        Action::Up => {
            let mut input = input_with("a\nbc");
            press(&mut input, key);
            assert_eq!(cursor(&input), 1, "{context}: up a row");
            press(&mut input, key);
            assert_eq!(text(&input), HISTORY[2], "{context}: then history");
        }
        Action::Down => {
            let mut input = input_with("ab\ncd");
            input.act(Action::TextStart);
            press(&mut input, key);
            assert_eq!(cursor(&input), 3, "{context}: down a row");
        }
        Action::Backspace => {
            let mut input = input_with("ab");
            press(&mut input, key);
            assert_eq!(text(&input), "a", "{context}");
        }
        Action::Delete => {
            let mut input = input_with("ab");
            input.act(Action::LineStart);
            press(&mut input, key);
            assert_eq!(text(&input), "b", "{context}");
        }
        Action::KillWordBack => {
            let mut input = input_with("fix the");
            press(&mut input, key);
            assert_eq!(text(&input), "fix ", "{context}");
            assert_eq!(
                input.editor().slot().unwrap().expanded(),
                "the",
                "{context}"
            );
        }
        Action::KillWordForward => {
            let mut input = input_with("fix the");
            input.act(Action::LineStart);
            press(&mut input, key);
            assert_eq!(text(&input), " the", "{context}");
        }
        Action::KillToLineEnd => {
            let mut input = input_with("ab\ncd");
            input.act(Action::TextStart);
            press(&mut input, key);
            assert_eq!(text(&input), "\ncd", "{context}");
        }
        Action::KillToLineStart => {
            let mut input = input_with("ab\ncd");
            press(&mut input, key);
            assert_eq!(text(&input), "ab\n", "{context}");
        }
        Action::Yank => {
            let mut input = input_with("ab");
            input.act(Action::KillToLineStart);
            press(&mut input, key);
            assert_eq!(text(&input), "ab", "{context}");
        }
        Action::Undo => {
            let mut input = input_with("one two");
            press(&mut input, key);
            assert_eq!(text(&input), "one ", "{context}");
        }
        Action::Redo => {
            let mut input = input_with("one two");
            input.act(Action::Undo);
            press(&mut input, key);
            assert_eq!(text(&input), "one two", "{context}");
        }
        Action::ExpandChip => {
            let mut input = input();
            let long = "line\n".repeat(13);
            input.handle(&Event::Paste(long.clone()));
            assert_eq!(input.editor().chips().len(), 1, "{context}");
            press(&mut input, key);
            assert!(input.editor().chips().is_empty(), "{context}");
            assert_eq!(input.editor().text(), long, "{context}");
        }
        Action::HistorySearch => {
            let mut input = input();
            press(&mut input, key);
            assert!(input.search().is_some(), "{context}");
        }
        Action::ExternalEditor => {
            assert_eq!(
                press(&mut input(), key),
                Outcome::ExternalEditor,
                "{context}"
            );
        }
        Action::SwitchSendMode => {
            let mut input = input_with("also");
            input.set_running(true);
            press(&mut input, key);
            assert_eq!(input.send_mode(), SendMode::Queue, "{context}");
            assert_eq!(
                press(&mut input, "enter"),
                Outcome::Submit {
                    text: "also".to_owned(),
                    delivery: Delivery::Queue
                },
                "{context}"
            );
        }
        Action::MenuUp | Action::MenuDown => {
            let mut input = input_with("@");
            let rows = input.menu().unwrap().rows.len();
            press(&mut input, key);
            let expected = if action == Action::MenuDown {
                1
            } else {
                rows - 1
            };
            assert_eq!(
                input.menu().unwrap().selected(),
                Some(expected),
                "{context}"
            );
        }
        Action::MenuAccept => {
            let mut input = input_with("read @mai");
            press(&mut input, key);
            assert_eq!(text(&input), "read @src/main.rs ", "{context}");
            assert!(input.menu().is_none(), "{context}");
            // In a picker without a toggle, the same key accepts too.
            let mut input = crate::input();
            input.open_picker(effort(false));
            assert_eq!(
                press(&mut input, key),
                Outcome::Picked("effort".to_owned(), 2),
                "{context}"
            );
        }
        Action::MenuClose => {
            let mut input = input_with("@");
            press(&mut input, key);
            assert!(input.menu().is_none(), "{context}");
            assert_eq!(text(&input), "@", "{context}: the text stays");
        }
        Action::PickerErase => {
            let mut input = input();
            input.open_picker(effort(false));
            type_text(&mut input, "ma");
            assert_eq!(input.menu().unwrap().rows.len(), 1, "{context}");
            press(&mut input, key);
            assert_eq!(input.menu().unwrap().filter, "m", "{context}");
        }
        Action::PickerRemove => {
            let mut input = input();
            input.open_picker(effort(false));
            press(&mut input, "down");
            assert_eq!(
                press(&mut input, key),
                Outcome::Removed("effort".to_owned(), 3),
                "{context}"
            );
            assert!(input.menu().is_some(), "{context}: the picker stays open");
        }
        Action::ToggleFast => {
            let mut input = input();
            input.open_picker(effort(true));
            assert_eq!(
                press(&mut input, key),
                Outcome::Toggled("effort".to_owned(), true),
                "{context}"
            );
            assert!(input.menu().is_some(), "{context}: the picker stays open");
            assert_eq!(
                press(&mut input, "enter"),
                Outcome::Picked("effort".to_owned(), 2),
                "{context}: Enter accepts"
            );
        }
        Action::SearchOlder => {
            let mut input = input();
            input.act(Action::HistorySearch);
            press(&mut input, key);
            assert_eq!(input.search_found().map(Fragment::as_str), Some(HISTORY[2]), "{context}");
            press(&mut input, key);
            assert_eq!(input.search_found().map(Fragment::as_str), Some(HISTORY[1]), "{context}");
        }
        Action::SearchNewer => {
            let mut input = input();
            input.act(Action::HistorySearch);
            input.act(Action::SearchOlder);
            input.act(Action::SearchOlder);
            press(&mut input, key);
            assert_eq!(input.search_found().map(Fragment::as_str), Some(HISTORY[2]), "{context}");
        }
        Action::SearchAccept => {
            let mut input = input();
            input.act(Action::HistorySearch);
            type_text(&mut input, "clip");
            press(&mut input, key);
            assert!(input.search().is_none(), "{context}");
            assert_eq!(text(&input), HISTORY[1], "{context}");
        }
        Action::SearchCancel => {
            let mut input = input_with("draft");
            input.act(Action::HistorySearch);
            type_text(&mut input, "clip");
            press(&mut input, key);
            assert!(input.search().is_none(), "{context}");
            assert_eq!(text(&input), "draft", "{context}");
        }
        Action::SearchErase => {
            let mut input = input();
            input.act(Action::HistorySearch);
            type_text(&mut input, "lexz");
            assert_eq!(input.search_found(), None, "{context}");
            press(&mut input, key);
            assert_eq!(input.search_found().map(Fragment::as_str), Some(HISTORY[0]), "{context}");
        }
        Action::FocusActivate | Action::FocusReturn | Action::FocusUp | Action::FocusDown => {
            let mut input = input();
            input.focus_mut().update(&[7], &[]);
            input.act(Action::FocusNext);
            let expected = match action {
                Action::FocusActivate => Outcome::Activate(Target::ToolBox(7)),
                Action::FocusReturn => Outcome::Focused(Target::Input),
                Action::FocusUp => Outcome::Move(Target::ToolBox(7), false),
                _ => Outcome::Move(Target::ToolBox(7), true),
            };
            assert_eq!(press(&mut input, key), expected, "{context}");
        }
        // A panel's own keys (step 24): a focused panel is given each, for
        // the application to take or pass on (its tests check what each
        // does to the plan and the review: `app::tests::panels`); a focused
        // box has no use for them, and one that types goes to the editor,
        // the focus with it.
        action if action.is_panel_key() => {
            let mut panel = input();
            panel.focus_mut().update(&[], &[3]);
            panel.act(Action::FocusNext);
            assert_eq!(
                press(&mut panel, key),
                Outcome::Panel(Target::Panel(3), action),
                "{context}"
            );
            assert_eq!(text(&panel), "", "{context}: nothing typed");
            let mut boxed = input();
            boxed.focus_mut().update(&[7], &[]);
            boxed.act(Action::FocusNext);
            match cortex_tui::keys::typed(parse(key).unwrap()) {
                Some(c) => {
                    assert_eq!(
                        press(&mut boxed, key),
                        Outcome::Focused(Target::Input),
                        "{context}"
                    );
                    assert_eq!(text(&boxed), c.to_string(), "{context}: typed");
                }
                None => {
                    assert_eq!(press(&mut boxed, key), Outcome::Nothing, "{context}");
                    assert_eq!(boxed.focus().focused(), Target::ToolBox(7), "{context}");
                }
            }
        }
        // A dialog's keys are looked up in its own context, and the
        // application's tests check what each answers (`app::dialog`).
        Action::AllowOnce
        | Action::AllowSession
        | Action::AllowProject
        | Action::Deny
        | Action::DenyWithNote
        | Action::Never
        | Action::ShowWhole => {
            let map = cortex_tui::keys::KeyMap::defaults();
            assert_eq!(
                map.lookup(&[cortex_tui::keys::Context::Dialog], parse(key).unwrap()),
                Some(action),
                "{context}"
            );
            assert_eq!(
                input().act(action),
                Outcome::Ignored(action),
                "{context}: the input never answers a dialog"
            );
        }
        other => panic!("{other} has a default key and no check here"),
    }
}

#[test]
fn every_default_binding_does_its_action() {
    let mut checked = 0;
    for entry in ACTIONS {
        for key in entry.keys {
            check(entry.action, key);
            checked += 1;
        }
    }
    // Every action but `suspend` has a key.
    assert!(checked >= ACTIONS.len() - 1);
}

#[test]
fn every_action_but_suspend_has_a_default_key() {
    for entry in ACTIONS {
        assert_eq!(
            entry.keys.is_empty(),
            entry.action == Action::Suspend,
            "{}",
            entry.action
        );
    }
}

#[test]
fn the_three_new_line_keys_and_the_redo_keys_are_bound() {
    let map = cortex_tui::keys::KeyMap::defaults();
    for key in ["shift+enter", "alt+enter", "ctrl+j"] {
        assert_eq!(
            map.lookup(&[cortex_tui::keys::Context::Editor], parse(key).unwrap()),
            Some(Action::NewLine),
            "{key}"
        );
    }
    for key in ["ctrl+shift+z", "alt+z"] {
        assert_eq!(
            map.lookup(&[cortex_tui::keys::Context::Editor], parse(key).unwrap()),
            Some(Action::Redo),
            "{key}"
        );
    }
}
