//! A pty test sends a key through `send_key`, never as a raw escape
//! sequence (`docs/testing-and-ci.md`, "Testing the TUI", since the twelfth
//! tooling step).
//!
//! On Windows the program under test sits behind a pseudo-console that has
//! asked for Windows Terminal's win32 input mode, and there a key arrives as
//! input records. A plain xterm sequence typed into it is misread or
//! dropped, so a test that writes `b"\x1b[5~"` for PageUp passes on Linux and
//! means something else on Windows (`docs/conpty.md`). The test kit's
//! `Session::send_key` sends each key as the terminal would on each system.
//!
//! The rule reads every literal handed to a `.send(` call in the crates'
//! Rust files, decodes its escapes, and fails on each escape sequence in it
//! that is a key: a lone Escape, Alt with a character (`ESC x`), `SS3 X`
//! and a `CSI` sequence that ends as a key does. What is not a key passes:
//! the mouse's SGR reports (`CSI < … M`), bracketed paste's marks
//! (`CSI 200 ~`, `CSI 201 ~`), focus reports (`CSI I`, `CSI O`), and a
//! terminal's answers (`CSI ? …`, a sequence with an intermediate such as
//! `$`, an `OSC`). A test of the input decoder itself, which must send the
//! raw bytes, says so on the line or the line before: `// raw key: WHY`.
//!
//! It cannot see a sequence built at run time (`format!`), which is how a
//! test writes the win32 records it checks; that is a test of the decoder.

use std::collections::BTreeMap;

use super::Violation;
use crate::mutate::lex;

/// What marks a raw key as meant, with the reason after it.
const ALLOW: &str = "// raw key:";

/// The file that defines `send_key`, whose own sequences are the point.
const DEFINER: &str = "crates/cineris-testkit/src/pty.rs";

/// Whether the rule reads a file: the crates' Rust files.
pub(super) fn is_read(file: &str) -> bool {
    file.starts_with("crates/") && file.ends_with(".rs") && file != DEFINER
}

/// Checks every file of `sources` the rule reads.
pub(super) fn check(sources: &BTreeMap<String, String>) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (file, source) in sources {
        // Only a file that sends something and writes an escape is lexed.
        if is_read(file)
            && source.contains(".send(")
            && (source.contains("\\x1b") || source.contains("\\x1B") || source.contains("\\u{1b}"))
        {
            violations.extend(check_file(file, source));
        }
    }
    violations
}

fn check_file(file: &str, source: &str) -> Vec<Violation> {
    let all = lex::tokens(source);
    let text = |index: usize| all[index].text(source);
    let lines: Vec<&str> = source.lines().collect();
    let mut violations = Vec::new();
    for index in 0..all.len().saturating_sub(3) {
        if text(index) != "." || text(index + 1) != "send" || text(index + 2) != "(" {
            continue;
        }
        // The argument's literals: `b"…"`, `&b"…"`, `"…".as_bytes()`.
        let mut at = index + 3;
        let mut depth = 0usize;
        let mut keys = Vec::new();
        while at < all.len() {
            match text(at) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" if depth == 0 => break,
                ")" | "]" | "}" => depth -= 1,
                literal => {
                    if let Some(content) = lex::string_content(literal) {
                        let raw = literal.starts_with("br") || literal.starts_with('r');
                        let bytes = if raw {
                            literal[content].as_bytes().to_vec()
                        } else {
                            unescape(&literal[content])
                        };
                        keys.extend(keys_in(&bytes));
                    }
                }
            }
            at += 1;
        }
        if keys.is_empty() {
            continue;
        }
        let line = lex::line_of(source, all[index].start);
        let marked = |line: usize| {
            line.checked_sub(1)
                .and_then(|at| lines.get(at))
                .and_then(|text| text.split_once(ALLOW))
                .is_some_and(|(_, reason)| !reason.trim().is_empty())
        };
        if marked(line) || marked(line - 1) {
            continue;
        }
        violations.push(Violation::new(
            file,
            Some(line),
            format!(
                "a pty test sends {} as raw bytes. Under Windows' win32 input mode a \
                 pseudo-console misreads or drops a plain sequence: send each key with \
                 `session.send_key(NAME)` (`escape`, `up`, `pageup`, `f5`, with `shift+`, \
                 `ctrl+` or `alt+` before it: cineris_testkit::pty::key_bytes). A test of the \
                 input decoder itself, which must send the bytes, says so on the line or the \
                 line before: `{ALLOW} WHY`.",
                keys.join(", ")
            ),
        ));
    }
    violations
}

