//! The shell tool as the core builds it: what a command is given, what
//! becomes of what it changed, and the second interrupt
//! (`docs/architecture/05-tools.md`, "`shell`";
//! `docs/architecture/06-permissions-and-sandbox.md`, "Command analysis as
//! built"). These start a shell, so they are named `sys_…`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// What the test of the second interrupt reads, which needs a command
// that ignores the first: Unix only.
#[cfg(unix)]
use cineris_protocol::event::{Delta, Event, TurnOutcome};
use cineris_protocol::item::ToolResultItem;
use cineris_protocol::methods::{Op, OpResult, SessionArtifactParams, SessionRewindParams};
use cineris_protocol::{Item, Part};
use serde_json::json;

use crate::{Front, World, calls, says};

/// A provider key, made up.
const KEY: &str = "cineris-shell-test-key-77e1";

/// The command that prints the environment, one `NAME=value` a line.
fn print_env() -> &'static str {
    if cfg!(windows) { "cmd /c set" } else { "env" }
}

/// The result of the turn's shell call.
fn shell_result(front: &Front) -> ToolResultItem {
    front
        .items()
        .into_iter()
        .find_map(|item| match item {
            Item::ToolResult(result) if result.tool == "shell" => Some(result),
            _ => None,
        })
        .expect("a shell result")
}

