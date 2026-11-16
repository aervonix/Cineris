//! A tool call as its line in a round's box: the verb and target from the
//! tool's name and arguments, the state, measure and detail from its result
//! (`docs/architecture/09-tui.md`, "Tool rounds").
//!
//! The payload's shape belongs to each tool (`cortex-tools`); what is read
//! here is the part a line shows. A field a tool does not send, or a tool
//! this build does not know, leaves the line with its verb and its state.
//!
//! Everything is kept as it came: the views pass it through
//! `cortex_term::sanitize` when they draw it.

use std::time::Duration;

use cortex_protocol::item::{ToolOutcome, ToolResultItem};
use cortex_protocol::message::Part;
use serde_json::Value;

use crate::view::{Call, CallState, Change, Detail, DiffLine, Measure, Target};

/// The call's line before its arguments are known: the verb from the
/// tool's name, running.
#[must_use]
pub(crate) fn started(tool: &str) -> Call {
    Call {
        verb: verb(tool),
        target: Target::None,
        state: CallState::Running(Duration::ZERO),
        measure: Measure::None,
        took: None,
        detail: Detail::None,
    }
}

/// The tool as a bold imperative.
fn verb(tool: &str) -> String {
    match tool {
        "read" => "Read",
        "edit" => "Edit",
        "write" => "Write",
        "shell" => "Run",
        "search" => "Search",
        "list" => "List",
        "outline" => "Outline",
        "plan" => "Plan",
        "remember" => "Remember",
        "tool_search" => "Find tools",
        "tool_call" => "Call",
        other => return other.to_owned(),
    }
    .to_owned()
}

/// What the call acts on, from its arguments once they are whole.
#[must_use]
pub(crate) fn target(tool: &str, arguments: &Value) -> Target {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    match tool {
        "read" | "edit" | "write" | "list" | "outline" => match text("path") {
            Some(path) => Target::Path(path.to_owned()),
            None => Target::None,
        },
        "search" => match text("pattern") {
            Some(pattern) => Target::Text(pattern.to_owned()),
            None => Target::None,
        },
        // The box's line shows a command on one line; the whole of it,
        // line breaks and all, is in the viewer and the approval's detail.
        "shell" => match text("command") {
            Some(command) => Target::Text(command.to_owned()),
            None => Target::None,
        },
        "tool_search" => match text("query") {
            Some(query) => Target::Text(query.to_owned()),
            None => Target::None,
        },
        "tool_call" => match text("tool") {
            Some(name) => Target::Text(name.to_owned()),
            None => Target::None,
        },
        _ => Target::None,
    }
}

/// The call once its result has come: its state, measure, time and detail.
/// `output` is what streamed while it ran.
pub(crate) fn finish(call: &mut Call, result: &ToolResultItem, output: &str) {
    call.state = match result.outcome {
        ToolOutcome::Success => CallState::Succeeded,
        ToolOutcome::Denied => CallState::Denied,
        ToolOutcome::Interrupted => CallState::Interrupted,
        _ => CallState::Failed,
    };
    call.took = result.duration_ms.map(Duration::from_millis);
    let payload = result.payload.as_ref().unwrap_or(&Value::Null);
    let number = |key: &str| payload.get(key).and_then(Value::as_u64);
    let failed = call.state == CallState::Failed;
    call.measure = match result.tool.as_str() {
        "read" => match payload.get("kind").and_then(Value::as_str) {
            Some("image") => Measure::Note("image".to_owned()),
            _ => match (number("first_line"), number("last_line")) {
                (Some(first), Some(last)) => Measure::Lines { first, last },
                _ => Measure::None,
            },
        },
        "edit" | "write" => match payload.get("kind").and_then(Value::as_str) {
            Some("unchanged") => Measure::Note("unchanged".to_owned()),
            _ => match (number("added"), number("removed")) {
                (Some(added), Some(removed)) => Measure::Changes { added, removed },
                _ => Measure::None,
            },
        },
        "shell" => shell_measure(payload, failed),
        "search" => match number("matches") {
            Some(0) => Measure::Note("no matches".to_owned()),
            Some(1) => Measure::Note("1 match".to_owned()),
            Some(matches) => Measure::Note(format!("{matches} matches")),
            None => Measure::None,
        },
        "list" => match number("entries") {
            Some(1) => Measure::Note("1 entry".to_owned()),
            Some(entries) => Measure::Note(format!("{entries} entries")),
            None => Measure::None,
        },
        _ => Measure::None,
    };
    call.detail = match result.tool.as_str() {
        "edit" | "write" => payload
            .get("diff")
            .and_then(Value::as_str)
            .map(|diff| Detail::Diff(diff_lines(diff)))
            .unwrap_or(Detail::None),
        "shell" => {
            let text = if output.is_empty() {
                result_text(result)
            } else {
                output.to_owned()
            };
            if text.is_empty() {
                Detail::None
            } else {
                Detail::Output(text)
            }
        }
        // A failure says why under its line.
        _ if failed => {
            let text = result_text(result);
            if text.is_empty() {
                Detail::None
            } else {
                Detail::Output(text)
            }
        }
        _ => Detail::None,
    };
}

