//! A hook command's start (`docs/steps/28/answered.md`, "The processes",
//! the hook command's column; point 6), in an open session, through
//! `Core::run_hook_command` until the loop's points call hooks (the step's
//! next part makes these turns).
//!
//! The programs are this test binary, entered as [`crate::HOOK`], which
//! reads the event and reports, fails or runs on as its first argument
//! says; and, for a hook that answers, a script that starts it and then
//! answers (a `.cmd` on Windows, started through `cmd.exe` as `exec`
//! starts one): the test runner writes lines of its own on the binary's
//! standard output, which no answer read strictly could carry. The tests
//! that start one are named `sys_…`, and are few: one session runs every
//! start shape in turn (one that gates, one that replaces the arguments,
//! one that fails, one that runs past its time), so that the system tier
//! pays for one session's start.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cineris_core::ext::{HookOutcome, HookRun};
use cineris_ext::hook::Decision;
use serde_json::{Value, json};

use crate::{Front, HOOK, World};

/// This test binary as a hook's program in `mode`, then `args`.
fn this_binary(mode: &str, args: &[&str]) -> Vec<String> {
    let program = std::env::current_exe().expect("the test binary has a path");
    std::iter::once(program.display().to_string())
        .chain(
            ["--exact", "launcher_entry", "--nocapture", HOOK, mode]
                .iter()
                .chain(args)
                .map(|word| (*word).to_owned()),
        )
        .collect()
}

/// The `[hooks.<name>]` entry of a hook on `event` that runs `command`,
/// with `extra` keys.
fn entry(name: &str, event: &str, command: &[String], extra: &str) -> String {
    let words: Vec<String> = command.iter().map(|word| format!("'{word}'")).collect();
    format!(
        "[hooks.{name}]\nevent = \"{event}\"\ncommand = [{}]\n{extra}\n",
        words.join(", ")
    )
}