/// What the model is shown of a result.
fn text_of(result: &ToolResultItem) -> String {
    result
        .content
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

/// Everything a command printed, as a front end shows it: the artifact the
/// tool kept the whole output in when the model's copy was cut, where a
/// line longer than the line limit ends with a marker (`… [+1,201
/// characters]`) in place of the rest of it; the model's copy when nothing
/// was cut, which is then the whole output.
async fn whole_output(front: &Front, result: &ToolResultItem) -> String {
    let Some(artifact) = result.artifacts.first() else {
        return text_of(result);
    };
    let read = Op::SessionArtifact(SessionArtifactParams {
        session_id: front.session.clone(),
        artifact: artifact.id.clone(),
    });
    let OpResult::SessionArtifact(whole) = front.op(read).await.expect("the whole output is read")
    else {
        panic!("not the result of session/artifact");
    };
    assert!(
        !whole.cut,
        "the whole output is more than a front end is given"
    );
    whole.text
}

/// How many directories of its own the test puts at the end of the `PATH`
/// it gives, at the least: more than a break that keeps a number of
/// entries keeps, whatever the length of the test's folder.
const OWN_DIRECTORIES: usize = 48;

/// The environment is read from everything the command printed, never
/// from the model's copy of it. The line that holds `PATH` is longer than
/// the model is shown a line, on every machine: the test puts directories
/// of its own at its end until it is. In the model's copy the line is cut,
/// and what follows its last separator is a piece of an entry and the
/// cut's marker. Read from there (until step 25), that piece was taken for
/// an entry, which was absolute or not by where the runner's `PATH` put the
/// cut: after the rename shortened the checkout's path, the cut kept one
/// character of an entry, `C… [+1,201 characters]`, and the test failed on
/// every hosted Windows runner.
#[test]
fn sys_a_command_is_given_the_environment_the_command_analysis_relies_on() {
    let world = World::new(
        "shell_env",
        [
            calls(&[("c1", "shell", json!({"command": print_env()}))]),
            says("Seen."),
        ],
    );
    world.user_config(
        "[providers.mine]\nprotocol = \"anthropic-messages\"\nbase_url = \
         \"https://llm.example.invalid\"\nkey_env = \"MY_PROVIDER_KEY\"\n\n[shell]\nenv_remove = \
         [\"EXTRA_REMOVED\", \"PREFIXED_*\"]\n",
    );
    let separator = if cfg!(windows) { ';' } else { ':' };
    let real_path = std::env::var("PATH").unwrap();
    // Absolute directories of the test's own, at the end of the `PATH` it
    // gives: more characters of them than a line the model is shown, so
    // that the model's copy of the line is cut on every machine, and at
    // least `OWN_DIRECTORIES` of them. They count down, and the last is
    // there twice: a `PATH` put in order, or with a repeat taken out, is not
    // the one the command was given.
    let line_limit = cineris_tools::OutputLimits::default().max_line_chars();
    let mut own: Vec<String> = Vec::new();
    while own.len() < OWN_DIRECTORIES
        || own
            .iter()
            .map(|entry| entry.chars().count() + 1)
            .sum::<usize>()
            <= line_limit
    {
        let entry = world.dir.join(format!("not-a-directory-{:03}", own.len()));
        own.push(entry.to_str().unwrap().to_owned());
    }
    own.reverse();
    own.push(own[own.len() - 1].clone());
    // The user's own `PATH`, with the two kinds of entry that name the
    // directory a command runs in, and the test's own directories.
    let path = format!(
        ".{separator}{real_path}{separator}{separator}bin{separator}{}",
        own.join(&separator.to_string())
    );
    // Whether an entry is an absolute directory, read as the system reads
    // it: on Windows an entry may be in quotes, and on Linux a quote is a
    // character of a name.
    let absolute = |entry: &str| {
        let entry = if cfg!(windows) {
            entry.trim_matches('"')
        } else {
            entry
        };
        Path::new(entry).is_absolute()
    };
    // What the command must be given: the absolute entries of that `PATH`,
    // as they were and in order. A shell may put its own before them, as
    // PowerShell 7 puts its folder.
    let expected: Vec<&str> = path
        .split(separator)
        .filter(|entry| absolute(entry))
        .collect();
    let core = world.core_with(|options| {
        let mut env = world.shell_env(&[
            ("CDPATH", "/somewhere/else"),
            ("OLDPWD", "/somewhere/before"),
            ("RIPGREP_CONFIG_PATH", "/etc/ripgreprc"),
            // The user already gives git a setting through its environment.
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "user.name"),
            ("GIT_CONFIG_VALUE_0", "Dev"),
            ("ANTHROPIC_API_KEY", KEY),
            ("OPENAI_API_KEY", KEY),
            ("OPENROUTER_API_KEY", KEY),
            ("MY_PROVIDER_KEY", KEY),
            ("EXTRA_REMOVED", "gone"),
            ("PREFIXED_ONE", "gone"),
            ("KEPT", "yes"),
        ]);
        env.retain(|(name, _)| !name.eq_ignore_ascii_case("PATH"));
        env.push(("PATH".into(), path.clone().into()));
        options.shell_env = Some(env);
        // The shell the harness picks when nothing is configured, not the
        // rig's cheaper one: this test keeps the product's own pick covered
        // from a turn to the command's output.
        options.shell = crate::default_shell();
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn_allowing("show the environment").await;
        let result = shell_result(&front);
        // The listing reaches the model, with no key in it.
        let shown = text_of(&result);
        assert!(shown.contains("KEPT=yes"), "{shown}");
        assert!(!shown.contains(KEY), "{shown}");
        let text = whole_output(&front, &result).await;
        let env: BTreeMap<String, String> = text
            .lines()
            .filter_map(|line| line.trim_end().split_once('='))
            .map(|(name, value)| (name.to_ascii_uppercase(), value.to_owned()))
            .collect();
        assert_eq!(env.get("KEPT").map(String::as_str), Some("yes"), "{text}");

        // What the analysis takes to be unset is unset.
        assert_eq!(env.get("CDPATH"), None, "{text}");
        assert_eq!(env.get("OLDPWD"), None, "{text}");
        assert_eq!(env.get("RIPGREP_CONFIG_PATH"), None, "{text}");

        // Git's monitor is off, after the setting the user already gives.
        assert_eq!(env.get("GIT_CONFIG_COUNT").map(String::as_str), Some("2"));
        assert_eq!(
            env.get("GIT_CONFIG_KEY_0").map(String::as_str),
            Some("user.name")
        );
        assert_eq!(
            env.get("GIT_CONFIG_VALUE_0").map(String::as_str),
            Some("Dev")
        );
        assert_eq!(
            env.get("GIT_CONFIG_KEY_1").map(String::as_str),
            Some("core.fsmonitor")
        );
        assert_eq!(
            env.get("GIT_CONFIG_VALUE_1").map(String::as_str),
            Some("false")
        );

        // No provider's key: the built-in one's, a configured one's, and
        // what configuration lists.
        assert_eq!(env.get("ANTHROPIC_API_KEY"), None);
        // Every built-in provider's, whichever protocol it speaks.
        assert_eq!(env.get("OPENAI_API_KEY"), None);
        assert_eq!(env.get("OPENROUTER_API_KEY"), None);
        assert_eq!(env.get("MY_PROVIDER_KEY"), None);
        assert_eq!(env.get("EXTRA_REMOVED"), None);
        assert_eq!(env.get("PREFIXED_ONE"), None);
        assert!(!text.contains(KEY), "{text}");

        // `PATH` names only absolute directories, and every one it was
        // given, whole and in order.
        let given = env.get("PATH").expect("a PATH");
        let entries: Vec<&str> = given.split(separator).collect();
        let relative: Vec<(usize, &str)> = entries
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, entry)| !absolute(entry))
            .collect();
        assert!(
            relative.is_empty(),
            "entries of the PATH a command was given that are not absolute, by their place: \
             {relative:?}\nthe PATH: {given}"
        );
        assert!(
            entries.ends_with(&expected),
            "the PATH a command was given does not end with the absolute entries of the one the \
             harness was given, whole and in order: {given}\nthe harness's: {path}"
        );
        // The model's copy of the `PATH` line was cut, so what was checked
        // is the whole output: what this test is for.
        assert!(
            !result.artifacts.is_empty(),
            "the output was not cut, so the PATH was no longer than a line: {shown}"
        );
        front.close().await;
    });
}

