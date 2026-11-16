//! JSON as the binary prints it: every `--json` output of a subcommand,
//! and headless mode's `json` and `jsonl` (since the follow-ups of step 23,
//! whose open item found three outputs made "visible" with the terminal's
//! `\u{202e}`, which is not JSON).
//!
//! A line is JSON and safe at a terminal at once: JSON's own escaping
//! passes the C1 controls, the bidirectional controls and the tag
//! characters, which a title, a path or a model's name can hold, so each is
//! written as JSON's `\u` escape ([`line`]). A parser reads back exactly
//! the text that went in.

use std::fmt::Write as _;

/// A line of JSON made safe at a terminal, and still JSON: every character
/// that acts unseen (a C1 control, a bidirectional control, a tag
/// character: JSON's own escaping passes them) is written as a `\u`
/// escape, in two halves above the basic plane. Such a character can only
/// be inside a string of the JSON, where the escape means the same.
pub(crate) fn line(json: &str) -> String {
    let mut line = String::with_capacity(json.len());
    for character in json.chars() {
        if cortex_base::acts_unseen(character) {
            let mut halves = [0_u16; 2];
            for half in character.encode_utf16(&mut halves) {
                let _ = write!(line, "\\u{half:04x}");
            }
        } else {
            line.push(character);
        }
    }
    line
}

/// A value serialised (`serde_json::to_string(&value)`) as one [`line`] of
/// JSON, with its end. A value of the types printed always serialises;
/// should one not, the line says so as JSON rather than as nothing.
pub(crate) fn record(json: Result<String, serde_json::Error>) -> String {
    let json = json.unwrap_or_else(|error| {
        serde_json::json!({"type": "error", "message": error.to_string()}).to_string()
    });
    let mut text = line(&json);
    text.push('\n');
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Checks that `printed` is lines of JSON with nothing in them that
    /// acts unseen, and gives back what each line parses to.
    pub(crate) fn parsed(printed: &str) -> Vec<serde_json::Value> {
        assert!(printed.ends_with('\n'), "{printed:?}");
        assert!(
            !printed
                .chars()
                .any(|character| character != '\n' && cortex_base::acts_unseen(character)),
            "{:?}",
            printed
                .chars()
                .filter(|character| *character != '\n' && cortex_base::acts_unseen(*character))
                .collect::<String>()
        );
        printed
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|error| panic!("not JSON ({error}): {line:?}"))
            })
            .collect()
    }

    /// One line of JSON, still JSON, with nothing in it that acts unseen
    /// at a terminal, for every hostile name: what is seen stays as it is.
    #[test]
    fn a_record_is_json_and_shows_what_acts_unseen() {
        for name in cortex_testkit::hostile::all() {
            let text = format!("a{}b é 日本 😀 \"q\" \\", name.text);
            let printed = record(serde_json::to_string(&serde_json::json!({"title": text})));
            assert_eq!(printed.matches('\n').count(), 1, "{}", name.label);
            let back = parsed(&printed);
            assert_eq!(back[0]["title"], text.as_str(), "{}", name.label);
            assert!(printed.contains("é 日本 😀"), "{printed}");
        }
        let printed = record(serde_json::to_string("\u{202e}\u{e0041}"));
        assert!(printed.contains("\\u202e\\udb40\\udc41"), "{printed}");
    }
}
