//! Text from models, tools and files made safe to show
//! (`docs/architecture/09-tui.md`, "Sanitising").
//!
//! Only the renderer emits control sequences. Every control character in
//! content is shown as a visible symbol instead, so content can never move
//! the cursor, change a mode, set a title, write a link, or ask the terminal
//! anything. Two layers make sure of it:
//!
//! - [`sanitize`], the one function content passes through before a
//!   component lays it out. It keeps line breaks and tabs, which layout
//!   needs, and makes every other control character visible.
//! - [`visible`], which the renderer applies to every character it writes
//!   from a [`Line`](crate::Line), line breaks and tabs included. A component
//!   that forgot the first layer still cannot send an escape sequence.
//!
//! The control characters are C0 (`U+0000` to `U+001F`), DEL (`U+007F`) and
//! C1 (`U+0080` to `U+009F`). C1 matters as much as ESC: some terminals take
//! `U+009B` as the start of a control sequence, exactly like `ESC [`.
//!
//! **What else acts unseen** (since step 23, which closed
//! the open item on names a client shows as they are): [`sanitize`]
//! also writes as an escape, `\u{…}`, every character that is not a control
//! and would change how the text around it reads without being seen, by
//! `cortex_base::visible`'s rule: the bidirectional controls, which can
//! reorder a line so that it reads as something else, the characters of no
//! width, the tag characters, which shadow ASCII unseen, the line and
//! paragraph separators, and the characters that render as nothing. Text is
//! escaped once, here, as it is drawn, and stays exact as data. **The
//! zero-width joiner and non-joiner are kept:** they join an emoji sequence
//! and shape Persian and the Indic scripts, and can neither reorder text
//! nor hide any.

use std::borrow::Cow;

/// What a C1 control character is shown as. They have no picture of their
/// own in Unicode.
const C1_SYMBOL: char = '\u{FFFD}';

/// What the escape of what acts unseen leaves for the second pass: the
/// control characters, which become pictures, and the two joiners.
const KEEP: [char; 67] = {
    let mut keep = ['\u{200d}'; 67];
    let mut code = 0u32;
    while code < 0x20 {
        keep[code as usize] = match char::from_u32(code) {
            Some(c) => c,
            None => '\u{200d}',
        };
        code += 1;
    }
    keep[0x20] = '\u{7f}';
    let mut c1 = 0x80u32;
    while c1 < 0xa0 {
        keep[(c1 - 0x80 + 0x21) as usize] = match char::from_u32(c1) {
            Some(c) => c,
            None => '\u{200d}',
        };
        c1 += 1;
    }
    keep[0x41] = '\u{200c}';
    keep
};

/// The visible stand-in for a control character, or `None` for a character
/// that is not one.
///
/// C0 characters become their Control Pictures (`␛` for ESC, `␇` for BEL),
/// DEL becomes `␡`, and C1 characters become `�`. Each stand-in is one column
/// wide, as the control character is counted
/// ([`cortex_base::text::width`]).
#[must_use]
pub fn visible(c: char) -> Option<char> {
    match c {
        '\0'..='\u{1f}' => char::from_u32(0x2400 + u32::from(c)),
        '\u{7f}' => Some('\u{2421}'),
        '\u{80}'..='\u{9f}' => Some(C1_SYMBOL),
        _ => None,
    }
}

/// `text` with every control character but the line feed and the tab made
/// visible, and every other character that acts unseen written as an
/// escape (see the [module documentation](self)). A carriage return just
/// before a line feed is dropped, so text with Windows line endings is
/// plain lines.
///
/// Text with nothing to change is returned as it is, borrowed.
#[must_use]
pub fn sanitize(text: &str) -> Cow<'_, str> {
    match cortex_base::visible_keeping(text, &KEEP) {
        Cow::Borrowed(text) => pictures(text),
        Cow::Owned(escaped) => Cow::Owned(pictures(&escaped).into_owned()),
    }
}