/// A command that ignores the polite interrupt sits out a grace that
/// nothing ends in this test. The second interrupt kills it.
#[cfg(unix)]
#[test]
fn sys_a_second_interrupt_kills_a_command_that_ignores_the_first() {
    let world = World::new(
        "kill_now",
        [calls(&[(
            "c1",
            "shell",
            json!({"command": "trap '' INT TERM; echo started; sleep 600"}),
        )])],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let before = front.ends().len();
        front.prompt("run it").await;
        // Allow the command, and wait for it to say it is running.
        let (id, _) = front.approval().await;
        front
            .answer(&id, cineris_protocol::methods::ApprovalChoice::AllowOnce)
            .unwrap();
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, cineris_core::Outgoing::Event(event)
                    if matches!(&event.event, Event::ItemDelta(delta)
                        if matches!(&delta.delta, Delta::ToolOutput(text) if text.text.contains("started"))))
                })
            })
            .await;

        front.interrupt(false).await;
        // The polite stage was sent, and its grace is running. Nobody ends
        // it, and the command does not listen.
        for _ in 0..10_000 {
            if world
                .timer
                .running()
                .contains(&cineris_tools::process::DEFAULT_GRACE)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            world
                .timer
                .running()
                .contains(&cineris_tools::process::DEFAULT_GRACE),
            "the command was not asked to stop"
        );
        front.drain();
        assert_eq!(front.ends().len(), before, "the first interrupt ended the turn");

        front.interrupt(true).await;
        front.turns(before + 1).await;
        assert_eq!(front.ends()[before].outcome, TurnOutcome::Interrupted);
        let payload = front
            .items()
            .into_iter()
            .find_map(|item| match item {
                Item::ToolResult(result) if result.tool == "shell" => result.payload,
                _ => None,
            })
            .expect("the command has a result");
        assert_eq!(payload["stopped_with"], "kill");
        assert_eq!(payload["interrupted"], true);
        front.close().await;
    });
}

