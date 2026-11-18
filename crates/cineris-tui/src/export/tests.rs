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
/// is serialised, the title with them, and counts the masks the export
/// holds: a private key, a web address's password and a token in a tool's
/// output reach neither format.
#[test]
fn the_clients_pass_masks_every_string_and_the_title_in_both_formats() {
    let export = Export::new(
        "s_4f2a9c1e77",
        Some(&format!("debugging {TOKEN}")),
        session(),
        false,
    );
    // The private key, the password, the token and the title's token.
    assert_eq!(export.masked, 4);
    for format in [Format::Markdown, Format::Json] {
        let (text, masked) = export.text(format);
        assert_eq!(masked, 4, "{format:?}");
        for secret in ["MIIEvQIBADANBgkqhkiG9w0BAQEFAASC", "hunter2", TOKEN] {
            assert!(!text.contains(secret), "{secret} in {format:?}: {text}");
        }
        assert!(text.contains("[redacted]"), "{text}");
    }
    let json: Value = serde_json::from_str(&export.json()).unwrap();
    assert_eq!(json["masked"], 4);
    assert_eq!(json["items"].as_array().unwrap().len(), 4);
}

/// A web address whose password the core masked goes through the
/// client's pass as it is, and the export counts its one mask once: the
/// filter does not hand it back as a change (the full review of step 25's
/// second pull request).
#[test]
fn an_address_the_core_masked_is_counted_once() {
    let items = vec![item(json!({"kind": "user_message", "id": "i_1", "parts": [
        {"kind": "text", "text": "proxy http://alice:[redacted]@proxy.example:3128/"}
    ]}))];
    let export = Export::new("s_1", None, items, false);
    assert_eq!(export.masked, 1);
    for format in [Format::Markdown, Format::Json] {
        let (text, masked) = export.text(format);
        assert_eq!(masked, 1, "{format:?}");
        assert!(
            text.contains("http://alice:[redacted]@proxy.example:3128/"),
            "{text}"
        );
    }
}