/// The bytes a string literal's text stands for: its escapes decoded.
fn unescape(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buffer = [0; 4];
            out.extend(c.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        match chars.next() {
            Some('x') => {
                let digits: String = chars.by_ref().take(2).collect();
                out.extend(u8::from_str_radix(&digits, 16).ok());
            }
            Some('u') => {
                let digits: String = chars
                    .by_ref()
                    .skip_while(|&c| c == '{')
                    .take_while(|&c| c != '}')
                    .collect();
                if let Some(c) = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    let mut buffer = [0; 4];
                    out.extend(c.encode_utf8(&mut buffer).as_bytes());
                }
            }
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('0') => out.push(0),
            Some('\n') => {
                // A line continued: the white space at the next line's start
                // is not part of the text.
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
            }
            Some(other) => {
                let mut buffer = [0; 4];
                out.extend(other.encode_utf8(&mut buffer).as_bytes());
            }
            None => {}
        }
    }
    out
}

/// Each escape sequence in `bytes` that is a key, written for a message
/// (`ESC [ 5 ~`).
fn keys_in(bytes: &[u8]) -> Vec<String> {
    const ESC: u8 = 0x1b;
    let mut keys = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != ESC {
            at += 1;
            continue;
        }
        let start = at;
        let (end, key) = match bytes.get(at + 1) {
            // A lone Escape, or one before another: the Escape key.
            None | Some(&ESC) => (at + 1, true),
            Some(b'[') => {
                // CSI: parameters, intermediates, a final byte.
                let mut end = at + 2;
                while end < bytes.len() && (0x20..=0x3f).contains(&bytes[end]) {
                    end += 1;
                }
                let body = &bytes[at + 2..end.min(bytes.len())];
                let last = bytes.get(end).copied();
                let end = (end + 1).min(bytes.len());
                (end, csi_is_key(body, last))
            }
            // SS3 and its letter: F1 to F4, and keypad keys.
            Some(b'O') => ((at + 3).min(bytes.len()), bytes.len() > at + 2),
            // OSC and the other strings: answers, not keys. They end at
            // BEL or ST; the rest of the literal is passed over.
            Some(b']' | b'P' | b'_' | b'^') => (bytes.len(), false),
            // Alt with a character.
            Some(_) => (at + 2, true),
        };
        if key {
            keys.push(spelled(&bytes[start..end]));
        }
        at = end;
    }
    keys
}

/// Whether a CSI sequence with these parameters and final byte is a key.
fn csi_is_key(body: &[u8], last: Option<u8>) -> bool {
    let Some(last) = last else {
        return false;
    };
    // A private answer (`CSI ? …`), the mouse (`CSI < …`), or anything
    // with an intermediate byte (`CSI … $ y`): not a key.
    if body
        .first()
        .is_some_and(|byte| matches!(byte, b'?' | b'<' | b'>' | b'='))
        || body.iter().any(|byte| (0x20..=0x2f).contains(byte))
    {
        return false;
    }
    match last {
        // Focus in and out.
        b'I' | b'O' => false,
        // Bracketed paste's marks; any other number is a key.
        b'~' => !matches!(body, b"200" | b"201"),
        // A cursor position answer has a row; F3 with modifiers is `1 ; m R`.
        b'R' => body.is_empty() || body.starts_with(b"1;"),
        b'A' | b'B' | b'C' | b'D' | b'E' | b'F' | b'H' | b'P' | b'Q' | b'S' | b'Z' | b'u'
        | b'_' => true,
        _ => false,
    }
}

