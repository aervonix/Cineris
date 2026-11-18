//! The redaction hook (`docs/security.md`, section 4).
//!
//! A [`Redactor`] sees the text of every entry and every text artifact
//! before it is written, and may mask what looks like a secret. This crate
//! provides the hook and a redactor that does nothing; the filter itself
//! (the harness's own credentials by value, and well-known key formats) is
//! `cineris_base::SecretFilter`, which the core puts behind the hook.
//!
//! **An entry it changed says so.** What the log then holds is not what
//! the model was sent or wrote, and on the providers that tie reasoning to
//! the history before it, the reasoning that came after can no longer be
//! replayed against it. So the entry is written with `"redacted": true` in
//! its envelope ([`Entry::redacted`](crate::Entry)), and whoever resumes
//! the session reads the mark and sends what follows without its reasoning
//! (`docs/architecture/07-sessions-and-context.md`, "Secrets in sessions").
//! A caller that altered an entry itself before handing it over, as the
//! core does when a provider's own block holds a secret, sets the mark
//! with `Session::append_altered`.
//!
//! **What it does not see:**
//!
//! - **The provider's own blocks** (`raw` fields: reasoning signatures,
//!   encrypted content, the provider's usage object). Changing a byte of one
//!   makes the provider refuse it on replay. A block that holds a secret is
//!   the caller's to deal with before the entry is handed over: the core
//!   stores such a message without its raw blocks, and marks it.
//! - **File contents kept for undo** (`checkpoints/`). A restore must put
//!   back exactly what was there.
//! - **Artifacts that are not text**, such as images.

use std::borrow::Cow;

use serde_json::Value;

/// Masks secrets in text before it is written to a session.
pub trait Redactor: Send + Sync + std::fmt::Debug {
    /// The text with anything that looks like a secret masked. Text with
    /// nothing to mask comes back borrowed.
    fn redact<'a>(&self, text: &'a str) -> Cow<'a, str>;

    /// Whether this redactor never changes anything, so that the work of
    /// walking an entry can be skipped.
    fn is_noop(&self) -> bool {
        false
    }
}

/// A redactor that changes nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRedaction;

impl Redactor for NoRedaction {
    fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(text)
    }

    fn is_noop(&self) -> bool {
        true
    }
}

/// The fields of an entry that hold JSON somebody else wrote: a tool
/// call's arguments (the model's), a tool result's payload, and the
/// free-form fields of this crate's own records. A key named `raw` inside
/// one is that somebody's, not a provider's block.
const FREE_FORM: [&str; 6] = [
    "arguments",
    "payload",
    "data",
    "detail",
    "basis",
    "allowed_by",
];

/// Applies `redactor` to every string in `value`, except in fields named
/// `raw`, which hold a provider's blocks. Says whether it changed anything.
///
/// A provider's block is a `raw` field of a part or of a usage record.
/// Inside JSON that is somebody's own ([`FREE_FORM`]) the name means
/// nothing, and what is under it is redacted like the rest.
pub(crate) fn value(value: &mut Value, redactor: &dyn Redactor) -> bool {
    walk(value, redactor, false, Raw::Keep) > 0
}

/// What an export reads (since step 25): every string the log's writer
/// masks, masked by the same walk and the same rule of [`FREE_FORM`], and
/// every provider block (`raw`) dropped, which the log keeps unmasked
/// because a changed byte makes the provider refuse it on replay, and which
/// an export never replays. Returns how many strings it masked.
///
/// The core calls it on each item of `session/read` with `redacted`, with
/// the filter the log's writer has, so the two can never mask different
/// sets: one walk, one list of free-form fields.
pub fn for_export(value: &mut Value, redactor: &dyn Redactor) -> usize {
    walk(value, redactor, false, Raw::Drop)
}

/// What the walk does with a provider's block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Raw {
    /// Leaves it as it is, unmasked: the log, which replays it.
    Keep,
    /// Takes it out: an export.
    Drop,
}

