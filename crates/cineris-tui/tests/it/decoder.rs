//! Every default binding as terminals send it, through the terminal layer's
//! decoder, to the key map: as plain virtual-terminal input (what xterm
//! sends with no keyboard protocol on, and what Windows sends with
//! virtual-terminal input), as Windows Terminal's win32 input records, and
//! as the enhanced keyboard protocol for the keys plain input cannot tell
//! apart.
//!
//! Plain input merges two bindings into others, as 09-tui's "Windows" table
//! says: `Shift+Enter` arrives as `Enter`, and `Ctrl+Shift+Z` as `Ctrl+Z`.
//! The test says so for each, and checks that the other keys of the same
//! action (`Alt+Enter`, `Ctrl+J`; `Alt+Z`) arrive as themselves.

use cineris_term::{Decoder, Event, Key, KeyCode, Mods};
use cineris_tui::keys::{ACTIONS, Action, Context, Gesture, KeyMap, parse};

/// The contexts a key of `context` is looked up in when that context has
/// the keyboard.
fn chain(context: Context) -> Vec<Context> {
    match context {
        Context::Global | Context::Editor => vec![Context::Editor],
        Context::Menu => vec![Context::Menu, Context::Editor],
        Context::Picker => vec![Context::Picker, Context::Menu],
        Context::Toggle => vec![Context::Toggle, Context::Picker, Context::Menu],
        Context::Fork => vec![Context::Fork, Context::Picker, Context::Menu],
        Context::Search => vec![Context::Search],
        Context::Focus => vec![Context::Focus],
        Context::Dialog => vec![Context::Dialog],
        Context::Viewer => vec![Context::Viewer],
        Context::Screen => vec![Context::Editor, Context::Screen],
    }
}

fn decode(bytes: &[u8]) -> Vec<Event> {
    let mut decoder = Decoder::new();
    let mut events = Vec::new();
    decoder.feed(bytes, &mut events);
    decoder.flush(&mut events);
    events
}

/// The xterm modifier parameter: 1 plus Shift 1, Alt 2, Ctrl 4.
fn param(mods: Mods) -> u8 {
    1 + u8::from(mods.contains(Mods::SHIFT))
        + 2 * u8::from(mods.contains(Mods::ALT))
        + 4 * u8::from(mods.contains(Mods::CTRL))
}

/// What a terminal with no keyboard protocol sends for `key`.
fn plain(key: Key) -> Vec<u8> {
    let alt = key.mods.contains(Mods::ALT);
    let ctrl = key.mods.contains(Mods::CTRL);
    let shift = key.mods.contains(Mods::SHIFT);
    let others = key.mods.contains(Mods::ALT) || ctrl || shift;
    let prefix = |bytes: Vec<u8>| -> Vec<u8> {
        if alt {
            let mut with = vec![0x1b];
            with.extend(bytes);
            with
        } else {
            bytes
        }
    };
    let cursor = |letter: u8| -> Vec<u8> {
        if others {
            format!("\x1b[1;{}{}", param(key.mods), char::from(letter)).into_bytes()
        } else {
            vec![0x1b, b'[', letter]
        }
    };
    let tilde = |number: u8| -> Vec<u8> {
        if others {
            format!("\x1b[{number};{}~", param(key.mods)).into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        }
    };
    match key.code {
        KeyCode::Enter => prefix(vec![b'\r']),
        KeyCode::Tab if shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => prefix(vec![b'\t']),
        KeyCode::Backspace if ctrl => prefix(vec![0x08]),
        KeyCode::Backspace => prefix(vec![0x7f]),
        KeyCode::Esc => prefix(vec![0x1b]),
        KeyCode::Up => cursor(b'A'),
        KeyCode::Down => cursor(b'B'),
        KeyCode::Right => cursor(b'C'),
        KeyCode::Left => cursor(b'D'),
        KeyCode::Home => cursor(b'H'),
        KeyCode::End => cursor(b'F'),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        // Ctrl with a letter is the letter's control code, whatever Shift
        // does; Ctrl+/ and Ctrl+_ are 0x1f.
        KeyCode::Char(c) if ctrl => {
            let code = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                '/' | '_' => 0x1f,
                ' ' => 0,
                other => panic!("no control code for {other}"),
            };
            prefix(vec![code])
        }
        // Shift with a letter is the upper-case letter.
        KeyCode::Char(c) => {
            let c = if shift { c.to_ascii_uppercase() } else { c };
            let mut text = [0; 4];
            prefix(c.encode_utf8(&mut text).as_bytes().to_vec())
        }
        other => panic!("no plain sequence for {other:?}"),
    }
}

