//! A tool call as its line in a round's box: the verb and target from the
//! tool's name and arguments, the state, measure and detail from its result
//! (`docs/architecture/09-tui.md`, "Tool rounds").
//!
//! The payload's shape belongs to each tool (`cortex-tools`); what is read
//! here is the part a line shows. A field a tool does not send, or a tool
//! this build does not know, leaves the line with its verb and its state.

use std::time::Duration;

use cortex_protocol::item::{ToolOutcome, ToolResultItem};
use cortex_protocol::message::Part;
use serde_json::Value;

use super::text::{one_line, shown};
use crate::view::{Call, CallState, Change, Detail, DiffLine, Measure, Target};

/// The call's line before its arguments are known: the verb from the
/// tool's name, running.
#[must_use]
pub fn started(tool: &str) -> Call {
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
        other => return one_line(other),
    }
    .to_owned()
}

/// What the call acts on, from its arguments once they are whole.
#[must_use]
pub fn target(tool: &str, arguments: &Value) -> Target {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    match tool {
        "read" | "edit" | "write" | "list" | "outline" => match text("path") {
            Some(path) => Target::Path(one_line(path)),
            None => Target::None,
        },
        "search" => match text("pattern") {
            Some(pattern) => Target::Text(one_line(pattern)),
            None => Target::None,
        },
        // A command is shown on one line in the box; the whole of it, line
        // breaks and all, is in the viewer and in the approval's detail.
        "shell" => match text("command") {
            Some(command) => Target::Text(one_line(command)),
            None => Target::None,
        },
        "tool_search" => match text("query") {
            Some(query) => Target::Text(one_line(query)),
            None => Target::None,
        },
        "tool_call" => match text("tool") {
            Some(name) => Target::Text(one_line(name)),
            None => Target::None,
        },
        _ => Target::None,
    }
}

/// The call once its result has come: its state, measure, time and detail.
/// `output` is what streamed while it ran.
pub fn finish(call: &mut Call, result: &ToolResultItem, output: &str) {
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
            .map(|diff| Detail::Diff(diff_lines(&shown(diff))))
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
                Detail::Output(shown(&text).into_owned())
            }
        }
        // A failure says why under its line.
        _ if failed => {
            let text = result_text(result);
            if text.is_empty() {
                Detail::None
            } else {
                Detail::Output(shown(&text).into_owned())
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
/// line by the old file's number, any other by the new one's. The changed
/// middle of a removed line and the added line after it is emphasised.
#[must_use]
pub fn diff_lines(diff: &str) -> Vec<DiffLine> {
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
fn emphasise(lines: &mut [DiffLine]) {
    for at in 1..lines.len() {
        if lines[at - 1].change != Change::Removed || lines[at].change != Change::Added {
            continue;
        }
        if at >= 2 && lines[at - 2].change == Change::Removed {
            // Several lines changed together: no one pairing is right.
            continue;
        }
        let (removed, added) = (&lines[at - 1].text, &lines[at].text);
        let prefix = common_prefix(removed, added);
        let suffix = common_suffix(&removed[prefix..], &added[prefix..]);
        let removed_end = removed.len() - suffix;
        let added_end = added.len() - suffix;
        if prefix < removed_end {
            lines[at - 1].emphasis = vec![prefix..removed_end];
        }
        if prefix < added_end {
            lines[at].emphasis = vec![prefix..added_end];
        }
    }
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or_else(|| a.len().min(b.len()), |((at, _), _)| at)
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut length = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        length += x.len_utf8();
    }
    length
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
        assert_eq!(
            target("shell", &json!({"command": "cargo test\n -p parser"})),
            Target::Text("cargo test  -p parser".to_owned())
        );
        assert_eq!(
            target("search", &json!({"pattern": "scan_\u{202e}number"})),
            Target::Text("scan_\\u{202e}number".to_owned())
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
        assert_eq!(call.detail, Detail::Output("test a ... FAILED\n".to_owned()));

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
        assert_eq!(lines[1].emphasis, vec![11..12]);
        assert_eq!(lines[2].emphasis, vec![11..12]);
    }

    #[test]
    fn a_hunk_header_without_counts_still_numbers_its_lines() {
        let lines = diff_lines("@@ -7 +9 @@\n-x\n+y\n\\ No newline at end of file");
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].number, lines[1].number), (7, 9));
    }
}
