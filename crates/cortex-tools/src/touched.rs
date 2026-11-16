//! What a finished call changed in the workspace, from what it declared and
//! what its result says (`docs/architecture/15-efficiency.md`, "Automatic
//! verification"): what sends a turn to the project's checks.
//!
//! A call's effects say what it **may** change, which is not what it did:
//! a command declares a write of the whole workspace, and a `write` that
//! found its content already there wrote nothing. So the result decides,
//! and this module is where its payloads are read, since the tools that
//! write them are here.
//!
//! | Call | What it changed |
//! |---|---|
//! | `write`, `edit` | Its paths, when it succeeded and its payload's `kind` is not `unchanged`. |
//! | A command | When it ran (succeeded, failed or was interrupted; not when it was refused or never started): the files its payload's `changes` name, whichever way they were found. A look that did not finish gives the workspace root: what changed under it is not known. No look at all (a command the engine found only reads) gives nothing. |
//! | Any other tool | The paths of its `write` effects, when it succeeded. |

use cortex_base::AbsPath;
use cortex_protocol::item::ToolOutcome;

use crate::contract::{Effect, Json};

/// What a call that has finished changed: real locations, the workspace
/// root standing for "something under it, not known what".
///
/// `effects` are the call's plan's, `outcome` and `payload` its result's,
/// and `root` the workspace root its payload's paths are relative to.
#[must_use]
pub fn changed_by(
    effects: &[Effect],
    outcome: &ToolOutcome,
    payload: Option<&Json>,
    root: &AbsPath,
) -> Vec<AbsPath> {
    let command = effects
        .iter()
        .any(|effect| matches!(effect, Effect::Exec { .. }));
    if command {
        return match outcome {
            // Refused before it ran: nothing of it ran.
            ToolOutcome::Denied => Vec::new(),
            _ => by_command(payload, root),
        };
    }
    if *outcome != ToolOutcome::Success {
        return Vec::new();
    }
    let unchanged = payload
        .and_then(|payload| payload.get("kind"))
        .and_then(Json::as_str)
        == Some("unchanged");
    if unchanged {
        return Vec::new();
    }
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Write(path) => Some(path.abs().clone()),
            _ => None,
        })
        .collect()
}

/// What a command's result says it changed.
fn by_command(payload: Option<&Json>, root: &AbsPath) -> Vec<AbsPath> {
    // No payload: the command never started. No `changes`: it was not
    // looked at, because the engine vouched that it only reads.
    let Some(changes) = payload
        .and_then(|payload| payload.get("changes"))
        .filter(|changes| changes.is_object())
    else {
        return Vec::new();
    };
    let mut changed: Vec<AbsPath> = ["created", "modified", "deleted"]
        .iter()
        .filter_map(|list| changes.get(list).and_then(Json::as_array))
        .flatten()
        .filter_map(Json::as_str)
        .filter_map(|relative| root.join(relative).ok())
        .collect();
    let finished = changes.get("complete").and_then(Json::as_bool) == Some(true)
        && changes.get("method").and_then(Json::as_str) != Some("none");
    if !finished {
        // What the command did is not known: the workspace stands for it.
        changed.push(root.clone());
    }
    changed
}

#[cfg(test)]
mod tests {
    use cortex_base::{PathOptions, PathResolver};
    use serde_json::json;

    use super::*;

    fn root() -> (cortex_testkit::Scratch, PathResolver) {
        let scratch = cortex_testkit::Scratch::outside("cortex-tools-touched", "unit");
        let resolver = PathResolver::new(scratch.path(), PathOptions::new()).expect("a workspace");
        (scratch, resolver)
    }

    fn command(resolver: &PathResolver) -> Vec<Effect> {
        let root = resolver.resolve(".").unwrap();
        vec![
            Effect::Exec {
                command: "make".to_owned(),
                cwd: root.clone(),
            },
            Effect::Write(root),
        ]
    }