/// Masks the strings of `value`, and returns how many it changed. A
/// string the redactor hands back owned but the same is not a change: an
/// entry is marked altered only for what it no longer holds (the full
/// review of step 25's second pull request).
fn walk(value: &mut Value, redactor: &dyn Redactor, free_form: bool, raw: Raw) -> usize {
    match value {
        Value::String(text) => {
            if let Cow::Owned(masked) = redactor.redact(text)
                && masked != *text
            {
                *text = masked;
                return 1;
            }
            0
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| walk(item, redactor, free_form, raw))
            .sum(),
        Value::Object(fields) => {
            if !free_form && raw == Raw::Drop {
                fields.remove("raw");
            }
            let mut changed = 0;
            for (key, item) in fields.iter_mut() {
                if free_form || key != "raw" {
                    let free_form = free_form || FREE_FORM.contains(&key.as_str());
                    changed += walk(item, redactor, free_form, raw);
                }
            }
            changed
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    /// Masks the word `hunter2`, for tests.
    #[derive(Debug)]
    pub(crate) struct MaskHunter;

    impl Redactor for MaskHunter {
        fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
            if text.contains("hunter2") {
                Cow::Owned(text.replace("hunter2", "[redacted]"))
            } else {
                Cow::Borrowed(text)
            }
        }
    }

    /// An export's walk masks every string the log's writer masks, in the
    /// same places, and drops the provider's blocks the log keeps (step
    /// 25): one walk, so the two never part.
    #[test]
    fn an_export_masks_what_the_log_masks_and_drops_the_providers_blocks() {
        let entry = json!({
            "item": {"parts": [
                {"kind": "text", "text": "the password is hunter2", "raw": "{\"text\":\"hunter2\"}"},
                {"kind": "tool_call", "arguments": {"raw": "hunter2", "cmd": "echo hunter2"}},
                {"kind": "tool_result", "payload": {"raw": {"out": "hunter2"}}}
            ],
            "usage": {"input": 3, "raw": {"key": "hunter2"}}},
        });
        let mut logged = entry.clone();
        assert!(value(&mut logged, &MaskHunter));
        let mut exported = entry;
        assert_eq!(for_export(&mut exported, &MaskHunter), 4);
        // What the log masked, the export masked the same.
        let mut without_raw = logged.clone();
        without_raw["item"]["parts"][0]
            .as_object_mut()
            .unwrap()
            .remove("raw");
        without_raw["item"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("raw");
        assert_eq!(exported, without_raw);
        // A field named `raw` that is somebody's own is kept, masked.
        assert_eq!(
            exported["item"]["parts"][1]["arguments"]["raw"],
            json!("[redacted]")
        );
        assert!(!exported.to_string().contains("hunter2"));
    }

    /// The core's filter, for tests.
    #[derive(Debug)]
    struct Filter(cineris_base::SecretFilter);

    impl Redactor for Filter {
        fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
            self.0.redact(text)
        }
    }

    /// A message that quotes a web address whose password is masked
    /// already (one an export wrote, or the log's own text copied back) is
    /// not marked altered: nothing in it changed, and a resume would drop
    /// the reasoning after it for nothing (the full review of step 25's
    /// second pull request).
    #[test]
    fn a_message_quoting_a_masked_address_is_not_marked_altered() {
        let filter = Filter(cineris_base::SecretFilter::new());
        let quoted = json!({"item": {"parts": [
            {"kind": "text", "text": "the proxy is http://alice:[redacted]@proxy.example:3128/"}
        ]}});
        let mut entry = quoted.clone();
        assert!(!value(&mut entry, &filter), "{entry}");
        assert_eq!(entry, quoted);
        assert_eq!(for_export(&mut entry, &filter), 0);
        // The same address with its password is masked, and marked.
        let mut entry = json!({"item": {"parts": [
            {"kind": "text", "text": "the proxy is http://alice:hunter2@proxy.example:3128/"}
        ]}});
        assert!(value(&mut entry, &filter));
        assert_eq!(entry, quoted);
    }

    /// Hands back an owned copy of every text, changed or not.
    #[derive(Debug)]
    struct OwnedCopy;

    impl Redactor for OwnedCopy {
        fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
            Cow::Owned(text.to_owned())
        }
    }

    /// A redactor that hands back the same text owned changed nothing:
    /// the entry is not marked, and an export counts nothing.
    #[test]
    fn the_same_text_handed_back_owned_is_no_change() {
        let mut entry = json!({"item": {"parts": [{"kind": "text", "text": "hello"}]}});
        assert!(!value(&mut entry, &OwnedCopy));
        assert_eq!(for_export(&mut entry, &OwnedCopy), 0);
    }

    #[test]
    fn every_string_is_redacted_except_raw_blocks() {
        let mut entry = json!({
            "item": {"parts": [
                {"kind": "text", "text": "the password is hunter2", "raw": "{\"text\":\"hunter2\"}"},
                {"kind": "tool_call", "arguments": {"cmd": "echo hunter2"}}
            ]},
            "n": 3
        });
        assert!(value(&mut entry, &MaskHunter), "it says that it masked");
        assert!(!value(&mut entry, &MaskHunter), "and that there is no more");
        assert_eq!(
            entry,
            json!({
                "item": {"parts": [
                    {"kind": "text", "text": "the password is [redacted]", "raw": "{\"text\":\"hunter2\"}"},
                    {"kind": "tool_call", "arguments": {"cmd": "echo [redacted]"}}
                ]},
                "n": 3
            })
        );
    }

    /// `raw` is a provider's block where a part or a usage record has it.
    /// Under a name the model chose, in a tool call's arguments or in what
    /// a tool returned, it is a key like any other.
    #[test]
    fn a_key_named_raw_in_somebody_elses_json_is_redacted() {
        let mut entry = json!({
            "item": {"parts": [
                {"kind": "tool_call",
                 "arguments": {"raw": "hunter2", "deep": [{"raw": {"x": "hunter2"}}]},
                 "raw": "{\"input\":\"hunter2\"}"}
            ]},
            "result": {"payload": {"raw": "hunter2"}},
            "usage": {"raw": "{\"note\":\"hunter2\"}"}
        });
        assert!(value(&mut entry, &MaskHunter));
        assert_eq!(
            entry,
            json!({
                "item": {"parts": [
                    {"kind": "tool_call",
                     "arguments": {"raw": "[redacted]", "deep": [{"raw": {"x": "[redacted]"}}]},
                     "raw": "{\"input\":\"hunter2\"}"}
                ]},
                "result": {"payload": {"raw": "[redacted]"}},
                "usage": {"raw": "{\"note\":\"hunter2\"}"}
            })
        );
    }
}
