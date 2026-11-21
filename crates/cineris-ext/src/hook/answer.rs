//! What a hook answers (`docs/architecture/12-extensions.md`, "Tier 3:
//! hooks", the contract): nothing, or one JSON object.
//!
//! - [`value`] reads what a hook command wrote as one JSON value, **with no
//!   object in it, at any depth, holding a key twice**: a reader that takes
//!   the first of two and one that takes the last would see two answers
//!   (`{"decision":"deny","decision":"allow"}`), or two calls. It is what
//!   the loop is handed for its events (`cineris_agent::HookOutcome`), and
//!   the loop reads it by the event's contract.
//! - [`Answer::read`] reads it by the contract of an event the loop does
//!   not have (`session.start`, `session.end`), and agrees with the loop's
//!   reading of the others:
//!
//! | Field | Events | Meaning |
//! |---|---|---|
//! | `decision` | `tool.before` (`allow`, `deny`, `ask`), `prompt.submit` (`allow`, `deny`) | Absent: no opinion. |
//! | `reason` | the same | Shown to the user, and to the model on a `deny`, cut to [`MAX_TEXT`]. |
//! | `arguments` | `tool.before` | Replacement arguments. |
//! | `context` | every event but `tool.before` and `session.end` | Text for a notice, a note on a result, or a summary's instructions, cut to [`MAX_TEXT`]. |
//!
//! **Read strictly** (the answer decides, so it is enforced): a field that
//! is not one of these, a field its event does not take (present as
//! `null` too), a field of the wrong type, a decision that is not one of
//! its event's, or anything but one object or `null` is a failure, which
//! on a gating event counts as `deny`. The answer is at most
//! [`MAX_ANSWER`] bytes, its `arguments` with it. The text kept is as the
//! hook wrote it: whoever shows it to the user or the model escapes what
//! acts unseen in it, and quotes it under a line that names the hook (the
//! loop's own reading for its events; the core's notice for
//! `session.start`). A failure's words quote what the hook wrote only as
//! code, cut ([`cineris_base::in_code`]).

use serde::Deserialize;
use serde_json::Value as Json;

use crate::event::Event;

/// The most of a hook's standard output that is read.
pub const MAX_OUTPUT: usize = 1024 * 1024;

/// The longest answer: more is a failure.
pub const MAX_ANSWER: usize = 64 * 1024;

/// The longest `reason` or `context` kept: more is cut, and said to be.
pub const MAX_TEXT: usize = 8 * 1024;

/// A hook's decision on a gating event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// It has no objection. It never lifts the policy's question.
    Allow,
    /// It refuses.
    Deny,
    /// The user is asked, the hook's reason shown.
    Ask,
}

impl Decision {
    /// The decision as a hook writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }
}

/// A hook's answer, read by its event's contract.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answer {
    /// Its decision. `None`: no opinion.
    pub decision: Option<Decision>,
    /// Why, as it says, cut to [`MAX_TEXT`].
    pub reason: Option<String>,
    /// For `tool.before`: what it would have the call run with.
    pub arguments: Option<Json>,
    /// The text it adds, cut to [`MAX_TEXT`].
    pub context: Option<String>,
    /// Whether `reason` or `context` was cut.
    pub cut: bool,
}

/// What a failure shows of a text the hook wrote: as code, at most 40
/// bytes of it.
fn shown(text: &str) -> String {
    cineris_base::in_code(cineris_base::text::truncate_bytes(text, 40)).into_owned()
}

/// What a hook wrote to its standard output as one JSON value: `None` for
/// nothing (or only white space). Strict: no object in it, at any depth,
/// holds a key twice.
///
/// # Errors
///
/// Why it is not one JSON value, in a sentence that can follow "the hook's
/// answer".
pub fn value(output: &[u8]) -> Result<Option<Json>, String> {
    if output.len() > MAX_ANSWER {
        return Err(format!(
            "is {} bytes, and an answer is at most {} KiB",
            output.len(),
            MAX_ANSWER / 1024
        ));
    }
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let mut reader = serde_json::Deserializer::from_slice(output);
    let Strict(value) = Strict::deserialize(&mut reader).map_err(|error| {
        format!(
            "is not one JSON value: {}",
            cineris_base::visible(&error.to_string())
        )
    })?;
    reader
        .end()
        .map_err(|_| "is more than one JSON value".to_owned())?;
    Ok(Some(value))
}

/// A JSON value read with no object in it, at any depth, holding a key
/// twice.
struct Strict(Json);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictValue).map(Strict)
    }
}

/// Reads one JSON value of any kind, refusing a key given twice.
struct StrictValue;

