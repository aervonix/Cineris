//! The look and the key map read again while the application runs (step
//! 24's hot reload): the look at the files after a key's frame or a
//! focus-in, at most once a second; the sections asked for again when a
//! reload applied a key of them; a theme chosen with `/theme` kept unless
//! the file's own changed; the key map replaced only when a binding
//! changed; and a section that cannot be read said, the old ones kept.

use super::*;
use cortex_protocol::event::ConfigChanged;
use cortex_protocol::methods::{ConfigGetResult, ConfigReloadResult};

use crate::app::{Restyled, ThemeChoice};
use crate::keys::Action;

/// A restyle as the program's would be, over the two keys these tests
/// change: `tui.theme`, and `keys.editor`'s table. `tui.theme = "broken"`
/// stands for a section that does not read.
fn restyle(config: &serde_json::Map<String, serde_json::Value>) -> Result<Restyled, String> {
    let theme = match config.get("tui.theme").and_then(|value| value.as_str()) {
        Some("light") => ThemeChoice::Light,
        Some("dark") => ThemeChoice::Dark,
        Some("broken") => return Err("the look could not be read again".to_owned()),
        _ => ThemeChoice::Auto,
    };
    let mut table: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(editor) = config.get("keys.editor").and_then(|value| value.as_object()) {
        for (action, keys) in editor {
            let keys = keys
                .as_array()
                .map(|keys| {
                    keys.iter()
                        .filter_map(|key| key.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            table.push((format!("editor.{action}"), keys));
        }
    }
    let (keymap, _) = crate::keys::KeyMap::with_table(
        table
            .iter()
            .map(|(name, keys)| (name.as_str(), keys.as_slice())),
    );
    Ok(Restyled {
        settings: Settings {
            theme,
            ..Settings::default()
        },
        keymap,
        notices: Vec::new(),
    })
}

fn restyled_app(now: Instant) -> (App, Scripted) {
    let (mut app, link) = app(now);
    app.set_restyle(Box::new(restyle));
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    (app, link)
}

/// The looks at the files sent since the last call.
fn looks(link: &Scripted) -> Vec<Ticket> {
    link.take_sent()
        .into_iter()
        .filter_map(|(ticket, op)| match op {
            Op::ConfigReload(params) => {
                assert!(params.changed_only, "a look is of what changed: {params:?}");
                Some(ticket)
            }
            _ => None,
        })
        .collect()
}

/// A frame drawn at `at`, as the loop draws one after input.
fn frame_at(app: &mut App, at: Instant) {
    let _ = app.frame(at);
    app.drawn(at);
}

fn changed(applied: &[&str]) -> Inbound {
    Inbound::Event(SessionEvent {
        session_id: SessionId::from(SESSION),
        seq: 40,
        turn_id: None,
        event: Event::ConfigChanged(ConfigChanged {
            applied: applied.iter().map(|key| (*key).to_owned()).collect(),
            ..ConfigChanged::default()
        }),
    })
}

/// The sections the application asked for again, answered with `config`.
fn answer_sections(app: &mut App, link: &Scripted, config: &[(&str, serde_json::Value)], now: Instant) {
    let (ticket, op) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::ConfigGet(_)))
        .expect("the sections are asked for again");
    let Op::ConfigGet(params) = op else {
        unreachable!()
    };
    assert_eq!(params.keys, ["tui", "keys"]);
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigGet(ConfigGetResult {
            config: config
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
            origins: Default::default(),
        })),
    ));
    drain(app, now);
}

