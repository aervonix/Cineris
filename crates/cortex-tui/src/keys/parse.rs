//! A key binding's text, as `[keys]` writes it: `ctrl+shift+z`, `alt+enter`,
//! `f5`, `/`.
//!
//! Modifiers come first, joined by `+`, in any order: `ctrl` (or `control`),
//! `alt` (or `option`, `meta`), `shift`, `super` (or `cmd`, `win`). The last
//! part is the key: a name (`enter`, `tab`, `backspace`, `esc`, `space`,
//! `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`,
//! `insert`, `delete`, `f1` to `f24`) or one character. Names and letters
//! are not case-sensitive: `Ctrl+Z` is `ctrl+z`, and Shift is written as
//! `shift`. A `+` key is written last after its own `+`: `ctrl++`.

use std::fmt;

use cortex_term::{Key, KeyCode, Mods};

/// Why a binding's text is not a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingError {
    /// The text as given.
    pub text: String,
    /// What is wrong with it.
    pub reason: String,
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a key: {}", self.text, self.reason)
    }
}

impl std::error::Error for BindingError {}

/// The key `text` names.
///
/// # Errors
///
/// When a modifier or the key's name is not known, a modifier is given
/// twice, or there is no key.
pub fn parse(text: &str) -> Result<Key, BindingError> {
    let error = |reason: &str| BindingError {
        text: text.to_owned(),
        reason: reason.to_owned(),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(error("it is empty"));
    }
    // `ctrl++` and `+`: the key is the last `+`.
    let (modifiers, key) = if trimmed == "+" {
        ("", "+")
    } else if let Some(head) = trimmed.strip_suffix("++") {
        (head, "+")
    } else {
        match trimmed.rsplit_once('+') {
            Some(("", _)) => return Err(error("a `+` has nothing before it")),
            Some((head, key)) => (head, key),
            None => ("", trimmed),
        }
    };
    let mut mods = Mods::NONE;
    if !modifiers.is_empty() {
        for part in modifiers.split('+') {
            let one = match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Mods::CTRL,
                "alt" | "option" | "meta" => Mods::ALT,
                "shift" => Mods::SHIFT,
                "super" | "cmd" | "win" => Mods::SUPER,
                "" => return Err(error("a `+` has nothing before it")),
                _ => return Err(error(&format!("`{part}` is not a modifier"))),
            };
            if mods.contains(one) {
                return Err(error(&format!("`{part}` is given twice")));
            }
            mods = mods | one;
        }
    }
    let key = key.trim();
    let code = match key.to_ascii_lowercase().as_str() {
        "" => return Err(error("there is no key after the modifiers")),
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "insert" | "ins" => KeyCode::Insert,
        "delete" | "del" => KeyCode::Delete,
        lower => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_control() => {
                    KeyCode::Char(c.to_lowercase().next().unwrap_or(c))
                }
                _ => match lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                    Some(n @ 1..=24) => KeyCode::F(n),
                    _ => return Err(error(&format!("`{key}` is not a key's name"))),
                },
            }
        }
    };
    Ok(Key::with(code, mods))
}

/// A key as a binding's text: what [`parse`] reads, and what hints show.
#[must_use]
pub fn name(key: Key) -> String {
    let mut out = String::new();
    for (mods, word) in [
        (Mods::CTRL, "ctrl+"),
        (Mods::ALT, "alt+"),
        (Mods::SHIFT, "shift+"),
        (Mods::SUPER, "super+"),
    ] {
        if key.mods.contains(mods) {
            out.push_str(word);
        }
    }
    match key.code {
        KeyCode::Char(' ') => out.push_str("space"),
        KeyCode::Char(c) => out.push(c),
        KeyCode::Enter => out.push_str("enter"),
        KeyCode::Tab => out.push_str("tab"),
        KeyCode::Backspace => out.push_str("backspace"),
        KeyCode::Esc => out.push_str("esc"),
        KeyCode::Up => out.push_str("up"),
        KeyCode::Down => out.push_str("down"),
        KeyCode::Left => out.push_str("left"),
        KeyCode::Right => out.push_str("right"),
        KeyCode::Home => out.push_str("home"),
        KeyCode::End => out.push_str("end"),
        KeyCode::PageUp => out.push_str("pageup"),
        KeyCode::PageDown => out.push_str("pagedown"),
        KeyCode::Insert => out.push_str("insert"),
        KeyCode::Delete => out.push_str("delete"),
        KeyCode::F(n) => out.push_str(&format!("f{n}")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_and_names_in_any_case_and_order() {
        assert_eq!(
            parse("ctrl+shift+z"),
            Ok(Key::with(KeyCode::Char('z'), Mods::CTRL | Mods::SHIFT))
        );
        assert_eq!(
            parse("Shift+Ctrl+Z"),
            Ok(Key::with(KeyCode::Char('z'), Mods::CTRL | Mods::SHIFT))
        );
        assert_eq!(parse("alt+Enter"), Ok(Key::with(KeyCode::Enter, Mods::ALT)));
        assert_eq!(parse("f12"), Ok(Key::plain(KeyCode::F(12))));
        assert_eq!(
            parse("ctrl++"),
            Ok(Key::with(KeyCode::Char('+'), Mods::CTRL))
        );
        assert_eq!(parse("+"), Ok(Key::plain(KeyCode::Char('+'))));
        assert_eq!(
            parse("ctrl+_"),
            Ok(Key::with(KeyCode::Char('_'), Mods::CTRL))
        );
        assert_eq!(parse("space"), Ok(Key::plain(KeyCode::Char(' '))));
    }

    #[test]
    fn what_is_not_a_key_says_why() {
        for bad in ["", "ctrl+", "hyper+a", "ctrl+ctrl+a", "f25", "enterr", "+a"] {
            let error = parse(bad).unwrap_err();
            assert!(!error.reason.is_empty(), "{bad}");
        }
    }

    #[test]
    fn a_name_reads_back_as_the_same_key() {
        for text in [
            "ctrl+shift+z",
            "alt+enter",
            "f5",
            "ctrl+_",
            "space",
            "pagedown",
            "esc",
        ] {
            let key = parse(text).unwrap();
            assert_eq!(parse(&name(key)), Ok(key), "{text}");
        }
    }
}