impl<'de> serde::de::Visitor<'de> for StrictValue {
    type Value = Json;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Json, E> {
        Ok(Json::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Json, E> {
        Ok(Json::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
        Ok(Json::from(value))
    }

    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Json, E> {
        serde_json::Number::from_f64(value)
            .map(Json::Number)
            .ok_or_else(|| E::custom("a number JSON cannot hold"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Json, E> {
        Ok(Json::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Json, E> {
        Ok(Json::String(value))
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element::<Strict>()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        use serde::de::Error as _;
        let mut object = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(A::Error::custom(format_args!(
                    "the key `{}` is given twice",
                    shown(&key)
                )));
            }
            let Strict(value) = map.next_value::<Strict>()?;
            object.insert(key, value);
        }
        Ok(Json::Object(object))
    }
}

/// The fields an event's answer may hold.
fn takes(event: Event, field: &str) -> bool {
    match field {
        "decision" | "reason" => matches!(event, Event::ToolBefore | Event::PromptSubmit),
        "arguments" => event == Event::ToolBefore,
        "context" => !matches!(event, Event::ToolBefore | Event::SessionEnd),
        _ => false,
    }
}

/// `text` cut to [`MAX_TEXT`] at a character, and whether it was.
fn cut(text: String) -> (String, bool) {
    if text.len() > MAX_TEXT {
        (
            cineris_base::text::truncate_bytes(&text, MAX_TEXT).to_owned(),
            true,
        )
    } else {
        (text, false)
    }
}

impl Answer {
    /// Reads a hook's answer to `event` from what it wrote to its standard
    /// output ([`value`]): nothing, `null`, or one object of the event's
    /// contract.
    ///
    /// # Errors
    ///
    /// Why it is not an answer to `event`, in a sentence that can follow
    /// "the hook's answer".
    pub fn read(event: Event, output: &[u8]) -> Result<Self, String> {
        Self::of(event, value(output)?)
    }

    /// [`Answer::read`] of a value [`value`] read already: `None` for
    /// nothing.
    ///
    /// # Errors
    ///
    /// As [`Answer::read`].
    pub fn of(event: Event, value: Option<Json>) -> Result<Self, String> {
        let fields = match value {
            None | Some(Json::Null) => return Ok(Self::default()),
            Some(Json::Object(fields)) => fields,
            Some(_) => return Err("is not a JSON object".to_owned()),
        };
        let mut answer = Self::default();
        for (field, given) in fields {
            // A field the event does not take is refused present as `null`
            // too: `null` is no leave to name it.
            if !takes(event, &field) {
                return Err(format!(
                    "has `{}`, which an answer to `{}` does not take",
                    shown(&field),
                    event.as_str()
                ));
            }
            let text = |given: Json| match given {
                Json::String(text) => Ok(text),
                _ => Err(format!("has `{field}` that is not text")),
            };
            match field.as_str() {
                "decision" => {
                    answer.decision = Some(match text(given)?.as_str() {
                        "allow" => Decision::Allow,
                        "deny" => Decision::Deny,
                        "ask" if event == Event::ToolBefore => Decision::Ask,
                        other => {
                            return Err(format!(
                                "has the decision `{}`, which is not one of `{}`'s: {}",
                                shown(other),
                                event.as_str(),
                                if event == Event::ToolBefore {
                                    "`allow`, `deny` or `ask`"
                                } else {
                                    "`allow` or `deny`"
                                }
                            ));
                        }
                    });
                }
                "reason" => {
                    let (reason, was_cut) = cut(text(given)?);
                    answer.reason = Some(reason);
                    answer.cut |= was_cut;
                }
                "context" => {
                    let (context, was_cut) = cut(text(given)?);
                    answer.context = Some(context);
                    answer.cut |= was_cut;
                }
                _ => answer.arguments = Some(given),
            }
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(event: Event, text: &str) -> Result<Answer, String> {
        Answer::read(event, text.as_bytes())
    }

    #[test]
    fn nothing_and_null_are_no_opinion() {
        for nothing in ["", " \n\t\r\n", "null", " null \n"] {
            assert_eq!(read(Event::ToolBefore, nothing).unwrap(), Answer::default());
        }
        assert_eq!(value(b"  ").unwrap(), None);
        assert_eq!(value(b"null").unwrap(), Some(Json::Null));
    }

    #[test]
    fn a_gating_answer_is_read_with_its_reason_and_arguments() {
        let answer = read(
            Event::ToolBefore,
            r#"{"decision": "ask", "reason": "a generated file", "arguments": {"path": "b"}}"#,
        )
        .unwrap();
        assert_eq!(answer.decision, Some(Decision::Ask));
        assert_eq!(answer.reason.as_deref(), Some("a generated file"));
        assert_eq!(answer.arguments.unwrap()["path"], "b");
        // Every kind of value reads as it is written.
        let deep = value(br#"{"a": [1, -2, 2.5, true, null, "x", {"b": {}}], "c": []}"#).unwrap();
        assert_eq!(
            deep,
            Some(serde_json::json!({"a": [1, -2, 2.5, true, null, "x", {"b": {}}], "c": []}))
        );
        let denied = read(Event::PromptSubmit, r#"{"decision":"deny","context":"x"}"#).unwrap();
        assert_eq!(denied.decision, Some(Decision::Deny));
        assert_eq!(denied.context.as_deref(), Some("x"));
    }

    #[test]
    fn an_unknown_field_or_decision_is_a_failure() {
        for (event, text, said) in [
            (
                Event::ToolBefore,
                r#"{"decision":"allow","why":"x"}"#,
                "has `why`",
            ),
            (Event::ToolBefore, r#"{"decision":"maybe"}"#, "not one of"),
            (Event::ToolBefore, r#"{"decision":"ALLOW"}"#, "not one of"),
            (
                Event::PromptSubmit,
                r#"{"decision":"ask"}"#,
                "`allow` or `deny`",
            ),
            (
                Event::ToolBefore,
                r#"{"decision":"deny","decision":"allow"}"#,
                "the key `decision` is given twice",
            ),
            (Event::ToolBefore, r#"{"decision":1}"#, "not text"),
            (Event::ToolBefore, r#"["deny"]"#, "not a JSON object"),
            (Event::ToolBefore, r#"{} {}"#, "more than one JSON value"),
            (Event::ToolBefore, "deny", "not one JSON value"),
            (
                Event::ToolBefore,
                "{\"decision\":\"deny\"",
                "not one JSON value",
            ),
            (
                Event::ToolBefore,
                r#"{"arguments":{"command":"ls","command":"rm -rf x"}}"#,
                "the key `command` is given twice",
            ),
            (
                Event::ToolBefore,
                r#"{"arguments":{"edits":[{"path":"a","path":"b"}]}}"#,
                "the key `path` is given twice",
            ),
        ] {
            let failed = read(event, text).unwrap_err();
            assert!(failed.contains(said), "{text}: {failed}");
        }
    }

    #[test]
    fn a_key_the_hook_wrote_is_quoted_as_code_and_cut() {
        let long = format!("{}`\u{202e}", "k".repeat(60));
        let failed = read(Event::ToolBefore, &format!(r#"{{"{long}": 1}}"#)).unwrap_err();
        assert!(!failed.contains('\u{202e}'), "{failed}");
        assert!(!failed.contains(&long), "{failed}");
        let twice = value(br#"{"a`b": 1, "a`b": 2}"#).unwrap_err();
        assert!(twice.contains("a\\`b"), "{twice}");
    }

    #[test]
    fn a_field_its_event_does_not_take_is_a_failure_null_or_not() {
        for (event, text) in [
            (Event::ToolAfter, r#"{"decision":"deny"}"#),
            (Event::ToolAfter, r#"{"arguments":{}}"#),
            (Event::ToolAfter, r#"{"arguments":null}"#),
            (Event::PromptSubmit, r#"{"arguments":{}}"#),
            (Event::ToolBefore, r#"{"context":"x"}"#),
            (Event::ToolBefore, r#"{"context":null}"#),
            (Event::SessionEnd, r#"{"context":"x"}"#),
            (Event::TurnEnd, r#"{"reason":"x"}"#),
        ] {
            let failed = read(event, text).unwrap_err();
            assert!(failed.contains("does not take"), "{text}: {failed}");
        }
        // Each event takes what its row says.
        for event in [
            Event::SessionStart,
            Event::PromptSubmit,
            Event::ToolAfter,
            Event::TurnEnd,
            Event::CompactionBefore,
        ] {
            assert!(read(event, r#"{"context":"x"}"#).is_ok(), "{event:?}");
        }
        assert_eq!(read(Event::SessionEnd, "{}").unwrap(), Answer::default());
    }

    #[test]
    fn an_answer_is_bounded_and_its_texts_are_cut() {
        let long = format!(r#"{{"context":"{}"}}"#, "a".repeat(MAX_ANSWER));
        assert!(
            read(Event::TurnEnd, &long)
                .unwrap_err()
                .contains("at most 64 KiB")
        );
        let context = format!(r#"{{"context":"{}é"}}"#, "a".repeat(MAX_TEXT - 1));
        let cut = read(Event::TurnEnd, &context).unwrap();
        assert!(cut.cut);
        assert_eq!(
            cut.context.unwrap().len(),
            MAX_TEXT - 1,
            "cut on a character"
        );
        let reason = format!(r#"{{"reason":"{}"}}"#, "r".repeat(MAX_TEXT + 10));
        let cut = read(Event::PromptSubmit, &reason).unwrap();
        assert!(cut.cut);
        assert_eq!(cut.reason.unwrap().len(), MAX_TEXT);
        let fits = read(Event::TurnEnd, r#"{"context":"short"}"#).unwrap();
        assert!(!fits.cut);
    }
}