fn git(dir: &Path, arguments: &[&str]) -> bool {
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(arguments)
        .current_dir(dir)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn sys_undo_puts_back_what_a_command_changed_where_git_still_has_it() {
    let world = World::new(
        "shell_undo",
        [
            says("Ready."),
            calls(&[(
                "c1",
                "shell",
                json!({"command": "echo changed > tracked.txt; echo new > created.txt"}),
            )]),
            says("Changed."),
        ],
    );
    world.put("tracked.txt", "committed\n");
    world.put("dirty.txt", "committed\n");
    if !(git(&world.workspace, &["init", "-q"])
        && git(&world.workspace, &["add", "."])
        && git(&world.workspace, &["commit", "-q", "-m", "first"]))
    {
        return;
    }
    let committed = std::fs::read(world.workspace.join("tracked.txt")).unwrap();
    let core = world.core_with(|options| {
        options.trust_project = true;
        // The shell is looked for, as in the real program, and the tool's
        // description names it. Its version is asked on the side and kept
        // for the runs after this one (`shell_cache`; the binary's system
        // test `sys_the_shells_version_is_kept_for_the_runs_after_the_first`
        // shows the next run naming it).
        options.shell = None;
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        let description = world.provider.requests()[0]
            .tools
            .iter()
            .find(|tool| tool.name == "shell")
            .map(|tool| tool.description.clone())
            .expect("a shell tool");
        assert!(
            ["bash", "sh", "PowerShell", "cmd"]
                .iter()
                .any(|shell| description.contains(shell)),
            "the shell is not named in the tool's description: {description}"
        );
        let keep = front
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::AssistantMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        front.turn_allowing("change things").await;
        assert_ne!(
            std::fs::read(world.workspace.join("tracked.txt")).unwrap(),
            committed
        );
        assert!(world.workspace.join("created.txt").exists());

        // The round's checkpoint: the tracked file with the content git
        // has, and the new file as one that did not exist.
        let log = front.log(&world);
        let checkpoint = log
            .iter()
            .find(|line| line["kind"] == "checkpoint")
            .expect("the command's round has a checkpoint");
        let files = checkpoint["files"].as_array().unwrap();
        let file = |path: &str| files.iter().find(|file| file["path"] == path).unwrap();
        assert!(file("tracked.txt")["before"].is_object(), "{checkpoint}");
        assert!(file("created.txt")["before"].is_null(), "{checkpoint}");

        front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: keep,
                restore_files: true,
            }))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(world.workspace.join("tracked.txt")).unwrap(),
            committed,
            "the file git still had was not put back"
        );
        assert!(
            !world.workspace.join("created.txt").exists(),
            "the file the command created is still there"
        );
        front.close().await;
    });
}

/// A shell keeps an inherited `PWD` that names the directory it starts
/// in, by whatever path, and `cd ..` then goes to the parent of that name.
/// For a workspace reached through a link that is not where the command
/// analysis followed the `cd` to, so every command is told the directory
/// as it was resolved.
#[cfg(unix)]
#[test]
fn sys_a_command_in_a_workspace_behind_a_link_starts_where_it_really_is() {
    let world = World::new(
        "shell_pwd",
        [
            calls(&[("c1", "shell", json!({"command": "cd .. && pwd"}))]),
            says("Seen."),
        ],
    );
    // The workspace is moved somewhere deeper, and a link is left where it
    // was: the harness is started in the link, as a user's shell would be.
    let link = world.workspace.clone();
    let real = world.dir.join("elsewhere").join("deep").join("app");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::rename(&link, &real).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let real_parent = std::fs::canonicalize(real.parent().unwrap()).unwrap();

    let core = world.core_with(|options| {
        options.shell_env = Some(world.shell_env(&[("PWD", link.to_str().unwrap())]));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn_allowing("where is the parent").await;
        let text = text_of(&shell_result(&front));
        assert_eq!(
            text.lines().next().map(str::trim),
            real_parent.to_str(),
            "the command went to the parent of the link: {text}"
        );
        front.close().await;
    });
}
