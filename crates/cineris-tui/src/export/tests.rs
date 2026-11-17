use serde_json::json;

use super::*;

/// A private key, made up.
const PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\n-----END PRIVATE KEY-----";
/// A token in a well-known format, made up.
const TOKEN: &str = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";

fn item(value: Value) -> Item {
    serde_json::from_value(value).expect("an item")
}

/// A session as the core gave it for an export, with what the core's pass
/// could have missed left in: its stricter formats, the client's, are
/// what this module adds.
fn session() -> Vec<Item> {
    vec![
        item(json!({"kind": "user_message", "id": "i_1", "parts": [
            {"kind": "text", "text": "Why does the build fail?"},
            {"kind": "image", "media_type": "image/png", "artifact": "a_1"}
        ]})),
        item(json!({"kind": "assistant_message", "id": "i_2", "parts": [
            {"kind": "reasoning", "text": "Look at the manifest.\nThen the lock file."},
            {"kind": "text", "text": "Reading the key file."},
            {"kind": "tool_call", "id": "c1", "name": "shell", "arguments": {"command": "cat key.pem"}}
        ]})),
        item(
            json!({"kind": "tool_result", "id": "i_3", "call_id": "c1", "tool": "shell",
            "outcome": "success", "content": [
                {"kind": "text", "text": format!("{PEM}\nproxy http://alice:hunter2@proxy.example:3128/\ntoken {TOKEN}\n")}
            ]}),
        ),
        item(json!({"kind": "notice", "id": "i_4", "source": "harness", "text": "Compacted."})),
    ]
}

#[test]
fn the_argument_names_the_format_or_a_file_by_its_ending() {
    assert_eq!(Format::from_argument(""), Ok((Format::Markdown, None)));
    assert_eq!(Format::from_argument(" json "), Ok((Format::Json, None)));
    assert_eq!(
        Format::from_argument("notes/out.JSON"),
        Ok((Format::Json, Some("notes/out.JSON".to_owned())))
    );
    assert_eq!(
        Format::from_argument("out.md"),
        Ok((Format::Markdown, Some("out.md".to_owned())))
    );
    assert!(Format::from_argument("out.txt").is_err());
}

/// The client's stricter pass masks every string of every item before it
/// is serialised, the title with them, and counts what it masked on top of
/// the core's count: a private key, a web address's password and a token
/// in a tool's output reach neither format.
#[test]
fn the_clients_pass_masks_every_string_and_the_title_in_both_formats() {
    let export = Export::new(
        "s_4f2a9c1e77",
        Some(&format!("debugging {TOKEN}")),
        session(),
        2,
        false,
    );
    // The core's 2, and the private key, the password, the token and the
    // title's token.
    assert_eq!(export.masked, 6);
    for format in [Format::Markdown, Format::Json] {
        let text = export.text(format);
        for secret in ["MIIEvQIBADANBgkqhkiG9w0BAQEFAASC", "hunter2", TOKEN] {
            assert!(!text.contains(secret), "{secret} in {format:?}: {text}");
        }
        assert!(text.contains("[redacted]"), "{text}");
    }
    let json: Value = serde_json::from_str(&export.json()).unwrap();
    assert_eq!(json["masked"], 6);
    assert_eq!(json["items"].as_array().unwrap().len(), 4);
}

/// The Markdown says what the session is, how many values are masked and
/// whether a turn was running; each prompt, answer, call, result and
/// notice in order; output fenced longer than any run of backticks in it.
#[test]
fn markdown_lays_the_session_out_in_order_with_fences_nothing_closes() {
    let mut items = session();
    items.push(item(
        json!({"kind": "tool_result", "id": "i_5", "call_id": "c2",
        "tool": "read", "outcome": "error", "content": [
            {"kind": "text", "text": "````\nnot closed\n```"}
        ]}),
    ));
    let text = Export::new("s_4f2a9c1e77", None, items, 0, true).markdown();
    // The heading escapes what Markdown would read as emphasis.
    assert!(text.starts_with("# Session s\\_4f2a9c1e77\n\n"), "{text}");
    assert!(
        text.contains("3 values were masked. A turn was running"),
        "{text}"
    );
    let order = [
        "## You",
        "Why does the build fail?[an image]",
        "## Answer",
        "> Look at the manifest.\n> Then the lock file.",
        "Tool call `shell`:",
        "Result of `shell`, done:",
        "> Compacted.",
        "Result of `read`, failed:\n\n`````text\n````\nnot closed\n```\n`````",
    ];
    let mut at = 0;
    for part in order {
        let found = text[at..]
            .find(part)
            .unwrap_or_else(|| panic!("{part:?} after {at} in {text}"));
        at += found + part.len();
    }
}

/// Hostile text in a title, a prompt, a tool's name and its output: in the
/// Markdown, no control character is left and nothing acts unseen; the
/// title cannot become something other than the heading.
#[test]
fn markdown_holds_no_control_character_and_nothing_that_acts_unseen() {
    let hostile =
        "a\u{1b}[31mred\u{1b}[0m \u{202e}evil\u{202c} \u{9b}2J\r tag\u{e0041} `x` # [l](u)";
    let items = vec![
        item(
            json!({"kind": "user_message", "id": "i_1", "parts": [{"kind": "text", "text": hostile}]}),
        ),
        item(
            json!({"kind": "tool_result", "id": "i_2", "call_id": "c", "tool": hostile,
            "outcome": "success", "content": [{"kind": "text", "text": hostile}]}),
        ),
    ];
    let text = Export::new(
        "s_1",
        Some(&format!("{hostile}\n# second")),
        items,
        0,
        false,
    )
    .markdown();
    for c in text.chars() {
        assert!(
            !c.is_control() || matches!(c, '\n' | '\t'),
            "{c:?} in {text:?}"
        );
    }
    assert_eq!(
        crate::testing::unseen(&text),
        Vec::<char>::new(),
        "{text:?}"
    );
    let heading = text.lines().next().unwrap();
    assert!(heading.starts_with("# a"), "{heading}");
    assert!(
        heading.contains("\\# second"),
        "the title stays one heading: {heading}"
    );
}
