//! A hook's contract (`docs/architecture/12-extensions.md`, "Tier 3:
//! hooks"; `docs/steps/28/answered.md`, points 3 and 6), the same for a
//! command and a script function:
//!
//! - [`Filter`]: the tools a hook on a tool event runs on, by name or as
//!   `source/*`; a `tool_call` is matched by `tool_call` and by the
//!   `source/tool` it names;
//! - [`answer`]: what a hook answers, read strictly: an unknown field, a
//!   field its event does not take, or an unknown decision is a failure;
//! - [`command`]: a hook command, compiled from its configuration's entry:
//!   its program found by `cineris.exec`'s rules, its placeholders whole
//!   arguments, its caps and its times; and what it is pinned and shown by.
//!
//! Where a hook runs, and how its start is decided, is the core's.

pub mod answer;
pub mod command;

pub use answer::{Answer, Decision};
pub use command::{Arg, CommandHook, Entry, Expansion};

use crate::registration::{MAX_HOOK_TOOLS, check_name};

/// The tools a hook on a tool event runs on: each entry a tool's name
/// (`edit`, `tool_call`), a source's tool (`tracker/find_issue`), or every
/// tool of a source (`tracker/*`). Empty: every tool.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    entries: Vec<String>,
}

impl Filter {
    /// The filter of `entries`, each checked: at most [`MAX_HOOK_TOOLS`],
    /// each a name by the registry's rules ([`check_name`]), `source/name`
    /// or `source/*`.
    ///
    /// # Errors
    ///
    /// Which entry is not one, and why.
    pub fn new(entries: Vec<String>) -> Result<Self, String> {
        if entries.len() > MAX_HOOK_TOOLS {
            return Err(format!(
                "`tools` lists at most {MAX_HOOK_TOOLS}, and this lists {}",
                entries.len()
            ));
        }
        for entry in &entries {
            let fine = match entry.split_once('/') {
                None => check_name(entry).is_ok(),
                Some((source, tool)) => {
                    check_name(source).is_ok() && (tool == "*" || check_name(tool).is_ok())
                }
            };
            if !fine {
                return Err(format!(
                    "`{}` in `tools` is not a tool: a name of ASCII letters, digits, `_`, `-` and \
                     `.`, `source/name`, or `source/*` for every tool of a source",
                    cineris_base::in_code(cineris_base::text::truncate_bytes(entry, 80))
                ));
            }
        }
        Ok(Self { entries })
    }

    /// The entries, as written.
    #[must_use]
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Whether it runs on every tool: it lists none.
    #[must_use]
    pub fn is_every(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a call of `tool` runs a hook with this filter. `named` is,
    /// for a `tool_call`, the `source/tool` it names: such a call is
    /// matched by `tool_call` and by that name.
    #[must_use]
    pub fn matches(&self, tool: &str, named: Option<&str>) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        let names = std::iter::once(tool).chain(named);
        names.into_iter().any(|name| {
            self.entries
                .iter()
                .any(|entry| match entry.strip_suffix('*') {
                    // `source/*`: the slash is kept, so `tracker/*` is not
                    // `trackers/x`.
                    Some(source) => name.starts_with(source) && name.len() > source.len(),
                    None => entry == name,
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(entries: &[&str]) -> Filter {
        Filter::new(entries.iter().map(|entry| (*entry).to_owned()).collect()).unwrap()
    }

    #[test]
    fn an_empty_filter_runs_on_every_tool() {
        let every = filter(&[]);
        assert!(every.is_every());
        assert!(every.matches("edit", None));
        assert!(every.matches("tool_call", Some("tracker/find")));
    }

    #[test]
    fn a_tool_call_is_matched_by_tool_call_and_by_the_tool_it_names() {
        let named = Some("tracker/find_issue");
        assert!(filter(&["tool_call"]).matches("tool_call", named));
        assert!(filter(&["tracker/find_issue"]).matches("tool_call", named));
        assert!(filter(&["tracker/*"]).matches("tool_call", named));
        assert!(!filter(&["tracker/other"]).matches("tool_call", named));
        assert!(!filter(&["trackers/*"]).matches("tool_call", named));
        assert!(!filter(&["edit"]).matches("tool_call", named));
        // A built-in tool is its own name, and no source's.
        assert!(filter(&["edit", "write"]).matches("write", None));
        assert!(!filter(&["tracker/*"]).matches("edit", None));
        // `source/*` needs a tool after the slash.
        assert!(!filter(&["tracker/*"]).matches("tool_call", Some("tracker/")));
    }

    #[test]
    fn an_entry_is_a_name_a_sources_tool_or_a_sources_every_tool() {
        for good in ["edit", "tool_call", "a.b-c_d", "tracker/find", "tracker/*"] {
            assert!(Filter::new(vec![good.to_owned()]).is_ok(), "{good}");
        }
        for bad in [
            "",
            "*",
            "ed*t",
            "a/b/c",
            "/edit",
            "edit/",
            "a b",
            "tracker/*x",
            "*/x",
            "é",
        ] {
            let refused = Filter::new(vec![bad.to_owned()]).unwrap_err();
            assert!(refused.contains("is not a tool"), "{bad}: {refused}");
        }
        let many = vec!["edit".to_owned(); MAX_HOOK_TOOLS + 1];
        assert!(Filter::new(many).unwrap_err().contains("at most 64"));
    }
}
