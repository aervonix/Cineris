//! The integration tests of `cortex-tui`'s input: every default binding
//! from key to action to state, the bindings through the terminal layer's
//! decoder as terminals send them, the editor's grapheme and undo rules,
//! paste, history, the menus, and the external editor (tier 3, `sys_…`).
//! And its views: the reference screens drawn again with them through the
//! inline presenter into the test kit's virtual terminal (`screens`), and
//! their share of a frame (`frames`).

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "tests: the external editor's children are processes, and a test fails by panicking"
)]

mod bindings;
mod children;
mod decoder;
#[path = "../../examples/editor-demo/app.rs"]
#[allow(
    dead_code,
    reason = "the demo is the example's, and this binary uses part of it"
)]
mod demo;
mod editing;
mod external;
mod frames;
mod input;
mod perf;
mod screens;
#[cfg(unix)]
mod signals;

use cortex_term::{Event, Key};
use cortex_tui::complete::Command;
use cortex_tui::history::MemoryHistory;
use cortex_tui::input::{Input, Outcome};
use cortex_tui::keys::{KeyMap, parse};

/// The prompts the test history holds, oldest first.
pub(crate) const HISTORY: [&str; 3] = ["fix the lexer", "run clippy\nand the tests", "newest"];

/// An input with the default keys, the test history, three files and two
/// commands, its text wrapping at 40 columns.
pub(crate) fn input() -> Input {
    let history = MemoryHistory::from_prompts(HISTORY.map(String::from));
    let mut input = Input::new(KeyMap::defaults(), Box::new(history));
    input.set_width(40);
    input.set_files(vec![
        "src/lex.rs".to_owned(),
        "src/main.rs".to_owned(),
        "README.md".to_owned(),
    ]);
    input.set_commands(vec![
        Command {
            name: "model".to_owned(),
            description: "pick the model".to_owned(),
        },
        Command {
            name: "mode".to_owned(),
            description: "pick the level".to_owned(),
        },
    ]);
    input
}

/// An input holding `text`, with the cursor at its end.
pub(crate) fn input_with(text: &str) -> Input {
    let mut input = input();
    type_text(&mut input, text);
    input
}

/// Types `text` a character at a time, as keys; a `\n` as `Ctrl+J`.
pub(crate) fn type_text(input: &mut Input, text: &str) {
    for c in text.chars() {
        let key = if c == '\n' {
            parse("ctrl+j").unwrap()
        } else {
            Key::plain(cortex_term::KeyCode::Char(c))
        };
        input.handle(&Event::Key(key));
    }
}

/// Presses the key `binding` names.
pub(crate) fn press(input: &mut Input, binding: &str) -> Outcome {
    input.handle(&Event::Key(parse(binding).unwrap()))
}

/// Becomes one of the external editor tests' children when the arguments
/// say so ([`children`]), and passes at once otherwise.
#[test]
fn child_entry() {
    if let Some(options) = DEMO.options() {
        let options = demo::Options::parse(options.split(',')).expect("the demo's options");
        demo::run(&options).expect("the demo runs");
        #[expect(clippy::exit, reason = "a child ends when the demo does")]
        std::process::exit(0);
    }
    children::enter();
    #[cfg(unix)]
    signals::enter();
}

/// This binary as the editor demo, with its options, separated by commas.
pub(crate) const DEMO: cortex_testkit::role::Role = cortex_testkit::role::Role::new("cxtui-demo=");