/// A sequence as a message writes it: `ESC[5~`.
fn spelled(bytes: &[u8]) -> String {
    let text: String = bytes
        .iter()
        .map(|&byte| match byte {
            0x1b => "ESC".to_owned(),
            0x20..=0x7e => char::from(byte).to_string(),
            other => format!("<{other:02x}>"),
        })
        .collect();
    format!("`{text}`")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{check, keys_in};

    fn found(source: &str) -> Vec<String> {
        let sources = BTreeMap::from([(
            "crates/cineris-cli/tests/system/tui.rs".to_owned(),
            source.to_owned(),
        )]);
        check(&sources).iter().map(ToString::to_string).collect()
    }

    /// Planted raw keys fail, each named, with what to do; the same keys
    /// through `send_key` pass.
    #[test]
    fn a_raw_key_sent_to_a_pty_fails() {
        for planted in [
            r#"session.send(b"\x1b[5~");"#,
            r#"session.send(b"\x1b");"#,
            r#"session.send(b"\x1bOP");"#,
            r#"session.send(b"abc\x1b[1;5A");"#,
            r#"session.send("\u{1b}x".as_bytes());"#,
            r#"session.send(&b"\x1B[D"[..]);"#,
        ] {
            let found = found(planted);
            assert_eq!(found.len(), 1, "{planted}: {found:?}");
            assert!(found[0].contains("send_key"), "{found:?}");
            assert!(found[0].starts_with("crates/cineris-cli/tests/system/tui.rs:1:"));
        }
        assert!(found(r#"session.send_key("pageup"); session.send(b"\x03");"#).is_empty());
    }

    /// The mouse, bracketed paste, focus and a terminal's answers are not
    /// keys; a mark with a reason lets a decoder's test send raw bytes, and
    /// a mark with none does not.
    #[test]
    fn what_is_not_a_key_and_a_marked_line_pass() {
        for passes in [
            r#"session.send(b"\x1b[<64;10;5M\x1b[<0;4;3m");"#,
            r#"session.send(b"\x1b[200~text\x1b[201~");"#,
            r#"session.send(b"\x1b[I\x1b[O");"#,
            r#"session.send(b"\x1b[?9001;0$y");"#,
            r#"session.send(b"\x1b]11;rgb:00/00/00\x1b\\");"#,
            r#"session.send(b"\x1b[24;80R");"#,
            "// raw key: a test of the decoder\nsession.send(b\"\\x1b[A\");",
            "session.send(b\"\\x1bx\"); // raw key: Alt as plain bytes",
            // In a comment or a string that is not sent.
            "// session.send(b\"\\x1b[A\");\nlet x = \"\\x1b[A\";",
        ] {
            assert!(found(passes).is_empty(), "{passes}: {:?}", found(passes));
        }
        assert_eq!(found("// raw key:\nsession.send(b\"\\x1b[A\");").len(), 1);
    }

    /// The sequences, one by one.
    #[test]
    fn each_sequence_is_told_key_or_not() {
        assert_eq!(keys_in(b"\x1b\x1b"), ["`ESC`", "`ESC`"]);
        assert_eq!(keys_in(b"\x1b[15;5~"), ["`ESC[15;5~`"]);
        assert_eq!(keys_in(b"\x1b[1;5R"), ["`ESC[1;5R`"]);
        assert_eq!(
            keys_in(b"\x1b[38;72;0;1;256;1_"),
            ["`ESC[38;72;0;1;256;1_`"]
        );
        assert!(
            keys_in(b"\x1b[2J\x1b[H").len() == 1,
            "home is a key, erase is not"
        );
    }
}