/// A script in `dir` called `name` that hands its arguments and its input
/// to this binary, which writes them to `report` (when there is one), with
/// nothing of the binary's on the script's standard output; and then
/// answers `answer`. A `.cmd` on Windows, a shell script elsewhere. Its
/// path.
fn script(dir: &Path, name: &str, report: Option<&Path>, answer: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let exe = std::env::current_exe().expect("the test binary has a path");
    #[cfg(windows)]
    {
        let path = dir.join(format!("{name}.cmd"));
        let mut text = String::from("@echo off\r\n");
        match report {
            Some(report) => text.push_str(&format!(
                "\"{}\" --exact launcher_entry --nocapture {HOOK} report \"{}\" %* >nul 2>&1\r\n",
                exe.display(),
                report.display()
            )),
            None => text.push_str("more >nul\r\n"),
        }
        text.push_str(&format!("echo {answer}\r\n"));
        std::fs::write(&path, text).unwrap();
        path
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        let mut text = String::from("#!/bin/sh\n");
        match report {
            Some(report) => text.push_str(&format!(
                "'{}' --exact launcher_entry --nocapture {HOOK} report '{}' \"$@\" >/dev/null \
                 2>&1\n",
                exe.display(),
                report.display()
            )),
            None => text.push_str("cat >/dev/null\n"),
        }
        text.push_str(&format!("printf '%s\\n' '{answer}'\n"));
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

/// A run of a tool event's hook for `edit`, naming `paths`.
fn for_edit(paths: &[&str]) -> HookRun {
    HookRun {
        input: json!({"event": "tool.before", "tool": "edit", "arguments": {"path": "src/a.rs"}}),
        tool: Some("edit".to_owned()),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        turn: None,
    }
}

/// The answer of a hook that answered.
fn answered(outcome: &HookOutcome) -> &cineris_ext::hook::Answer {
    let HookOutcome::Answered(answer) = outcome else {
        panic!("the hook did not answer: {outcome:?}");
    };
    answer
}

/// Every start shape in one session: a hook that gates is given its
/// placeholders as whole arguments (`{path}` once for each file by place,
/// resolved and plain), the event on its standard input and no provider's
/// key, and its `deny` is read; one that replaces the arguments has them
/// read; one that exits with an error has failed, saying what it wrote to
/// standard error; and one that runs past its time on the session's clock
/// is killed, and has failed.
#[test]
fn sys_a_hook_command_gates_replaces_fails_and_runs_past_its_time() {
    let world = World::new("hook_shapes", []);
    let tools = world.dir.join("hooks");
    let report = world.dir.join("report.json");
    let gate = script(
        &tools,
        "gate",
        Some(&report),
        r#"{"decision":"deny","reason":"generated"}"#,
    );
    let swap = script(
        &tools,
        "swap",
        None,
        r#"{"arguments":{"command":"echo replaced"}}"#,
    );
    let gate_command: Vec<String> = [gate.display().to_string()]
        .into_iter()
        .chain(["{path}", "{tool}", "{workspace}", "{event}"].map(str::to_owned))
        .collect();
    world.user_config(
        &[
            entry("gate", "tool.before", &gate_command, ""),
            entry("swap", "tool.before", &[swap.display().to_string()], ""),
            entry("broken", "turn.end", &this_binary("fail", &[]), ""),
            entry(
                "slow",
                "tool.before",
                &this_binary("hang", &[]),
                "timeout = \"2s\"",
            ),
        ]
        .concat(),
    );
    world.put("src/a.rs", "fn main() {}\n");
    let env = world.shell_env(&[("ANTHROPIC_API_KEY", "sk-test-not-for-hooks")]);
    let core = world.core_with(|options| options.shell_env = Some(env));
    world.run(async {
        let front = Front::start(&world, &core).await;
        let session = front.session.clone();

        // One that gates. The same file named three ways, and on Windows
        // in another case, is one file, given once.
        let mut named = vec!["src/a.rs", "./src/a.rs", "src/../src/a.rs"];
        if cfg!(windows) {
            named.push("SRC/A.RS");
        }
        let ran = core
            .run_hook_command(&session, "gate", for_edit(&named))
            .await
            .unwrap();
        let answer = answered(&ran);
        assert_eq!(answer.decision, Some(Decision::Deny));
        assert_eq!(answer.reason.as_deref(), Some("generated"));
        let given: Value =
            serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        let args: Vec<&str> = given["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        assert_eq!(args.len(), 4, "{args:?}");
        assert!(!args[0].starts_with(r"\\?\"), "a plain path: {}", args[0]);
        assert!(
            cineris_base::same_location(
                Path::new(args[0]),
                &world.workspace.join("src").join("a.rs")
            ),
            "{}",
            args[0]
        );
        assert_eq!(args[1], "edit");
        assert!(!args[2].starts_with(r"\\?\"), "{}", args[2]);
        assert!(cineris_base::same_location(
            Path::new(args[2]),
            &world.workspace
        ));
        assert_eq!(args[3], "tool.before");
        let stdin: Value = serde_json::from_str(given["stdin"].as_str().unwrap()).unwrap();
        assert_eq!(stdin, for_edit(&[]).input, "the event on standard input");
        assert_eq!(
            given["key"],
            Value::Null,
            "no provider's key reaches a hook"
        );

        // One that replaces the arguments.
        let ran = core
            .run_hook_command(&session, "swap", for_edit(&[]))
            .await
            .unwrap();
        let answer = answered(&ran);
        assert_eq!(
            answer.arguments.clone().map(Value::Object),
            Some(json!({"command": "echo replaced"}))
        );
        assert_eq!(answer.decision, None);

        // One that fails.
        let broken = core
            .run_hook_command(
                &session,
                "broken",
                HookRun {
                    input: json!({"event": "turn.end"}),
                    ..HookRun::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            broken,
            HookOutcome::Failed(
                "The hook `broken` failed: it exited with 3, saying: the hook went wrong."
                    .to_owned()
            )
        );

        // One that runs past its time: killed when the session's clock
        // reaches it.
        let mut slow = std::pin::pin!(core.run_hook_command(&session, "slow", for_edit(&[])));
        let mut its_time = std::pin::pin!(async {
            world.timer.sleeping_for(Duration::from_secs(2)).await;
            world.timer.advance(Duration::from_secs(2));
        });
        let mut advanced = false;
        let slow = std::future::poll_fn(|context| {
            if !advanced {
                advanced = its_time.as_mut().poll(context).is_ready();
            }
            slow.as_mut().poll(context)
        })
        .await
        .unwrap();
        let HookOutcome::Failed(why) = &slow else {
            panic!("{slow:?}");
        };
        assert!(why.contains("ran past its 2000 ms"), "{why}");
        front.close().await;
    });
}

/// A project's hook is listed by the trust question in words, and a yes
/// pins it as it was shown, so that a session opened again is not asked
/// about it; it is the session's, and its start is refused all the same,
/// as the engine would ask about it at `supervised` and a project's hook
/// never puts a question: nothing is started. A project's hook of a user's
/// name is never one the session runs.
#[test]
fn a_projects_hook_is_listed_pinned_and_never_passes_a_question() {
    use crate::extensions_found::{open_answering, yes};
    use crate::extensions_trust::not_now;
    use cineris_protocol::methods::TrustResourceKind;

    let world = World::new("hook_project_trust", []);
    world.user_config(&entry("mine", "turn.end", &this_binary("fail", &[]), ""));
    world.project_config(
        &[
            entry(
                "lint",
                "tool.after",
                &this_binary("report", &["{path}"]),
                "tools = [\"edit\"]",
            ),
            entry("mine", "turn.end", &this_binary("hang", &[]), ""),
        ]
        .concat(),
    );
    let core = world.core();
    // Not now: a project not trusted runs none of its hooks.
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        let none = core
            .run_hook_command(&front.session, "lint", for_edit(&["src/a.rs"]))
            .await;
        assert!(none.is_err(), "{none:?}");
        front.close().await;
    });
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        let [question] = asked.as_slice() else {
            panic!("one question: {asked:?}");
        };
        let hooks: Vec<_> = question
            .resources
            .iter()
            .filter(|resource| resource.kind == TrustResourceKind::Hook)
            .collect();
        let [lint] = hooks.as_slice() else {
            panic!("the project's own hook alone: {:?}", question.resources);
        };
        assert_eq!(lint.name, "lint");
        assert!(lint.detail.contains("on `tool.after`"), "{}", lint.detail);
        assert!(lint.detail.contains("for `edit`"), "{}", lint.detail);
        let refused = core
            .run_hook_command(&front.session, "lint", for_edit(&["src/a.rs"]))
            .await
            .unwrap();
        let HookOutcome::Failed(why) = &refused else {
            panic!("{refused:?}");
        };
        assert!(why.contains("a project's hook never asks"), "{why}");
        front.drain();
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("This project's hook `mine` does not run")),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
    // A session opened again finds it approved as it is: not asked again.
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        let ran = core
            .run_hook_command(&front.session, "lint", for_edit(&["src/a.rs"]))
            .await
            .unwrap();
        assert!(matches!(ran, HookOutcome::Failed(_)), "{ran:?}");
        front.close().await;
    });
}

/// The user's own hook runs where the engine would ask, but not where a
/// rule denies its line, nor where a rule of the managed configuration
/// asks about it: neither starts anything.
#[test]
fn a_users_hook_is_held_back_by_a_deny_or_a_managed_ask() {
    let program = std::env::current_exe().expect("the test binary has a path");
    for (name, managed, user) in [
        (
            "hook_user_denied",
            String::new(),
            format!(
                "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"deny\"\n",
                program.display()
            ),
        ),
        (
            "hook_user_managed_ask",
            format!(
                "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"ask\"\n",
                program.display()
            ),
            String::new(),
        ),
    ] {
        let world = World::new(name, []);
        world.user_config(&format!(
            "{user}{}",
            entry("gate", "tool.before", &this_binary("fail", &[]), "")
        ));
        if !managed.is_empty() {
            let file = world.dirs().managed_config.unwrap();
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, &managed).unwrap();
        }
        let core = world.core();
        world.run(async {
            let front = Front::start(&world, &core).await;
            let held = core
                .run_hook_command(&front.session, "gate", for_edit(&[]))
                .await
                .unwrap();
            let HookOutcome::Failed(why) = &held else {
                panic!("{name}: {held:?}");
            };
            let said = if managed.is_empty() {
                "a permission rule refuses it"
            } else {
                "a rule of the managed configuration"
            };
            assert!(why.contains(said), "{name}: {why}");
            front.close().await;
        });
    }
}

/// An event of more than 1 MiB is no hook's input: the hook has failed,
/// and nothing is started.
#[test]
fn an_event_past_its_cap_starts_nothing() {
    let world = World::new("hook_event_cap", []);
    world.user_config(&entry("gate", "turn.end", &this_binary("fail", &[]), ""));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let ran = core
            .run_hook_command(
                &front.session,
                "gate",
                HookRun {
                    input: json!({"text": "a".repeat(1024 * 1024)}),
                    ..HookRun::default()
                },
            )
            .await
            .unwrap();
        let HookOutcome::Failed(why) = &ran else {
            panic!("{ran:?}");
        };
        assert!(why.contains("a hook is given at most 1 MiB"), "{why}");
        front.close().await;
    });
}

