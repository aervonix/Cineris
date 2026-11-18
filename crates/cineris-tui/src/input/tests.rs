//! The input's rules of step 24: the item edit in the prompt's own editor,
//! a key map replaced, and keys a focused box or panel does not take going
//! to the editor.

use cineris_term::{Event, Key, KeyCode};

use super::*;
use crate::history::MemoryHistory;
use crate::keys::parse;

fn input() -> Input {
    let mut input = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    input.set_width(40);
    input
}

fn press(input: &mut Input, binding: &str) -> Outcome {
    input.handle(&Event::Key(parse(binding).expect("a key")))
}

fn type_text(input: &mut Input, text: &str) -> Vec<Outcome> {
    text.chars()
        .map(|c| input.handle(&Event::Key(Key::plain(KeyCode::Char(c)))))
        .collect()
}

fn text(input: &Input) -> String {
    input.editor().expanded()
}

#[test]
fn an_item_edit_saves_with_enter_and_brings_the_draft_back_whole() {
    let mut input = input();
    type_text(&mut input, "a draft");
    input.act(Action::WordLeft);
    let cursor = input.editor().cursor();
    input.begin_item_edit("reject a trailing _", "saves the item");
    assert_eq!(text(&input), "reject a trailing _");
    assert_eq!(input.editor().cursor(), "reject a trailing _".len());
    assert_eq!(
        input.item_edit(),
        Some(vec![
            "enter saves the item".to_owned(),
            "esc cancels".to_owned()
        ])
    );
    type_text(&mut input, " too");
    assert_eq!(
        press(&mut input, "enter"),
        Outcome::ItemEdited("reject a trailing _ too".to_owned())
    );
    assert_eq!(input.item_edit(), None);
    assert_eq!(text(&input), "a draft", "the draft is back");
    assert_eq!(input.editor().cursor(), cursor, "with its cursor");
    // Nothing reached the history: `Up` on the draft finds no prompt.
    assert_eq!(press(&mut input, "up"), Outcome::Nothing);
    assert_eq!(text(&input), "a draft");
}

#[test]
fn an_item_edit_cancels_with_esc_and_is_never_a_prompt() {
    let mut input = input();
    type_text(&mut input, "draft");
    input.set_running(true);
    input.begin_item_edit("", "saves the item");
    type_text(&mut input, "new item");
    // `Esc` cancels the edit, and does not stop the turn.
    assert_eq!(press(&mut input, "esc"), Outcome::ItemEditCancelled);
    assert_eq!(text(&input), "draft");
    assert_eq!(input.item_edit(), None);
    // Ended, `Esc` is the turn's again.
    assert_eq!(press(&mut input, "esc"), Outcome::Interrupt);
    // An empty item is given as it is: the panel decides.
    input.begin_item_edit("", "saves the item");
    assert_eq!(
        press(&mut input, "enter"),
        Outcome::ItemEdited(String::new())
    );
}

#[test]
fn an_item_edit_is_one_line_with_no_menu_and_no_history() {
    let history = MemoryHistory::from_prompts(["an earlier prompt".to_owned()]);
    let mut input = Input::new(KeyMap::defaults(), Box::new(history));
    input.set_commands(vec![crate::complete::Command {
        name: "plan".to_owned(),
        description: String::new(),
    }]);
    input.begin_item_edit("", "saves the item");
    type_text(&mut input, "/pl");
    assert!(input.menu().is_none(), "no command menu");
    assert_eq!(press(&mut input, "up"), Outcome::Nothing, "no history");
    assert_eq!(
        press(&mut input, "alt+enter"),
        Outcome::Ignored(Action::NewLine)
    );
    assert_eq!(
        press(&mut input, "ctrl+r"),
        Outcome::Ignored(Action::HistorySearch)
    );
    assert_eq!(text(&input), "/pl");
    assert_eq!(
        press(&mut input, "enter"),
        Outcome::ItemEdited("/pl".to_owned())
    );
    // The draft's own menu comes back with it.
    type_text(&mut input, "/pl");
    assert!(input.menu().is_some());
}

#[test]
fn an_item_edit_takes_the_focus_and_another_ends_the_first() {
    let mut input = input();
    input.focus_mut().update(&[], &[2]);
    input.act(Action::FocusNext);
    input.begin_item_edit("one", "saves the item");
    assert!(input.focus().on_input());
    input.begin_item_edit("two", "saves the item");
    assert_eq!(text(&input), "two");
    assert!(input.cancel_item_edit());
    assert_eq!(text(&input), "", "the first draft, not the first item");
    assert!(!input.cancel_item_edit());
}

#[test]
fn the_item_edits_hint_names_the_keys_the_map_gives() {
    let submit = vec!["ctrl+s".to_owned()];
    let (map, errors) = KeyMap::with_table([("editor.submit", submit.as_slice())]);
    assert!(errors.is_empty());
    let mut input = input();
    input.set_keymap(map);
    input.begin_item_edit("x", "saves the item");
    assert_eq!(
        input.item_edit().unwrap()[0],
        "ctrl+s saves the item",
        "the new map's key"
    );
    assert_eq!(
        press(&mut input, "ctrl+s"),
        Outcome::ItemEdited("x".to_owned())
    );
}

#[test]
fn a_new_key_map_takes_the_next_key() {
    let mut input = input();
    type_text(&mut input, "hello");
    let keys = vec!["ctrl+s".to_owned()];
    let (map, _) = KeyMap::with_table([("editor.submit", keys.as_slice())]);
    input.set_keymap(map);
    assert_eq!(press(&mut input, "enter"), Outcome::Nothing);
    assert_eq!(
        press(&mut input, "ctrl+s"),
        Outcome::Submit {
            text: "hello".to_owned(),
            delivery: Delivery::Prompt
        }
    );
}

#[test]
fn typing_with_a_box_or_a_panel_focused_reaches_the_editor_in_order() {
    for focused in [Target::ToolBox(7), Target::Panel(3)] {
        let mut input = input();
        match focused {
            Target::ToolBox(id) => input.focus_mut().update(&[id], &[]),
            Target::Panel(id) => input.focus_mut().update(&[], &[id]),
            Target::Input => unreachable!(),
        }
        input.act(Action::FocusNext);
        assert_eq!(input.focus().focused(), focused);
        // `q` is no key of the focus's: typed, and the focus comes back.
        let outcomes = type_text(&mut input, "quiz");
        assert_eq!(outcomes[0], Outcome::Focused(Target::Input), "{focused:?}");
        assert_eq!(text(&input), "quiz", "{focused:?}: none lost or reordered");
        assert!(input.focus().on_input());
    }
    // A panel's own key is the panel's, not typed; a box has no use for it.
    let mut panel = input();
    panel.focus_mut().update(&[], &[3]);
    panel.act(Action::FocusNext);
    assert_eq!(
        press(&mut panel, "e"),
        Outcome::Panel(Target::Panel(3), Action::FocusEdit)
    );
    assert_eq!(text(&panel), "");
    // What the application passes on goes to the editor, the focus with it.
    assert_eq!(
        panel.pass_to_editor(parse("e").unwrap()),
        Outcome::Focused(Target::Input)
    );
    assert_eq!(text(&panel), "e");
    assert_eq!(
        panel.pass_to_editor(parse("alt+up").unwrap()),
        Outcome::Nothing
    );
    let mut boxed = input();
    boxed.focus_mut().update(&[7], &[]);
    boxed.act(Action::FocusNext);
    assert_eq!(press(&mut boxed, "e"), Outcome::Focused(Target::Input));
    assert_eq!(text(&boxed), "e");
}