/// Windows Terminal's win32 input record for `key` pressed, then released.
fn win32(key: Key) -> Option<Vec<u8>> {
    let ctrl = key.mods.contains(Mods::CTRL);
    let (vk, unit): (u32, u32) = match key.code {
        KeyCode::Enter => (0x0d, 13),
        KeyCode::Tab => (0x09, 9),
        KeyCode::Backspace => (0x08, 8),
        KeyCode::Esc => (0x1b, 27),
        KeyCode::PageUp => (0x21, 0),
        KeyCode::PageDown => (0x22, 0),
        KeyCode::End => (0x23, 0),
        KeyCode::Home => (0x24, 0),
        KeyCode::Left => (0x25, 0),
        KeyCode::Up => (0x26, 0),
        KeyCode::Right => (0x27, 0),
        KeyCode::Down => (0x28, 0),
        KeyCode::Delete => (0x2e, 0),
        // A panel's `space` (step 24).
        KeyCode::Char(' ') if !ctrl => (0x20, 0x20),
        KeyCode::Char(c @ 'a'..='z') => {
            let vk = u32::from(c.to_ascii_uppercase());
            let unit = if ctrl {
                vk - 0x40
            } else if key.mods.contains(Mods::SHIFT) {
                vk
            } else {
                u32::from(c)
            };
            (vk, unit)
        }
        // Punctuation with Ctrl depends on the keyboard's layout, and the
        // record carries a control code or nothing: not a key the map can
        // be given in this mode. Ctrl+Z is undo there.
        KeyCode::Char('/' | '_') if ctrl => return None,
        // The viewer's search (step 25): the key of `/` on a US layout.
        KeyCode::Char('/') => (0xbf, u32::from('/')),
        other => panic!("no record for {other:?}"),
    };
    let mut state = 0;
    if key.mods.contains(Mods::SHIFT) {
        state |= 0x10;
    }
    if ctrl {
        state |= 0x08;
    }
    if key.mods.contains(Mods::ALT) {
        state |= 0x02;
    }
    Some(format!("\x1b[{vk};0;{unit};1;{state};1_\x1b[{vk};0;{unit};0;{state};1_").into_bytes())
}

/// The enhanced keyboard protocol's report of `key` (flag 1: only keys
/// that are ambiguous otherwise).
fn kitty(key: Key) -> Vec<u8> {
    let code = match key.code {
        KeyCode::Enter => 13,
        KeyCode::Tab => 9,
        KeyCode::Backspace => 127,
        KeyCode::Esc => 27,
        KeyCode::Char(c) => u32::from(c),
        _ => return plain(key),
    };
    format!("\x1b[{code};{}u", param(key.mods)).into_bytes()
}

/// The bindings plain input sends as another key, and the key it sends.
const MERGED: [(&str, &str); 2] = [("shift+enter", "enter"), ("ctrl+shift+z", "ctrl+z")];

fn found(map: &KeyMap, context: Context, bytes: &[u8]) -> Option<Action> {
    let events = decode(bytes);
    let keys: Vec<Key> = events
        .iter()
        .filter_map(|event| match event {
            Event::Key(key) => Some(*key),
            _ => None,
        })
        .collect();
    assert_eq!(keys.len(), 1, "{bytes:?} decoded as {events:?}");
    map.lookup(&chain(context), keys[0])
}

#[test]
fn every_default_binding_as_plain_input() {
    let map = KeyMap::defaults();
    for entry in ACTIONS {
        // The mouse's gestures are not keys (step 25).
        for text in entry
            .keys
            .iter()
            .filter(|text| Gesture::from_name(text).is_none())
        {
            let key = parse(text).unwrap();
            let action = found(&map, entry.context, &plain(key));
            match MERGED.iter().find(|(binding, _)| binding == text) {
                Some((_, sent)) => {
                    let as_sent = map.lookup(&chain(entry.context), parse(sent).unwrap());
                    assert_eq!(action, as_sent, "{text} arrives as {sent}");
                    assert_ne!(action, Some(entry.action), "{text} is merged");
                }
                None => assert_eq!(action, Some(entry.action), "{text}: {:?}", plain(key)),
            }
        }
    }
}

#[test]
fn every_default_binding_as_win32_input_records() {
    let map = KeyMap::defaults();
    let mut skipped = Vec::new();
    for entry in ACTIONS {
        // The mouse's gestures are not keys (step 25).
        for text in entry
            .keys
            .iter()
            .filter(|text| Gesture::from_name(text).is_none())
        {
            let key = parse(text).unwrap();
            match win32(key) {
                Some(bytes) => {
                    assert_eq!(
                        found(&map, entry.context, &bytes),
                        Some(entry.action),
                        "{text}"
                    );
                }
                None => skipped.push(*text),
            }
        }
    }
    assert_eq!(
        skipped,
        ["ctrl+_", "ctrl+/"],
        "only the layout-dependent undo keys"
    );
}

#[test]
fn the_merged_keys_arrive_whole_under_the_enhanced_protocol() {
    let map = KeyMap::defaults();
    for (text, _) in MERGED {
        let key = parse(text).unwrap();
        let action = found(&map, Context::Editor, &kitty(key));
        assert_eq!(action.unwrap().name(), {
            let expected = if text == "shift+enter" {
                Action::NewLine
            } else {
                Action::Redo
            };
            expected.name()
        });
    }
    // And every binding with a modifier, as the protocol reports it.
    for entry in ACTIONS {
        // The mouse's gestures are not keys (step 25).
        for text in entry
            .keys
            .iter()
            .filter(|text| Gesture::from_name(text).is_none())
        {
            let key = parse(text).unwrap();
            if key.mods.is_empty() || matches!(key.code, KeyCode::Char('_' | '/')) {
                continue;
            }
            assert_eq!(
                found(&map, entry.context, &kitty(key)),
                Some(entry.action),
                "{text}"
            );
        }
    }
}

#[test]
fn a_new_line_always_has_a_key_plain_input_sends_as_itself() {
    let map = KeyMap::defaults();
    for text in ["alt+enter", "ctrl+j"] {
        let bytes = plain(parse(text).unwrap());
        assert_eq!(
            found(&map, Context::Editor, &bytes),
            Some(Action::NewLine),
            "{text}"
        );
    }
    let bytes = plain(parse("alt+z").unwrap());
    assert_eq!(found(&map, Context::Editor, &bytes), Some(Action::Redo));
}