/// A command's measure: how it ended, when not well.
fn shell_measure(payload: &Value, failed: bool) -> Measure {
    if payload.get("timed_out").and_then(Value::as_bool) == Some(true) {
        return Measure::Note("timed out".to_owned());
    }
    if let Some(signal) = payload.get("signal").and_then(Value::as_i64) {
        return Measure::Note(format!("signal {signal}"));
    }
    match payload.get("exit_code").and_then(Value::as_i64) {
        Some(code) if failed || code != 0 => Measure::Note(format!("exit {code}")),
        _ => Measure::None,
    }
}

/// The text the model was shown, as one string.
fn result_text(result: &ToolResultItem) -> String {
    result
        .content
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A unified diff's lines, numbered as the gutter shows them: a removed
/// line by the old file's number, any other by the new one's. Of a removed
/// line and the added line after it, the whole words that changed are
/// emphasised (see [`emphasise`]).
#[must_use]
pub(crate) fn diff_lines(diff: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let (mut old, mut new) = (0u64, 0u64);
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("@@") {
            let (from, to) = hunk_starts(header);
            old = from;
            new = to;
            continue;
        }
        let (change, text) = match line.as_bytes().first() {
            Some(b'-') => (Change::Removed, &line[1..]),
            Some(b'+') => (Change::Added, &line[1..]),
            Some(b' ') => (Change::Same, &line[1..]),
            // A line of no sign (`\ No newline at end of file`) is not a
            // line of either file.
            _ => continue,
        };
        let number = match change {
            Change::Removed => {
                old += 1;
                old - 1
            }
            Change::Added => {
                new += 1;
                new - 1
            }
            Change::Same => {
                old += 1;
                new += 1;
                new - 1
            }
        };
        lines.push(DiffLine {
            number,
            change,
            text: text.to_owned(),
            emphasis: Vec::new(),
        });
    }
    emphasise(&mut lines);
    lines
}

/// The two starts of a hunk's header, ` -a,b +c,d @@`.
fn hunk_starts(header: &str) -> (u64, u64) {
    let start = |sign: char| {
        header
            .split_whitespace()
            .find_map(|part| part.strip_prefix(sign))
            .and_then(|range| range.split(',').next())
            .and_then(|first| first.parse::<u64>().ok())
            .unwrap_or(1)
    };
    (start('-'), start('+'))
}

/// Marks what changed in each removed line followed at once by an added
/// one: the middle between their common start and common end.
/// Marks what changed between a removed line and the added line after it
/// (09-tui, "Diff": changed words are bold; the owner, 2026-11-16). Each
/// line is cut into tokens: a word (a run of letters, digits and `_`, so
/// that `foo_bar` is one), a run of spaces, or any other single character.
/// The tokens the two lines do not share, in order, are the change; of
/// those, the words are emphasised, whole, and the punctuation and spaces
/// around them stay plain. A line whose change holds no word (a comma
/// added) has its changed punctuation emphasised, so that the change still
/// shows; a change of spaces alone has nothing to emphasise, and is shown
/// by the line's sign and tint.
fn emphasise(lines: &mut [DiffLine]) {
    for at in 1..lines.len() {
        if lines[at - 1].change != Change::Removed || lines[at].change != Change::Added {
            continue;
        }
        if at >= 2 && lines[at - 2].change == Change::Removed {
            // Several lines changed together: no one pairing is right.
            continue;
        }
        let removed = tokens(&lines[at - 1].text);
        let added = tokens(&lines[at].text);
        let (removed_kept, added_kept) =
            shared(&lines[at - 1].text, &removed, &lines[at].text, &added);
        lines[at - 1].emphasis = changed(&removed, &removed_kept);
        lines[at].emphasis = changed(&added, &added_kept);
    }
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    /// Letters, digits and `_`.
    Word,
    /// A run of spaces.
    Space,
    /// Any other single character.
    Mark,
}