/// The client's pass drops the provider's blocks again, whatever the core
/// did: a `raw` field outside the JSON that is somebody else's, and every
/// provider block part, so a core that kept them still exports none. A key
/// named `raw` in a tool call's arguments is the model's, and is kept,
/// masked.
#[test]
fn the_clients_pass_drops_every_provider_block() {
    let items = vec![item(
        json!({"kind": "assistant_message", "id": "i_1", "parts": [
        {"kind": "reasoning", "text": "Thinking.", "raw": r#"{"signature":"sig-1"}"#},
        {"kind": "text", "text": "Done.", "raw": r#"{"type":"text","citations":"c-1"}"#},
        {"kind": "provider_block", "block_type": "server_tool_use", "raw": r#"{"input":"p-1"}"#},
        {"kind": "tool_call", "id": "c1", "name": "shell",
         "arguments": {"raw": format!("echo {TOKEN}")}, "raw": r#"{"id":"x-1"}"#}
    ], "stop_reason": "end"}),
    )];
    let export = Export::new("s_1", None, items, false);
    assert_eq!(export.items.len(), 1, "the message is kept");
    let json = export.json();
    for block in ["sig-1", "c-1", "p-1", "x-1", "provider_block", TOKEN] {
        assert!(!json.contains(block), "{block} in {json}");
    }
    let value: Value = serde_json::from_str(&json).unwrap();
    let parts = value["items"][0]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 3, "{json}");
    assert_eq!(parts[2]["arguments"]["raw"], json!("echo [redacted]"));
    assert_eq!(export.masked, 1);
}

/// A key of an object that holds a secret is renamed to what the filter
/// makes of it, with a number after it when that name is taken, and each
/// is counted: a tool's arguments keyed by a token reach neither format.
#[test]
fn a_key_that_holds_a_secret_is_renamed_and_counted() {
    let other = "ghp_z9Y8x7W6v5U4t3S2r1Q0p9O8n7M6l5K4j3I2";
    let items = vec![item(
        json!({"kind": "tool_result", "id": "i_1", "call_id": "c1",
        "tool": "env", "outcome": "success", "content": [],
        "payload": {TOKEN: "first", other: "second", "plain": "third"}}),
    )];
    let export = Export::new("s_1", None, items, false);
    assert_eq!(export.masked, 2);
    let (json, masked) = export.text(Format::Json);
    assert_eq!(masked, 2);
    for secret in [TOKEN, other] {
        assert!(!json.contains(secret), "{secret} in {json}");
    }
    let value: Value = serde_json::from_str(&json).unwrap();
    let payload = value["items"][0]["payload"].as_object().unwrap();
    let mut values: Vec<&str> = payload.values().filter_map(Value::as_str).collect();
    values.sort_unstable();
    assert_eq!(values, ["first", "second", "third"], "nothing is lost");
    assert!(payload.contains_key("[redacted]"), "{json}");
    assert!(payload.contains_key("[redacted] 2"), "{json}");
    assert!(payload.contains_key("plain"), "{json}");
}

/// A key split by a control character, or across two parts of a message,
/// is whole only in the Markdown, which leaves the character out and joins
/// the parts: the text as shown goes through the filter once more, and
/// each mask it adds is counted, not those the text held already.
#[test]
fn a_key_that_only_the_markdown_joins_is_masked_there() {
    let (first, second) = TOKEN.split_at(20);
    let items = vec![
        item(json!({"kind": "user_message", "id": "i_1", "parts": [
            {"kind": "text", "text": format!("split {first}\r{second}, and [redacted]")}
        ]})),
        item(json!({"kind": "user_message", "id": "i_2", "parts": [
            {"kind": "text", "text": format!("parted {first}")},
            {"kind": "text", "text": format!("{second} end")}
        ]})),
        item(
            json!({"kind": "tool_result", "id": "i_3", "call_id": "c", "tool": "read",
            "outcome": "success", "content": [{"kind": "text", "text": format!("{first}\u{0}{second}")}]}),
        ),
    ];
    let export = Export::new("s_1", None, items, false);
    assert_eq!(export.masked, 1, "no string holds the key whole");
    let (text, masked) = export.text(Format::Markdown);
    assert!(!text.contains(TOKEN), "{text}");
    assert!(text.contains("split [redacted], and [redacted]"), "{text}");
    assert!(text.contains("parted [redacted] end"), "{text}");
    assert_eq!(masked, 4);
    assert!(text.contains("4 values were masked"), "{text}");
    // JSON keeps the control character, escaped: nothing to join there.
    assert_eq!(export.text(Format::Json).1, 1);
}

/// A tool's name is put on one line, as the title is: a line break in it
/// cannot start a heading or end the line that names it.
#[test]
fn a_tools_name_stays_on_its_line() {
    let items = vec![
        item(json!({"kind": "assistant_message", "id": "i_1", "parts": [
            {"kind": "tool_call", "id": "c1", "name": "shell\n# Injected", "arguments": {}}
        ], "stop_reason": "tool_use"})),
        item(json!({"kind": "tool_result", "id": "i_2", "call_id": "c1",
            "tool": "shell\n## Also", "outcome": "success", "content": []})),
    ];
    let text = Export::new("s_1", None, items, false).markdown();
    assert!(text.contains("Tool call `shell # Injected`:"), "{text}");
    assert!(text.contains("Result of `shell ## Also`, done:"), "{text}");
    for line in text.lines() {
        assert!(
            !line.starts_with("# Injected") && !line.starts_with("## Also"),
            "{line:?} in {text}"
        );
    }
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
    let text = Export::new("s_4f2a9c1e77", None, items, true).markdown();
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
    let text = Export::new("s_1", Some(&format!("{hostile}\n# second")), items, false).markdown();
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