/// A key asks the core to look at the files after its frame, at most once
/// a second and never while a look is out; the terminal's focus regained
/// asks too.
#[test]
fn a_key_or_the_focus_asks_for_a_look_after_its_frame_at_most_once_a_second() {
    let now = Instant::now();
    let (mut app, link) = restyled_app(now);
    key(&mut app, "x", now);
    assert!(looks(&link).is_empty(), "looked before the key's frame");
    frame_at(&mut app, now);
    let first = looks(&link);
    assert_eq!(first.len(), 1);

    // Within the second, and while the first is out: none.
    let soon = now + Duration::from_millis(400);
    key(&mut app, "y", soon);
    frame_at(&mut app, soon);
    assert!(looks(&link).is_empty(), "two looks within a second");
    link.give(Inbound::Done(
        first[0],
        Ok(OpResult::ConfigReload(ConfigReloadResult::default())),
    ));
    drain(&mut app, soon);

    let later = now + Duration::from_millis(1_100);
    app.terminal(&TermEvent::Focus(true), later);
    frame_at(&mut app, later);
    assert_eq!(looks(&link).len(), 1, "the focus regained asks for a look");
}

/// A reload that applied a key of the look, whoever's it was, reads the
/// look again; the key map stays, its readiness untouched, when no binding
/// changed.
#[test]
fn a_reload_of_the_look_reads_it_again_and_leaves_the_keys() {
    let now = Instant::now();
    let (mut app, link) = restyled_app(now);
    link.give(changed(&["tui.theme"]));
    drain(&mut app, now);
    answer_sections(
        &mut app,
        &link,
        &[
            ("tui.theme", "light".into()),
            ("keys.editor", serde_json::json!({"submit": ["ctrl+j"]})),
        ],
        now,
    );
    assert_eq!(app.settings().theme, ThemeChoice::Light);
    let submit = app.input().keymap().keys_of(Action::Submit);
    assert!(
        !submit.contains(&parse("ctrl+j").unwrap()),
        "the key map was replaced for a change of the look"
    );
}

/// A binding reloaded is the next key's: the key map is replaced.
#[test]
fn a_binding_reloaded_reaches_the_next_key() {
    let now = Instant::now();
    let (mut app, link) = restyled_app(now);
    link.give(changed(&["keys.editor"]));
    drain(&mut app, now);
    answer_sections(
        &mut app,
        &link,
        &[("keys.editor", serde_json::json!({"submit": ["ctrl+j"]}))],
        now,
    );
    let submit = app.input().keymap().keys_of(Action::Submit);
    assert!(submit.contains(&parse("ctrl+j").unwrap()), "{submit:?}");
}

/// A theme chosen with `/theme` for this run stays through a reload of
/// another key of the look, and gives way to the file's own theme when
/// that changed.
#[test]
fn a_theme_chosen_for_the_run_stays_unless_the_files_own_changed() {
    let now = Instant::now();
    let (mut app, link) = restyled_app(now);
    run_command(&mut app, "/theme", now);
    // auto, dark, light: dark.
    key(&mut app, "down", now);
    key(&mut app, "enter", now);
    assert_eq!(app.settings().theme, ThemeChoice::Dark);
    let _ = link.take_sent();

    link.give(changed(&["tui.prose_width"]));
    drain(&mut app, now);
    answer_sections(&mut app, &link, &[("tui.theme", "light".into())], now);
    assert_eq!(app.settings().theme, ThemeChoice::Dark, "the run's choice was lost");

    link.give(changed(&["tui.theme"]));
    drain(&mut app, now);
    answer_sections(&mut app, &link, &[("tui.theme", "light".into())], now);
    assert_eq!(app.settings().theme, ThemeChoice::Light);
}

/// A section that cannot be read is said, and the look stays as it was.
#[test]
fn a_look_that_cannot_be_read_is_said_and_the_old_one_kept() {
    let now = Instant::now();
    let (mut app, link) = restyled_app(now);
    link.give(changed(&["tui.theme"]));
    drain(&mut app, now);
    answer_sections(&mut app, &link, &[("tui.theme", "broken".into())], now);
    assert_eq!(app.settings().theme, ThemeChoice::Auto);
    assert!(notices(&mut app, now).contains("could not be read again"));
}