/// `text`'s tokens, with their byte ranges.
fn tokens(text: &str) -> Vec<(std::ops::Range<usize>, Piece)> {
    let kind = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Piece::Word
        } else if c.is_whitespace() {
            Piece::Space
        } else {
            Piece::Mark
        }
    };
    let mut out: Vec<(std::ops::Range<usize>, Piece)> = Vec::new();
    for (at, c) in text.char_indices() {
        let piece = kind(c);
        let end = at + c.len_utf8();
        match out.last_mut() {
            Some((range, last)) if *last == piece && piece != Piece::Mark => range.end = end,
            _ => out.push((at..end, piece)),
        }
    }
    out
}

/// The most tokens two lines are compared by, the product of their counts:
/// past it (lines of thousands of tokens) the shared tokens are the common
/// start and end alone.
const COMPARED: usize = 250_000;

/// Which tokens of each line the other shares, in order (their longest
/// common run of tokens, by text).
fn shared(
    a_text: &str,
    a: &[(std::ops::Range<usize>, Piece)],
    b_text: &str,
    b: &[(std::ops::Range<usize>, Piece)],
) -> (Vec<bool>, Vec<bool>) {
    let same = |i: usize, j: usize| a_text[a[i].0.clone()] == b_text[b[j].0.clone()];
    let mut a_kept = vec![false; a.len()];
    let mut b_kept = vec![false; b.len()];
    if a.len().saturating_mul(b.len()) > COMPARED {
        let start = (0..a.len().min(b.len()))
            .take_while(|&i| same(i, i))
            .count();
        let end = (0..a.len().min(b.len()) - start)
            .take_while(|&k| same(a.len() - 1 - k, b.len() - 1 - k))
            .count();
        for i in (0..start).chain(a.len() - end..a.len()) {
            a_kept[i] = true;
        }
        for j in (0..start).chain(b.len() - end..b.len()) {
            b_kept[j] = true;
        }
        return (a_kept, b_kept);
    }
    // lengths[i][j]: the longest common run of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut lengths = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lengths[i * width + j] = if same(i, j) {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if same(i, j) {
            a_kept[i] = true;
            b_kept[j] = true;
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (a_kept, b_kept)
}

/// The ranges to emphasise: the changed words, or, when the change holds
/// none, its changed punctuation.
fn changed(
    tokens: &[(std::ops::Range<usize>, Piece)],
    kept: &[bool],
) -> Vec<std::ops::Range<usize>> {
    let of = |wanted: Piece| -> Vec<std::ops::Range<usize>> {
        tokens
            .iter()
            .zip(kept)
            .filter(|((_, piece), kept)| !**kept && *piece == wanted)
            .map(|((range, _), _)| range.clone())
            .collect()
    };
    let words = of(Piece::Word);
    if words.is_empty() {
        of(Piece::Mark)
    } else {
        words
    }
}

#[cfg(test)]
mod tests {
    use cortex_protocol::{CallId, ItemId};
    use serde_json::json;

    use super::*;

    fn result(tool: &str, outcome: ToolOutcome, payload: Value) -> ToolResultItem {
        ToolResultItem {
            id: ItemId::new("i_9"),
            call_id: CallId::new("c_1"),
            tool: tool.to_owned(),
            outcome,
            content: vec![Part::text("the text the model saw")],
            payload: Some(payload),
            artifacts: Vec::new(),
            duration_ms: Some(3400),
            created_at: None,
        }
    }

    #[test]
    fn each_tool_names_its_verb_and_target() {
        assert_eq!(started("shell").verb, "Run");
        assert_eq!(started("read").verb, "Read");
        assert_eq!(started("my_ext_tool").verb, "my_ext_tool");
        assert_eq!(
            target("edit", &json!({"path": "src/lex.rs"})),
            Target::Path("src/lex.rs".to_owned())
        );
        // As it came: the view draws it on one line, sanitised.
        assert_eq!(
            target("shell", &json!({"command": "cargo test\n -p parser"})),
            Target::Text("cargo test\n -p parser".to_owned())
        );
        assert_eq!(target("plan", &json!({"items": []})), Target::None);
    }

