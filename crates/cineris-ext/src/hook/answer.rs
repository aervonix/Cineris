//! What a hook answers (`docs/architecture/12-extensions.md`, "Tier 3:
//! hooks", the contract): nothing, or one JSON object.
//!
//! | Field | Events | Meaning |
//! |---|---|---|
//! | `decision` | `tool.before` (`allow`, `deny`, `ask`), `prompt.submit` (`allow`, `deny`) | Absent: no opinion. |
//! | `reason` | the same | Shown to the user, and to the model on a `deny`. |
//! | `arguments` | `tool.before` | Replacement arguments: an object. |
//! | `context` | every event but `tool.before` and `session.end` | Text for a notice, a note on a result, or a summary's instructions, cut to [`MAX_CONTEXT`]. |
//!
//! **Read strictly** (the answer decides, so it is enforced): a field that
//! is not one of these, a field its event does not take, a field given
//! twice, a decision that is not one of its event's, or anything but one
//! object is a failure, which on a gating event counts as `deny`. So is an
//! object anywhere in `arguments` that holds a key twice: a reader that
//! takes the first and one that takes the last would see two calls. The
//! answer is at most [`MAX_ANSWER`] bytes, its `arguments` with it.

use serde::Deserialize;
use serde_json::Value as Json;

use crate::event::Event;

/// The most of a hook's standard output that is read.
pub const MAX_OUTPUT: usize = 1024 * 1024;

/// The longest answer: more is a failure.
pub const MAX_ANSWER: usize = 64 * 1024;

/// The longest `context` kept: more is cut, and said to be.
pub const MAX_CONTEXT: usize = 8 * 1024;

/// A hook's decision on a gating event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// A hook's answer, read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answer {
    /// Its decision. `None`: no opinion.
    pub decision: Option<Decision>,
    /// Why, as it says.
    pub reason: Option<String>,
    /// For `tool.before`: the arguments it would have the call run with.
    pub arguments: Option<serde_json::Map<String, Json>>,
    /// The text it adds, cut to [`MAX_CONTEXT`].
    pub context: Option<String>,
    /// Whether `context` was cut.
    pub context_cut: bool,
}

/// The answer as written: every field, none of them twice, nothing else.
#[derive(Default)]
struct Written {
    decision: Option<String>,
    reason: Option<String>,
    arguments: Option<Json>,
    context: Option<String>,
}

/// The fields of an answer.
const FIELDS: &[&str] = &["decision", "reason", "arguments", "context"];

impl<'de> Deserialize<'de> for Written {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = Written;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Written, A::Error> {
                use serde::de::Error as _;
                let mut written = Written::default();
                let mut seen: Vec<String> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    // A field given twice is read by some readers as the
                    // first and by others as the last: refused.
                    if seen.contains(&key) {
                        return Err(A::Error::custom(format_args!("duplicate field `{key}`")));
                    }
                    match key.as_str() {
                        "decision" => written.decision = map.next_value()?,
                        "reason" => written.reason = map.next_value()?,
                        "arguments" => {
                            written.arguments = map.next_value::<Option<Strict>>()?.map(|it| it.0);
                        }
                        "context" => written.context = map.next_value()?,
                        other => return Err(A::Error::unknown_field(other, FIELDS)),
                    }
                    seen.push(key);
                }
                Ok(written)
            }
        }
        deserializer.deserialize_map(Fields)
    }
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
                    "duplicate key `{}` in `arguments`",
                    cineris_base::in_code(cineris_base::text::truncate_bytes(&key, 40))
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