#[cfg(target_os = "linux")]
mod confined {
    //! Where the sandbox enforces: a launcher of the hook's own.

    use super::*;
    use crate::sandbox::{enforcing_here, launcher};

    /// A hook without `write` writes nothing, and fails when it tries; one
    /// with it writes the workspace, and a protected path it changes is
    /// found by the look after it, which stops the turn.
    #[test]
    fn sys_a_hooks_writes_are_confined_and_looked_at() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("hook_confined", []);
        world.put(".github/README", "ci\n");
        world.user_config(
            &[
                entry(
                    "reader",
                    "tool.after",
                    &this_binary("write", &["out.txt"]),
                    "",
                ),
                entry(
                    "scribble",
                    "tool.after",
                    &this_binary("write", &[".github/workflows/x.yml"]),
                    "write = true",
                ),
            ]
            .concat(),
        );
        let core = world.core_with(|options| options.launcher = Some(launcher()));
        world.run(async {
            let front = Front::start(&world, &core).await;
            let after = || HookRun {
                input: json!({"event": "tool.after"}),
                tool: Some("edit".to_owned()),
                ..HookRun::default()
            };
            let reader = core
                .run_hook_command(&front.session, "reader", after())
                .await
                .unwrap();
            let HookOutcome::Failed(why) = &reader else {
                panic!("{reader:?}");
            };
            assert!(why.contains("exited with 4"), "{why}");
            assert!(world.read("out.txt").is_none());
            let scribble = core
                .run_hook_command(&front.session, "scribble", after())
                .await
                .unwrap();
            let HookOutcome::Stopped(note) = &scribble else {
                panic!("{scribble:?}");
            };
            assert!(note.contains(".github/workflows"), "{note}");
            assert!(world.read(".github/workflows/x.yml").is_some());
            // The turn is stopped: no more of its hooks starts.
            let again = core
                .run_hook_command(&front.session, "reader", after())
                .await
                .unwrap();
            assert!(matches!(again, HookOutcome::Stopped(_)), "{again:?}");
            // In the next turn, a protected path changed by hand since the
            // last look refuses the next start that has a write root.
            world.put(".github/workflows/new.yml", "on: push\n");
            let next = core
                .run_hook_command(
                    &front.session,
                    "scribble",
                    HookRun {
                        turn: Some("t_2".to_owned()),
                        ..after()
                    },
                )
                .await
                .unwrap();
            let HookOutcome::Stopped(note) = &next else {
                panic!("{next:?}");
            };
            assert!(note.contains("changed since the last command"), "{note}");
            front.close().await;
        });
    }
}
