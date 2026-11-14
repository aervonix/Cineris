#![allow(
    clippy::disallowed_methods,
    reason = "the tests read the reference screens"
)]

use cortex_term::{ColorDepth, Event, Key, KeyCode};

use super::*;
use crate::complete::{Command, Row, Toggle};
use crate::glyphs::Glyphs;
use crate::history::MemoryHistory;
use crate::keys::KeyMap;
use crate::testing::{self, DARK};
use crate::theme::Theme;

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn input() -> Input {
    let history = MemoryHistory::from_prompts(
        [
            "fix the lexer",
            "run clippy\nand the tests",
            "\tfn main() {}",
        ]
        .map(String::from),
    );
    let mut input = Input::new(KeyMap::defaults(), Box::new(history));
    input.set_files(
        (0..20)
            .map(|n| format!("src/lex/part_{n:02}.rs"))
            .chain([
                "README.md".to_owned(),
                "src/漢字/名前.rs".to_owned(),
                "odd\tname\n.rs".to_owned(),
            ])
            .collect(),
    );
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

fn typed(text: &str) -> Input {
    let mut input = input();
    for c in text.chars() {
        input.handle(&Event::Key(Key::plain(KeyCode::Char(c))));
    }
    input
}

/// The `/model` picker of screen 3a. Its rows' notes are laid out in
/// columns by whoever opens it.
fn model_picker() -> Menu {
    let rows = [
        (
            "opus-5.5",
            " anthropic   200k  $5 · $25 /M    fast  current",
        ),
        ("sonnet-5.5", " anthropic   200k  $3 · $15 /M"),
        ("haiku-5.5", " anthropic   200k  $1 · $5 /M"),
        ("fable-5.2", " anthropic   200k  $15 · $75 /M   fast"),
        ("deepseek-v4", " deepseek    128k  $0.27 · $1.10 /M"),
        ("qwen3-coder", " openrouter  256k  $0.40 · $1.60 /M"),
    ];
    let mut menu = Menu::picker(
        "model",
        rows.iter().map(|(l, d)| Row::new(*l, *d)).collect(),
        0,
        None,
    );
    menu.select(1);
    menu
}

/// The `/effort` picker of screen 3b, with its fast-mode row.
fn effort_picker() -> Menu {
    let rows = [
        ("low", "a quick look, least reasoning"),
        ("medium", "balanced"),
        ("high", "careful work; the default for this model   current"),
        ("max", "the most reasoning the model allows"),
    ];
    Menu::picker(
        "effort",
        rows.iter().map(|(l, d)| Row::new(*l, *d)).collect(),
        2,
        Some(Toggle {
            label: "fast mode".to_owned(),
            on: true,
            note: "· about 2.5x the output speed, at 2x the price".to_owned(),
        }),
    )
}

fn states() -> Vec<(&'static str, Input)> {
    let mut scrolled = typed("@part");
    for _ in 0..10 {
        scrolled.act(Action::MenuDown);
    }
    let mut model = typed("/model");
    model.open_picker(model_picker());
    let mut effort = typed("/effort");
    effort.open_picker(effort_picker());
    let mut filtered = typed("/effort");
    filtered.open_picker(effort_picker());
    for c in "me".chars() {
        filtered.handle(&Event::Key(Key::plain(KeyCode::Char(c))));
    }
    let mut search = input();
    search.act(Action::HistorySearch);
    for c in "clip".chars() {
        search.handle(&Event::Key(Key::plain(KeyCode::Char(c))));
    }
    let mut no_match = input();
    no_match.act(Action::HistorySearch);
    for c in "zzz".chars() {
        no_match.handle(&Event::Key(Key::plain(KeyCode::Char(c))));
    }
    let mut tab = input();
    tab.act(Action::HistorySearch);
    for c in "fn".chars() {
        tab.handle(&Event::Key(Key::plain(KeyCode::Char(c))));
    }
    let mut listing = Input::new(KeyMap::defaults(), Box::new(MemoryHistory::new()));
    listing.handle(&Event::Key(Key::plain(KeyCode::Char('@'))));
    vec![
        ("files, more than eight", typed("@")),
        ("files, one", typed("@名前")),
        ("files, none", typed("@qqq")),
        ("files, scrolled", scrolled),
        ("files, not listed yet", listing),
        ("commands", typed("/mo")),
        ("the /model picker", model),
        ("the /effort picker with fast mode", effort),
        ("a picker filtered", filtered),
        ("history search", search),
        ("history search, no match", no_match),
        ("history search, a prompt with a tab", tab),
        ("files, a name with a tab and a line break", typed("@oddna")),
    ]
}

#[test]
fn the_menus_in_every_variant() {
    let mut parts = Vec::new();
    for variant in testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        for (name, input) in states() {
            let lines = menu(&input, &look, variant.width()).expect("a menu is open");
            let problems = testing::problems(&lines, variant.width());
            assert!(problems.is_empty(), "{name}: {problems:?}");
            parts.push((format!("{} — {name}", variant.title()), lines));
        }
    }
    testing::assert_snapshot("menu", &testing::sections(&parts));
}

#[test]
fn no_menu_no_lines() {
    assert_eq!(menu(&input(), &look(), 80), None);
}

#[test]
fn the_reference_screens_menus_are_drawn_cell_for_cell() {
    let look = look();
    for (screen, picker, first, count) in
        [("3a", model_picker(), 5, 6), ("3b", effort_picker(), 5, 6)]
    {
        let reference = std::fs::read_to_string(format!(
            "{}/../../docs/architecture/tui-look/{screen}.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .replace("\r\n", "\n");
        let expected: Vec<&str> = reference.lines().skip(first).take(count).collect();
        let mut input = input();
        input.open_picker(picker);
        let got: Vec<String> = menu(&input, &look, 99)
            .unwrap()
            .iter()
            .map(|line| line.text().into_owned())
            .collect();
        assert_eq!(got, expected, "screen {screen}");
    }
}

#[test]
fn a_resize_while_a_menu_is_open_keeps_every_line_to_the_new_width() {
    let look = look();
    for (name, input) in states() {
        for width in (1..=120).rev() {
            let lines = menu(&input, &look, width).unwrap();
            assert!(
                testing::problems(&lines, width).is_empty(),
                "{name} at {width}: {:?}",
                testing::problems(&lines, width)
            );
        }
    }
}

#[test]
fn the_selected_row_is_marked_in_the_accent_and_drawn_as_the_selection() {
    let look = look();
    let input = typed("/mo");
    let lines = menu(&input, &look, 80).unwrap();
    let selected = &lines[0];
    let mark = selected.spans().iter().find(|s| s.text() == "◆").unwrap();
    assert_eq!(mark.style(), look.theme.style(Token::Accent));
    let label = selected
        .spans()
        .iter()
        .find(|s| s.text().starts_with("mode"))
        .unwrap();
    assert_eq!(label.style(), look.theme.style(Token::Selected));
    assert!(!lines[1].text().contains('◆'));
}