impl Answer {
    /// Reads a hook's answer to `event` from what it wrote to its standard
    /// output: nothing (or only white space) is no opinion; otherwise one
    /// JSON object, read strictly.
    ///
    /// # Errors
    ///
    /// Why it is not an answer to `event`, in a sentence that can follow
    /// "the hook's answer".
    pub fn read(event: Event, output: &[u8]) -> Result<Self, String> {
        if output.len() > MAX_ANSWER {
            return Err(format!(
                "is {} bytes, and an answer is at most {} KiB",
                output.len(),
                MAX_ANSWER / 1024
            ));
        }
        if output.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self::default());
        }
        let written: Written = serde_json::from_slice(output)
            .map_err(|error| format!("is not one JSON object of the contract: {error}"))?;
        let given = [
            ("decision", written.decision.is_some()),
            ("reason", written.reason.is_some()),
            ("arguments", written.arguments.is_some()),
            ("context", written.context.is_some()),
        ];
        if let Some((field, _)) = given
            .iter()
            .find(|(field, given)| *given && !takes(event, field))
        {
            return Err(format!(
                "has `{field}`, which an answer to `{}` does not take",
                event.as_str()
            ));
        }
        let decision = match written.decision.as_deref() {
            None => None,
            Some("allow") => Some(Decision::Allow),
            Some("deny") => Some(Decision::Deny),
            Some("ask") if event == Event::ToolBefore => Some(Decision::Ask),
            Some(other) => {
                return Err(format!(
                    "has the decision `{}`, which is not one of `{}`'s: {}",
                    cineris_base::in_code(cineris_base::text::truncate_bytes(other, 40)),
                    event.as_str(),
                    if event == Event::ToolBefore {
                        "`allow`, `deny` or `ask`"
                    } else {
                        "`allow` or `deny`"
                    }
                ));
            }
        };
        let arguments = match written.arguments {
            None => None,
            Some(Json::Object(arguments)) => Some(arguments),
            Some(_) => return Err("has `arguments` that are not an object".to_owned()),
        };
        let (context, context_cut) = match written.context {
            Some(text) if text.len() > MAX_CONTEXT => (
                Some(cineris_base::text::truncate_bytes(&text, MAX_CONTEXT).to_owned()),
                true,
            ),
            other => (other, false),
        };
        Ok(Self {
            decision,
            reason: written.reason,
            arguments,
            context,
            context_cut,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(event: Event, text: &str) -> Result<Answer, String> {
        Answer::read(event, text.as_bytes())
    }

    #[test]
    fn nothing_is_no_opinion() {
        for nothing in ["", " \n\t\r\n"] {
            assert_eq!(read(Event::ToolBefore, nothing).unwrap(), Answer::default());
        }
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
        let deep = read(
            Event::ToolBefore,
            r#"{"arguments": {"a": [1, -2, 2.5, true, null, "x", {"b": {}}], "c": []}}"#,
        )
        .unwrap();
        assert_eq!(
            Json::Object(deep.arguments.unwrap()),
            serde_json::json!({"a": [1, -2, 2.5, true, null, "x", {"b": {}}], "c": []})
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
                "unknown field",
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
                r#"{"decision":"allow","decision":"deny"}"#,
                "duplicate",
            ),
            (
                Event::ToolBefore,
                r#"{"decision":1}"#,
                "not one JSON object",
            ),
            (Event::ToolBefore, r#"{"arguments":[1]}"#, "not an object"),
            (
                Event::ToolBefore,
                r#"{"arguments":{"command":"ls","command":"rm -rf x"}}"#,
                "duplicate key `command` in `arguments`",
            ),
            (
                Event::ToolBefore,
                r#"{"arguments":{"edits":[{"path":"a","path":"b"}]}}"#,
                "duplicate key `path` in `arguments`",
            ),
            (Event::ToolBefore, r#"["deny"]"#, "not one JSON object"),
            (Event::ToolBefore, r#"{} {}"#, "not one JSON object"),
            (Event::ToolBefore, "deny", "not one JSON object"),
            (
                Event::ToolBefore,
                "{\"decision\":\"deny\"",
                "not one JSON object",
            ),
        ] {
            let failed = read(event, text).unwrap_err();
            assert!(failed.contains(said), "{text}: {failed}");
        }
    }

    #[test]
    fn a_field_its_event_does_not_take_is_a_failure() {
        for (event, text) in [
            (Event::ToolAfter, r#"{"decision":"deny"}"#),
            (Event::ToolAfter, r#"{"arguments":{}}"#),
            (Event::PromptSubmit, r#"{"arguments":{}}"#),
            (Event::ToolBefore, r#"{"context":"x"}"#),
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
    fn an_answer_is_bounded_and_its_context_is_cut() {
        let long = format!(r#"{{"context":"{}"}}"#, "a".repeat(MAX_ANSWER));
        assert!(
            read(Event::TurnEnd, &long)
                .unwrap_err()
                .contains("at most 64 KiB")
        );
        let context = format!(r#"{{"context":"{}é"}}"#, "a".repeat(MAX_CONTEXT - 1));
        let cut = read(Event::TurnEnd, &context).unwrap();
        assert!(cut.context_cut);
        assert_eq!(
            cut.context.unwrap().len(),
            MAX_CONTEXT - 1,
            "cut on a character"
        );
        let fits = read(Event::TurnEnd, r#"{"context":"short"}"#).unwrap();
        assert!(!fits.context_cut);
    }
}