/// `text` with its control characters as pictures.
fn pictures(text: &str) -> Cow<'_, str> {
    let needs = text
        .chars()
        .any(|c| c != '\n' && c != '\t' && visible(c).is_some());
    if !needs {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\t' => out.push(c),
            '\r' if chars.peek() == Some(&'\n') => {}
            _ => out.push(visible(c).unwrap_or(c)),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_character_but_line_feed_and_tab_is_made_visible() {
        assert_eq!(sanitize("plain text\n\tindented"), "plain text\n\tindented");
        assert!(matches!(sanitize("plain"), Cow::Borrowed(_)));
        assert_eq!(sanitize("a\u{1b}[2Jb"), "a\u{241b}[2Jb");
        assert_eq!(sanitize("bell\u{7}"), "bell\u{2407}");
        assert_eq!(sanitize("del\u{7f}"), "del\u{2421}");
        assert_eq!(sanitize("csi\u{9b}31m"), "csi\u{fffd}31m");
        assert_eq!(sanitize("win\r\nline"), "win\nline");
        assert_eq!(sanitize("bare\rreturn"), "bare\u{240d}return");
        // Nothing that is left could start a sequence.
        // Every control character is below U+00A0; the planes above are
        // sampled on a pull request and all checked in the nightly run.
        let all = std::env::var("CORTEX_PROPERTY_SCALE")
            .ok()
            .and_then(|scale| scale.parse::<u32>().ok())
            .is_some_and(|scale| scale > 1);
        let step = if all { 1 } else { 97 };
        for c in (0..0x3000)
            .chain((0x3000..=0x10ffff).step_by(step))
            .filter_map(char::from_u32)
        {
            let shown = sanitize(&c.to_string()).into_owned();
            assert!(
                shown
                    .chars()
                    .all(|s| s == '\n' || s == '\t' || visible(s).is_none()),
                "{c:?}"
            );
        }
    }

    #[test]
    fn what_acts_unseen_and_is_not_a_control_is_an_escape() {
        // Reordering, hiding and breaking lines unseen.
        assert_eq!(sanitize("a\u{202e}b"), "a\\u{202e}b");
        assert_eq!(sanitize("tag\u{e0041}"), "tag\\u{e0041}");
        assert_eq!(sanitize("zero\u{200b}width"), "zero\\u{200b}width");
        assert_eq!(sanitize("line\u{2028}sep"), "line\\u{2028}sep");
        // Both at once: the escape first, then the pictures.
        assert_eq!(sanitize("\u{1b}\u{2066}x"), "\u{241b}\\u{2066}x");
        // An emoji sequence keeps its joiner and its selector, and a word
        // that needs the non-joiner keeps it.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(sanitize(family), family);
        assert!(matches!(sanitize(family), Cow::Borrowed(_)));
        assert_eq!(sanitize("\u{2764}\u{fe0f}"), "\u{2764}\u{fe0f}");
        assert_eq!(sanitize("\u{645}\u{200c}\u{6cc}"), "\u{645}\u{200c}\u{6cc}");
        // A selector with nothing to select is an escape.
        assert_eq!(sanitize("\u{fe0f}x"), "\\u{fe0f}x");
    }

    #[test]
    fn the_escape_keeps_every_control_and_the_two_joiners_only() {
        for code in (0..0x20).chain([0x7f]).chain(0x80..0xa0) {
            let c = char::from_u32(code).unwrap_or(' ');
            assert!(KEEP.contains(&c), "{code:#x} is not kept");
        }
        let others: Vec<char> = KEEP.iter().copied().filter(|c| !c.is_control()).collect();
        assert_eq!(others, ['\u{200c}', '\u{200d}']);
    }

    #[test]
    fn every_stand_in_is_one_column() {
        for c in (0..=0x9f).filter_map(char::from_u32) {
            if let Some(shown) = visible(c) {
                assert_eq!(cortex_base::text::width(&shown.to_string()), 1, "{c:?}");
                assert_eq!(cortex_base::text::width(&c.to_string()), 1, "{c:?}");
            }
        }
    }
}