    fn names(changed: &[AbsPath], root: &AbsPath) -> Vec<String> {
        changed
            .iter()
            .map(|path| {
                if path == root {
                    ".".to_owned()
                } else {
                    path.strip_prefix(root)
                        .map(|relative| relative.as_str().to_owned())
                        .unwrap_or_default()
                }
            })
            .collect()
    }

    #[test]
    fn a_write_that_found_its_content_there_changed_nothing() {
        let (_scratch, resolver) = root();
        let file = resolver.resolve("a.txt").unwrap();
        let effects = [Effect::Write(file.clone())];
        let wrote = json!({"path": "a.txt", "kind": "created"});
        let same = json!({"path": "a.txt", "kind": "unchanged"});
        let root = resolver.root().clone();
        assert_eq!(
            changed_by(&effects, &ToolOutcome::Success, Some(&wrote), &root),
            [file.abs().clone()]
        );
        assert!(changed_by(&effects, &ToolOutcome::Success, Some(&same), &root).is_empty());
        assert!(changed_by(&effects, &ToolOutcome::Error, Some(&wrote), &root).is_empty());
    }

    #[test]
    fn a_command_changed_what_its_look_found_and_not_the_workspace_it_declared() {
        let (_scratch, resolver) = root();
        let root = resolver.root().clone();
        let effects = command(&resolver);
        let found = |method: &str, complete: bool| {
            json!({"exit_code": 1, "changes": {
                "method": method, "modified": ["src/a.rs"], "created": ["new.rs"],
                "deleted": [], "complete": complete,
            }})
        };
        // Failed or not, what it changed counts: `sed -i … && make` that
        // fails has still edited.
        for outcome in [ToolOutcome::Success, ToolOutcome::Error] {
            for method in ["git", "mtime"] {
                let changed = changed_by(&effects, &outcome, Some(&found(method, true)), &root);
                assert_eq!(names(&changed, &root), ["new.rs", "src/a.rs"], "{method}");
            }
        }
        // A look that did not finish: something may have changed.
        let changed = changed_by(&effects, &ToolOutcome::Success, Some(&found("git", false)), &root);
        assert_eq!(names(&changed, &root), ["new.rs", "src/a.rs", "."]);
        let none = json!({"exit_code": 0, "changes": {"method": "none", "complete": true}});
        let changed = changed_by(&effects, &ToolOutcome::Success, Some(&none), &root);
        assert_eq!(names(&changed, &root), ["."]);
        // Nothing found: nothing changed, though the plan says it may write
        // the whole workspace.
        let quiet = json!({"exit_code": 0, "changes": {"method": "git", "complete": true,
            "modified": [], "created": [], "deleted": []}});
        assert!(changed_by(&effects, &ToolOutcome::Success, Some(&quiet), &root).is_empty());
        // Not looked at (the engine found it only reads), refused, or never
        // started: nothing.
        let unlooked = json!({"exit_code": 0, "changes": null});
        assert!(changed_by(&effects, &ToolOutcome::Success, Some(&unlooked), &root).is_empty());
        assert!(
            changed_by(&effects, &ToolOutcome::Denied, Some(&found("git", true)), &root).is_empty()
        );
        assert!(changed_by(&effects, &ToolOutcome::Error, None, &root).is_empty());
    }

    #[test]
    fn a_tool_that_declares_writes_changed_them_when_it_succeeded() {
        let (_scratch, resolver) = root();
        let root = resolver.root().clone();
        let file = resolver.resolve("out/report.json").unwrap();
        let effects = [
            Effect::Ext {
                tool: "ext/report".to_owned(),
            },
            Effect::Write(file.clone()),
        ];
        assert_eq!(
            changed_by(&effects, &ToolOutcome::Success, None, &root),
            [file.abs().clone()]
        );
        // One that declares nothing it writes changed nothing that can be
        // told: an MCP server's tool, today.
        let mcp = [Effect::Ext {
            tool: "mcp/search".to_owned(),
        }];
        assert!(changed_by(&mcp, &ToolOutcome::Success, None, &root).is_empty());
    }
}
