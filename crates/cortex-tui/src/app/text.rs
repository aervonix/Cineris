//! Text from outside, as the application shows it
//! (`docs/outstanding/names-a-client-shows-as-they-are.md`, closed in step
//! 23).
//!
//! Every text the core sends (a model's answer, a tool's output, a notice,
//! an error, an approval's detail, a path) passes [`shown`] before a view
//! draws it. The views pass it through `cortex_term::sanitize` in turn,
//! which draws each control character as a visible symbol one column wide.
//! [`shown`] covers what that leaves: the characters that act unseen and
//! are not controls (the bidirectional controls, the characters of no
//! width, the tag characters that shadow ASCII, the line and paragraph
//! separators), each written as `\u{…}` by `cortex_base::visible`'s rule.
//! The controls are left to the renderer, so a tool's coloured output shows
//! `␛[31m` and not a longer escape, and a carriage return in it still takes
//! its line back to the start, as a terminal would.
//!
//! The zero-width joiner is kept: it joins an emoji sequence, and it can
//! neither reorder text nor hide any.

use std::borrow::Cow;

/// The characters [`shown`] leaves as they are: the controls, which the
/// renderer draws as symbols, and the zero-width joiner.
const KEEP: [char; 66] = {
    let mut keep = ['\u{200d}'; 66];
    let mut code = 0u8;
    while code < 0x20 {
        keep[code as usize] = code as char;
        code += 1;
    }
    keep[0x20] = '\u{7f}';
    let mut c1 = 0x80u32;
    while c1 < 0xa0 {
        // `char::from_u32` is not `const`; every value here is a scalar.
        keep[(c1 - 0x80 + 0x21) as usize] = match char::from_u32(c1) {
            Some(c) => c,
            None => '\u{200d}',
        };
        c1 += 1;
    }
    keep
};

/// `text` as the application shows it: every character that acts unseen
/// and is not a control written as an escape. See the [module
/// documentation](self).
#[must_use]
pub fn shown(text: &str) -> Cow<'_, str> {
    cortex_base::visible_keeping(text, &KEEP)
}

/// A one-line field from outside (a path, a model's name, a tool's name):
/// [`shown`], with line breaks and tabs as spaces, so that it cannot start
/// a line of its own.
#[must_use]
pub fn one_line(text: &str) -> String {
    shown(text).replace(['\n', '\r', '\t'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_left_to_the_renderer_and_the_rest_is_escaped() {
        assert_eq!(shown("plain"), "plain");
        // Controls stay, for `sanitize` to draw as symbols.
        assert_eq!(shown("a\u{1b}[2Kb\r\n\t"), "a\u{1b}[2Kb\r\n\t");
        assert_eq!(shown("x\u{7f}\u{9b}y"), "x\u{7f}\u{9b}y");
        // What acts unseen and is not a control is an escape.
        assert_eq!(shown("a\u{202e}b"), "a\\u{202e}b");
        assert_eq!(shown("tag\u{e0041}"), "tag\\u{e0041}");
        assert_eq!(shown("zero\u{200b}width"), "zero\\u{200b}width");
        assert_eq!(shown("line\u{2028}sep"), "line\\u{2028}sep");
        // An emoji sequence keeps its joiner and its selector.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(shown(family), family);
        assert_eq!(shown("\u{2764}\u{fe0f}"), "\u{2764}\u{fe0f}");
    }

    #[test]
    fn every_control_is_kept_and_nothing_else_is() {
        for code in (0..0x20).chain([0x7f]).chain(0x80..0xa0) {
            let c = char::from_u32(code).unwrap_or(' ');
            assert!(KEEP.contains(&c), "{code:#x} is not kept");
        }
        assert_eq!(
            KEEP.iter().filter(|c| !c.is_control()).count(),
            1,
            "only the joiner beside the controls"
        );
    }

    #[test]
    fn a_one_line_field_cannot_break_its_line() {
        assert_eq!(one_line("a\nb\tc\rd"), "a b c d");
        assert_eq!(one_line("x\u{202e}y"), "x\\u{202e}y");
    }
}