    #[test]
    fn a_result_gives_its_state_measure_and_detail() {
        let mut call = started("read");
        finish(
            &mut call,
            &result(
                "read",
                ToolOutcome::Success,
                json!({"first_line": 120, "last_line": 178}),
            ),
            "",
        );
        assert_eq!(call.state, CallState::Succeeded);
        assert_eq!(
            call.measure,
            Measure::Lines {
                first: 120,
                last: 178
            }
        );
        assert_eq!(call.took, Some(Duration::from_millis(3400)));

        let mut call = started("shell");
        finish(
            &mut call,
            &result(
                "shell",
                ToolOutcome::Error,
                json!({"exit_code": 101, "timed_out": false}),
            ),
            "test a ... FAILED\n",
        );
        assert_eq!(call.state, CallState::Failed);
        assert_eq!(call.measure, Measure::Note("exit 101".to_owned()));
        assert_eq!(
            call.detail,
            Detail::Output("test a ... FAILED\n".to_owned())
        );

        let mut call = started("edit");
        finish(
            &mut call,
            &result(
                "edit",
                ToolOutcome::Success,
                json!({"added": 1, "removed": 1, "diff": "@@ -1,2 +1,2 @@\n a\n-b\n+c"}),
            ),
            "",
        );
        assert_eq!(
            call.measure,
            Measure::Changes {
                added: 1,
                removed: 1
            }
        );
        let Detail::Diff(lines) = &call.detail else {
            panic!("a diff");
        };
        assert_eq!(lines.len(), 3);

        let mut call = started("edit");
        finish(
            &mut call,
            &result("edit", ToolOutcome::Denied, json!({})),
            "",
        );
        assert_eq!(call.state, CallState::Denied);
    }

    #[test]
    fn a_diff_is_numbered_as_the_gutter_shows_it() {
        let lines = diff_lines(
            "@@ -141,3 +141,4 @@\n     while x {\n-        if a { break; }\n+        if b { break; }\n+        go();\n     }",
        );
        let numbers: Vec<(u64, Change)> = lines.iter().map(|l| (l.number, l.change)).collect();
        assert_eq!(
            numbers,
            [
                (141, Change::Same),
                (142, Change::Removed),
                (142, Change::Added),
                (143, Change::Added),
                (144, Change::Same),
            ]
        );
        // The changed middle of a replaced line.
        assert_eq!(
            lines[1].emphasis,
            std::iter::once(11..12).collect::<Vec<_>>()
        );
        assert_eq!(
            lines[2].emphasis,
            std::iter::once(11..12).collect::<Vec<_>>()
        );
    }

    /// The emphasis of a replaced line's pair, as text.
    fn bold(removed: &str, added: &str) -> (Vec<String>, Vec<String>) {
        let lines = diff_lines(&format!("@@ -1 +1 @@\n-{removed}\n+{added}"));
        let of = |line: &DiffLine| {
            line.emphasis
                .iter()
                .map(|range| line.text[range.clone()].to_owned())
                .collect()
        };
        (of(&lines[0]), of(&lines[1]))
    }

    /// The owner's rule (2026-11-16): whole changed words are bold, and the
    /// punctuation and spaces around them stay plain.
    #[test]
    fn whole_changed_words_are_bold_and_what_is_around_them_plain() {
        // Screen 2a's line: a word added after a comma.
        assert_eq!(
            bold(
                "        with: { components: rustfmt }",
                "        with: { components: rustfmt, clippy }"
            ),
            (Vec::new(), vec!["clippy".to_owned()])
        );
        // A change inside a word: the word, whole.
        assert_eq!(
            bold("let foo_bar = 1;", "let foo_baz = 1;"),
            (vec!["foo_bar".to_owned()], vec!["foo_baz".to_owned()])
        );
        // Several words changed: each changed one, and none that stayed.
        assert_eq!(
            bold("let total = a + b;", "let sum = a + c;"),
            (
                vec!["total".to_owned(), "b".to_owned()],
                vec!["sum".to_owned(), "c".to_owned()]
            )
        );
        // A change of punctuation alone still shows: its character is bold.
        assert_eq!(
            bold("call(a b)", "call(a, b)"),
            (Vec::new(), vec![",".to_owned()])
        );
        // A change of spaces alone has nothing to make bold.
        assert_eq!(bold("a  b", "a b"), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_hunk_header_without_counts_still_numbers_its_lines() {
        let lines = diff_lines("@@ -7 +9 @@\n-x\n+y\n\\ No newline at end of file");
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].number, lines[1].number), (7, 9));
    }
}
