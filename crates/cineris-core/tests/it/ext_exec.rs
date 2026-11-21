//! An extension's `cineris.exec`, end to end (`docs/steps/28/answered.md`,
//! "The processes"): the model calls an extension's tool through
//! `tool_call`, its script asks the host to run a program, and the core
//! finds it, has it decided, confines it where the sandbox enforces, looks
//! at the protected paths around it, and runs it with a shell command's
//! environment. The tests that start a program are named `sys_…`.
//!
//! The program is mostly this test binary itself, entered as
//! [`crate::ECHO`]: it reports its arguments, its input and the variables
//! it must not see, on every system.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_core::ext::ApprovedExtension;
use cineris_protocol::methods::{ApprovalChoice, ApprovalDetail};
use serde_json::{Value, json};

use crate::{Answering, ECHO, ECHO_MARK, Front, World, calls, says};

/// The script of every test's extension: a tool `run` that runs what its
/// arguments say and gives back what the host answered, or why it refused.
const SCRIPT: &str = "cineris.tool({ name: 'run', description: 'runs a program',\n\
   input: { type: 'object', required: ['program'], additionalProperties: false, properties: {\n\
     program: { type: 'string' }, args: { type: 'array', items: { type: 'string' } },\n\
     cwd: { type: 'string' }, stdin: { type: 'string' }, timeout: { type: 'integer' } } },\n\
   run: async (a) => {\n\
     try {\n\
       const r = await cineris.exec(a.program, a.args || [],\n\
         { cwd: a.cwd, stdin: a.stdin, timeout: a.timeout });\n\
       return 'ran: ' + JSON.stringify(r);\n\
     } catch (e) { return 'refused: ' + String(e); }\n\
   } });\n";

/// An approved extension called `name` with these capabilities, written to
/// a folder of the world's own outside the workspace.
fn extension(world: &World, name: &str, capabilities: &str) -> ApprovedExtension {
    extension_running(world, name, capabilities, SCRIPT)
}

/// [`extension`] with `script` as its main module.
fn extension_running(
    world: &World,
    name: &str,
    capabilities: &str,
    script: &str,
) -> ApprovedExtension {
    let folder = world.dir.join("extensions").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"A test \
             extension\"\nmain = \"main.js\"\n\n[capabilities]\n{capabilities}\n"
        ),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), script).unwrap();
    let snapshot = Arc::new(cineris_ext::folder::Snapshot::read(&folder).unwrap());
    let hash = snapshot.hash();
    let (manifest, registration) = world
        .run(cineris_ext::engine::register(snapshot, &world.timer))
        .unwrap();
    ApprovedExtension {
        source: cineris_ext::source::Source::User,
        folder,
        hash,
        manifest,
        registration,
    }
}

/// A TOML literal string of `text`: a path, backslashes and all.
fn literal(text: &str) -> String {
    format!("'{text}'")
}

/// This test binary, which [`ECHO`] makes a program that says what it was
/// given.
fn echo_program() -> PathBuf {
    std::env::current_exe().expect("the test binary has a path")
}

/// Puts this binary in `dir`, outside the workspace, as `name` (with
/// `.exe` on Windows): a link to it on Unix, a hard link or a copy on
/// Windows. Returns its path.
fn place_echo(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    #[cfg(unix)]
    {
        let placed = dir.join(name);
        std::os::unix::fs::symlink(echo_program(), &placed).unwrap();
        placed
    }
    #[cfg(windows)]
    {
        let placed = dir.join(format!("{name}.exe"));
        if std::fs::hard_link(echo_program(), &placed).is_err() {
            std::fs::copy(echo_program(), &placed).unwrap();
        }
        placed
    }
}

/// The environment of the world's programs with `folders` first on
/// `PATH`, and `extra`.
fn path_with(
    world: &World,
    folders: &[&Path],
    extra: &[(&str, &str)],
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let base = world.shell_env(&[]);
    let inherited = base
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let mut entries: Vec<PathBuf> = folders.iter().map(|one| one.to_path_buf()).collect();
    entries.extend(std::env::split_paths(&inherited));
    let path = std::env::join_paths(entries).unwrap();
    let mut all = vec![("PATH", path.to_str().unwrap())];
    all.extend_from_slice(extra);
    world.shell_env(&all)
}

/// The echo on `PATH` as `probe-echo`, outside the workspace, and the
/// environment that finds it there.
fn echo_on_path(world: &World) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let dir = world.dir.join("tools");
    place_echo(&dir, "probe-echo");
    path_with(world, &[&dir], &[])
}

/// The arguments that make this binary the echo, before `given`.
fn echo_args(given: &[&str]) -> Vec<String> {
    ["--exact", "launcher_entry", "--nocapture", ECHO, "--"]
        .iter()
        .chain(given)
        .map(|one| (*one).to_owned())
        .collect()
}

/// A `tool_call` of `extension`'s `run` with `arguments`.
fn run_call(id: &str, extension: &str, arguments: &Value) -> cineris_testkit::Response {
    calls(&[(
        id,
        "tool_call",
        json!({"tool": format!("{extension}/run"), "arguments": arguments}),
    )])
}

/// What the script answered in the call's result: `ran: {…}` or
/// `refused: …`, after the line that labels it.
fn answered(front: &Front, index: usize) -> String {
    let results = front.results();
    let text = &results[index].2;
    text.split_once('\n')
        .map_or_else(|| text.clone(), |(_, rest)| rest.to_owned())
}

/// The host's answer to a program that ran.
fn ran(answer: &str) -> Value {
    let json = answer
        .strip_prefix("ran: ")
        .unwrap_or_else(|| panic!("the program did not run: {answer}"));
    serde_json::from_str(json).unwrap()
}

/// What the echo reported, from its standard output.
fn report(ran: &Value) -> Value {
    let stdout = ran["stdout"].as_str().expect("its output");
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(ECHO_MARK))
        .unwrap_or_else(|| panic!("no report in {stdout:?}"));
    serde_json::from_str(line).unwrap()
}

/// The program runs with its arguments one by one and its input; its
/// standard output and standard error come back apart, with its exit code.
#[test]
fn sys_a_program_runs_with_its_arguments_and_input_and_its_streams_apart() {
    let world = World::new("exec_runs", []);
    let program = echo_program();
    let probe = extension(
        &world,
        "probe",
        &format!("exec = [{}]", literal(&program.to_string_lossy())),
    );
    let mut args = echo_args(&["a b", "it's", "x&y", "--flag"]);
    args.push(String::new());
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": program, "args": args, "stdin": "line one\nline two"}),
    ));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.extensions = vec![probe]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("run it").await;
        let ran = ran(&answered(&front, 0));
        assert_eq!(ran["code"], 7, "{ran}");
        assert_eq!(ran["timedOut"], false);
        assert!(
            ran["stderr"]
                .as_str()
                .is_some_and(|err| err.contains("echo: to standard error")),
            "{ran}"
        );
        let report = report(&ran);
        assert_eq!(report["args"], json!(["a b", "it's", "x&y", "--flag", ""]));
        assert_eq!(report["stdin"], "line one\nline two");
        front.close().await;
    });
}

/// A provider's key, and a variable `shell.env_remove` names, do not reach
/// the program; another variable does.
#[test]
fn sys_a_provider_key_and_a_removed_variable_do_not_reach_the_program() {
    let world = World::new("exec_env", []);
    world.user_config("[shell]\nenv_remove = [\"PROBE_REMOVED\"]\n");
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[])}),
    ));
    world.provider.push(says("Done."));
    let dir = world.dir.join("tools");
    place_echo(&dir, "probe-echo");
    let env = path_with(
        &world,
        &[&dir],
        &[
            ("ANTHROPIC_API_KEY", "sk-test-not-for-programs"),
            ("PROBE_REMOVED", "removed"),
            ("PROBE_KEPT", "kept"),
        ],
    );
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("run it").await;
        let report = report(&ran(&answered(&front, 0)));
        assert_eq!(report["key"], Value::Null, "{report}");
        assert_eq!(report["removed"], Value::Null, "{report}");
        assert_eq!(report["kept"], "kept", "{report}");
        front.close().await;
    });
}

/// A program's own `PATH` holds no folder of the workspace, though the
/// harness's had one (an activated `.venv/bin`, direnv's `node_modules/.bin`):
/// a wrapper outside it (`#!/usr/bin/env node`) would otherwise find a
/// program a command planted there (the early review of step 28).
#[test]
fn sys_a_programs_own_path_holds_no_folder_of_the_workspace() {
    let world = World::new("exec_path", []);
    let planted = world.workspace.join("node_modules").join(".bin");
    std::fs::create_dir_all(&planted).unwrap();
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[])}),
    ));
    world.provider.push(says("Done."));
    let dir = world.dir.join("tools");
    place_echo(&dir, "probe-echo");
    let env = path_with(&world, &[&planted, &dir], &[("PROBE_PATH", "1")]);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("run it").await;
        let report = report(&ran(&answered(&front, 0)));
        let names = report["path"].as_array().expect("the folders of PATH");
        assert!(!names.contains(&json!(".bin")), "{report}");
        assert!(names.contains(&json!("tools")), "{report}");
        front.close().await;
    });
}

/// A program named by its bare name is never one in the workspace, nor one
/// behind a relative entry of `PATH`: neither is found, and nothing runs.
#[test]
fn a_program_in_the_workspace_or_behind_a_relative_path_entry_is_not_found() {
    let world = World::new("exec_lookup", []);
    let name = if cfg!(windows) {
        "planted.cmd"
    } else {
        "planted"
    };
    let planted = world.workspace.join("bin").join(name);
    std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
    std::fs::write(&planted, "echo ran > ran.txt\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let probe = extension(&world, "probe", "exec = [\"planted\"]");
    world
        .provider
        .push(run_call("c1", "probe", &json!({"program": "planted"})));
    world.provider.push(says("Done."));
    // The workspace's `bin` on `PATH` twice: as an absolute entry, and as
    // a relative one that names it from where the session starts.
    let path = std::env::join_paths([world.workspace.join("bin"), PathBuf::from("bin")]).unwrap();
    let env = world.shell_env(&[("PATH", path.to_str().unwrap())]);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("run it").await;
        let answer = answered(&front, 0);
        assert!(
            answer.starts_with("refused:") && answer.contains("`planted` was not found"),
            "{answer}"
        );
        assert!(world.read("ran.txt").is_none());
        front.close().await;
    });
}

/// A rule that denies the line refuses the program; nothing is asked and
/// nothing runs.
#[test]
fn a_program_a_rule_denies_is_refused() {
    let world = World::new("exec_denied", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \
         \"allow\"\n\n[[permissions.rules]]\neffect = \"exec\"\nmatch = \"probe-echo *\"\naction = \
         \"deny\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[])}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("run it").await;
        assert!(front.approvals().is_empty(), "{:?}", front.approvals());
        let answer = answered(&front, 0);
        assert!(
            answer.starts_with("refused:") && answer.contains("not allowed"),
            "{answer}"
        );
        front.close().await;
    });
}

/// A rule that allows the line runs the program without a question.
#[test]
fn sys_a_program_an_exec_rule_allows_runs_unasked() {
    let world = World::new("exec_allowed", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \
         \"allow\"\n\n[[permissions.rules]]\neffect = \"exec\"\nmatch = \"probe-echo *\"\naction = \
         \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&["ok"])}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("run it").await;
        assert!(front.approvals().is_empty(), "{:?}", front.approvals());
        let report = report(&ran(&answered(&front, 0)));
        assert_eq!(report["args"], json!(["ok"]));
        front.close().await;
    });
}

/// Where the engine asks, the question shows the line it read, each
/// argument quoted in the session shell's syntax, and offers only "allow
/// once" and "deny"; denied, nothing runs.
#[test]
fn a_question_about_a_program_shows_the_quoted_line_and_offers_only_once_or_deny() {
    let world = World::new("exec_asked", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": ["a b", "it's"]}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        let asked = front.turn_denying("run it").await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        let approval = &front.approvals()[0];
        assert_eq!(
            approval.choices,
            [ApprovalChoice::AllowOnce, ApprovalChoice::Deny]
        );
        let ApprovalDetail::Command(detail) = &approval.detail else {
            panic!("not a command: {:?}", approval.detail);
        };
        let quoted = if cfg!(windows) {
            "probe-echo 'a b' 'it''s'"
        } else {
            r"probe-echo 'a b' 'it'\''s'"
        };
        assert_eq!(detail.command, quoted);
        let answer = answered(&front, 0);
        assert!(answer.starts_with("refused:"), "{answer}");
        front.close().await;
    });
}

/// The engine judges a program's line, its arguments, and not its input,
/// which the script gives: a question about a program given input says how
/// long it is and how it begins, at most three lines, with every character
/// that acts unseen escaped (the review of the extensions' process starts).
#[test]
fn a_question_about_a_program_given_input_shows_how_the_input_begins() {
    let world = World::new("exec_input_shown", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    let input = "first\u{1b}[2J line\nsecond line\nthird line\nfourth line\n";
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": ["x"], "stdin": input}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        let asked = front.turn_denying("run it").await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        let approval = &front.approvals()[0];
        let shown = approval
            .reasons
            .iter()
            .find(|reason| reason.contains("bytes of input"))
            .unwrap_or_else(|| panic!("the input is not shown: {approval:?}"));
        assert!(
            shown.contains(&format!("given {} bytes of input", input.len())),
            "{shown}"
        );
        assert!(
            shown.contains(r"first\u{1b}[2J line\nsecond line\nthird line"),
            "{shown}"
        );
        assert!(
            !shown.contains("fourth") && !shown.contains('\u{1b}'),
            "{shown}"
        );
        assert!(answered(&front, 0).starts_with("refused:"));
        front.close().await;
    });
}

/// An interrupt while a program's question is open withdraws the question,
/// and the turn ends interrupted: the engine follows a program's host call
/// to its end, and its decision is given up with the call (the review of
/// step 28's fixes: the turn waited for an answer nobody would give).
#[test]
fn an_interrupt_withdraws_a_programs_open_question() {
    use cineris_protocol::event::{Event, Resolution, TurnOutcome};

    let world = World::new("exec_interrupt", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[])}),
    ));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("run it").await;
        let (_, request) = front.approval().await;
        assert_eq!(request.tool, "probe/run");
        front.interrupt(false).await;
        front.turns(1).await;
        assert_eq!(front.ends()[0].outcome, TurnOutcome::Interrupted);
        let resolutions: Vec<Resolution> = front
            .events()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::RequestResolved(resolved) => Some(resolved.resolution),
                _ => None,
            })
            .collect();
        assert_eq!(resolutions, [Resolution::Cancelled]);
        front.close().await;
    });
}

/// A program's question still open when the call's time is up is
/// withdrawn, and the call ends out of its time, not when a person
/// answers.
#[test]
fn a_programs_open_question_is_withdrawn_when_the_calls_time_is_up() {
    use cineris_protocol::event::{Event, Resolution};
    use std::time::Duration;

    let world = World::new("exec_question_time", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(
        &world,
        "probe",
        "exec = [\"probe-echo\"]\n\n[limits]\ntime = 5",
    );
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[])}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("run it").await;
        let (_, request) = front.approval().await;
        assert_eq!(request.tool, "probe/run");
        world.timer.sleeping_for(Duration::from_secs(5)).await;
        world.timer.advance(Duration::from_secs(5));
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("ran out of its time"), "{results:?}");
        let resolutions: Vec<Resolution> = front
            .events()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::RequestResolved(resolved) => Some(resolved.resolution),
                _ => None,
            })
            .collect();
        assert_eq!(resolutions.len(), 1, "{resolutions:?}");
        front.close().await;
    });
}

/// In a PowerShell session a program's line is judged by PowerShell's own
/// commands and aliases, which are not the program that runs: `write` is
/// `Write-Output` there, read-only, and the program is not. What that
/// shell calls read-only is asked about (the early review of step 28).
#[test]
fn sys_in_powershell_a_program_the_shell_calls_read_only_is_asked_about() {
    let world = World::new("exec_pwsh_alias", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"write\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "write", "args": echo_args(&[])}),
    ));
    world.provider.push(says("Done."));
    let dir = world.dir.join("tools");
    place_echo(&dir, "write");
    let env = path_with(&world, &[&dir], &[]);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
        options.shell = Some(cineris_tools::Shell::of_kind(
            cineris_tools::ShellKind::Pwsh,
            "pwsh",
        ));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        let asked = front.turn_denying("run it").await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        let approval = &front.approvals()[0];
        assert!(
            approval
                .reasons
                .iter()
                .any(|reason| reason.contains("whose own commands and aliases")),
            "{approval:?}"
        );
        assert_eq!(
            approval.choices,
            [ApprovalChoice::AllowOnce, ApprovalChoice::Deny]
        );
        let answer = answered(&front, 0);
        assert!(answer.starts_with("refused:"), "{answer}");
        front.close().await;
    });
}

/// With `exec_network` the program is decided as reaching the network
/// too: a session whose rules allow the line but say nothing of the
/// network asks about it.
#[test]
fn a_program_with_exec_network_is_decided_as_reaching_the_network() {
    let world = World::new("exec_network_decided", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \
         \"allow\"\n\n[[permissions.rules]]\neffect = \"exec\"\nmatch = \"probe-echo *\"\naction = \
         \"allow\"\n",
    );
    let probe = extension(
        &world,
        "probe",
        "exec = [\"probe-echo\"]\nexec_network = true",
    );
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[])}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        let asked = front.turn_denying("run it").await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        let approval = &front.approvals()[0];
        assert!(
            approval
                .reasons
                .iter()
                .any(|reason| reason.contains("network")),
            "{approval:?}"
        );
        assert!(answered(&front, 0).starts_with("refused:"));
        front.close().await;
    });
}

/// A program runs inside the workspace: a `cwd` that leads out of it is
/// refused before anything is asked or started.
#[test]
fn a_program_is_not_run_outside_the_workspace() {
    let world = World::new("exec_cwd", []);
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": echo_args(&[]), "cwd": ".."}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("run it").await;
        let answer = answered(&front, 0);
        assert!(
            answer.starts_with("refused:") && answer.contains("outside the workspace"),
            "{answer}"
        );
        // The only question was the extension's tool itself.
        assert_eq!(front.approvals().len(), 1, "{:?}", front.approvals());
        front.close().await;
    });
}

/// What a program changed in the workspace is in the round's checkpoint, as
/// a shell command's is: undo puts a tracked file back from git and takes
/// away a file it made.
#[cfg(unix)]
#[test]
fn sys_undo_puts_back_what_a_program_changed() {
    use cineris_protocol::Item;
    use cineris_protocol::methods::{Op, SessionRewindParams};
    use cineris_testkit::scratch::git;

    let world = World::new("exec_undo", [says("Ready.")]);
    world.put("tracked.txt", "committed\n");
    git(
        &world.workspace,
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(&world.workspace, &["add", "--all"]);
    git(
        &world.workspace,
        &["commit", "--quiet", "--message", "first"],
    );
    let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\"**\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "sh", "args": ["-c", "echo changed > tracked.txt; echo new > created.txt"]}),
    ));
    world.provider.push(says("Changed."));
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.trust_project = true;
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        let keep = front
            .items()
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::AssistantMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        front.answering(Answering::allow_all());
        front.turn("change things").await;
        assert_eq!(world.read("tracked.txt").as_deref(), Some("changed\n"));
        assert_eq!(world.read("created.txt").as_deref(), Some("new\n"));
        front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: Some(keep),
                before: None,
                restore_files: true,
                plan: None,
            }))
            .await
            .unwrap();
        assert_eq!(
            world.read("tracked.txt").as_deref(),
            Some("committed\n"),
            "the file git still had was not put back"
        );
        assert!(
            world.read("created.txt").is_none(),
            "the file the program made is still there"
        );
        front.close().await;
    });
}

/// The decision may wait on a person: a program decided late is given only
/// what is left of the call's time, never the time worked out when the
/// script asked (the review of the extensions' process starts). Here the
/// call has its 120 seconds, a program 30 unless it says otherwise, and the
/// person takes 100 to answer: the program has 20.
#[cfg(unix)]
#[test]
fn sys_a_program_decided_late_is_given_only_what_is_left_of_the_calls_time() {
    use std::time::Duration;

    let world = World::new("exec_time_left", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"sh\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "sh", "args": ["-c", "exec sleep 600"]}),
    ));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.extensions = vec![probe]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("run it").await;
        let (id, request) = front.approval().await;
        assert!(
            matches!(request.detail, ApprovalDetail::Command(_)),
            "not the program's question: {request:?}"
        );
        world.timer.advance(Duration::from_secs(100));
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let mut left = std::pin::pin!(world.timer.sleeping_for(Duration::from_secs(20)));
        let mut asked = std::pin::pin!(
            world
                .timer
                .sleeping_for(cineris_ext::host::EXEC_DEFAULT_TIMEOUT)
        );
        std::future::poll_fn(|context| {
            assert!(
                asked.as_mut().poll(context).is_pending(),
                "the program was given the time worked out before the question"
            );
            left.as_mut().poll(context)
        })
        .await;
        world.timer.advance(Duration::from_secs(20));
        front.turns(1).await;
        front.close().await;
    });
}

/// A file a program changed is not overwritten by the model's write from
/// what it read before: the model reads it again first, as after a shell
/// command's change.
#[cfg(unix)]
#[test]
fn sys_a_file_a_program_changed_is_not_overwritten_from_an_older_read() {
    let world = World::new(
        "exec_noted",
        [
            calls(&[("c1", "read", json!({"path": "notes.txt"}))]),
            says("Read."),
        ],
    );
    world.put("notes.txt", "first\n");
    let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\"**\"]");
    world.provider.push(run_call(
        "c2",
        "probe",
        &json!({"program": "sh", "args": ["-c", "echo program > notes.txt"]}),
    ));
    world.provider.push(calls(&[(
        "c3",
        "write",
        json!({"path": "notes.txt", "content": "model\n"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("read the notes").await;
        front.turn("change them").await;
        let results = front.results();
        let write = results
            .iter()
            .find(|result| result.0 == "write")
            .expect("the write's result");
        assert_ne!(write.1, "success", "{results:?}");
        front.close().await;
    });
    assert_eq!(world.read("notes.txt").as_deref(), Some("program\n"));
}

/// A manifest whose write roots hold the harness's own files (here its
/// configuration, kept inside the workspace) runs no program, before
/// anyone is asked: a program could change what it is allowed. One whose
/// write root holds none of them runs. On Windows too, where the roots
/// were judged in the file system's form (`\\?\C:\…`), which the judgement
/// could not read and passed over (the review of the extensions' process
/// starts).
#[test]
fn sys_a_write_root_that_holds_the_harness_files_runs_nothing() {
    let mut world = World::new("exec_harness_files", []);
    let config = world.workspace.join(".settings");
    std::fs::create_dir_all(config.join("cineris")).unwrap();
    // Where the configuration folder is, on either system
    // (`cineris-config`'s `Dirs::from_env`).
    world.env = world
        .env
        .clone()
        .with("XDG_CONFIG_HOME", config.to_str().unwrap())
        .with("APPDATA", config.to_str().unwrap());
    world.user_config("");
    std::fs::create_dir_all(world.workspace.join("out")).unwrap();
    let wide = extension(&world, "wide", "exec = [\"probe-echo\"]\nwrite = [\"**\"]");
    let narrow = extension(
        &world,
        "narrow",
        "exec = [\"probe-echo\"]\nwrite = [\"out/**\"]",
    );
    let arguments = json!({"program": "probe-echo", "args": echo_args(&["ok"])});
    world.provider.push(run_call("c1", "wide", &arguments));
    world.provider.push(run_call("c2", "narrow", &arguments));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![wide, narrow];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(Answering::allow_all());
        front.turn("run both").await;
        let answer = answered(&front, 0);
        assert!(answer.contains("harness's own files"), "{answer}");
        let report = report(&ran(&answered(&front, 1)));
        assert_eq!(report["args"], json!(["ok"]), "{report}");
        // The two tools, and the narrow one's program, were asked about;
        // the wide one's program never was.
        assert_eq!(front.approvals().len(), 3, "{:?}", front.approvals());
        front.close().await;
    });
}

/// An argument holding a character that a Windows code page's best fit
/// makes a `"` (here U+FF02) is refused on every system, naming the
/// argument and the character, before anything is asked or started.
#[test]
fn a_lookalike_of_a_quote_is_refused_before_anything_is_asked() {
    let world = World::new("exec_lookalike", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let probe = extension(&world, "probe", "exec = [\"probe-echo\"]");
    world.provider.push(run_call(
        "c1",
        "probe",
        &json!({"program": "probe-echo", "args": ["ok", "a\u{FF02} --flag"]}),
    ));
    world.provider.push(says("Done."));
    let env = echo_on_path(&world);
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("run it").await;
        assert!(front.approvals().is_empty(), "{:?}", front.approvals());
        let answer = answered(&front, 0);
        assert!(
            answer.starts_with("refused:")
                && answer.contains("argument 2")
                && answer.contains("a fullwidth quotation mark (U+FF02)"),
            "{answer}"
        );
        front.close().await;
    });
}

#[cfg(target_os = "linux")]
mod confined {
    //! Where the sandbox enforces: a launcher of the extension's own.

    use super::*;
    use crate::sandbox::{enforcing_here, launcher};

    /// A session at `level`, its tool calls all allowed.
    async fn front_at(
        world: &World,
        core: &cineris_core::Core,
        level: cineris_protocol::lease::Level,
    ) -> Front {
        let front = Front::start_as(
            world,
            core,
            cineris_core::ClientInfo::user(),
            cineris_protocol::session::SessionSettings {
                level: Some(level),
                ..cineris_protocol::session::SessionSettings::default()
            },
        )
        .await
        .expect("the session starts");
        front.answering(Answering::allow_all());
        front
    }

    /// A program writes inside its manifest's write root and in the
    /// session's temporary directory, and nowhere else: not the rest of
    /// the workspace, and not outside it. It is confined so at
    /// `unrestricted` too, where the shell's commands are not; and its
    /// input reaches it through the launcher.
    #[test]
    fn sys_a_program_writes_only_its_write_roots_and_the_temporary_directory() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_confined", []);
        std::fs::create_dir_all(world.workspace.join("out")).unwrap();
        let outside = world.dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\"out/**\"]");
        let script = format!(
            "cat > out/stdin.txt; echo in > out/in.txt; echo root > root.txt; echo far > \
             '{}/far.txt'; echo t > \"$TMPDIR/t\" && cat \"$TMPDIR/t\" > out/from-tmp.txt; echo \
             done",
            outside.display()
        );
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "sh", "args": ["-c", script], "stdin": "piped in\n"}),
        ));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Unrestricted).await;
            front.turn("run it").await;
            let ran = ran(&answered(&front, 0));
            assert!(ran["stdout"].as_str().unwrap().contains("done"), "{ran}");
            assert_eq!(world.read("out/stdin.txt").as_deref(), Some("piped in\n"));
            assert_eq!(world.read("out/in.txt").as_deref(), Some("in\n"));
            assert_eq!(world.read("out/from-tmp.txt").as_deref(), Some("t\n"));
            assert!(world.read("root.txt").is_none(), "{ran}");
            assert!(!outside.join("far.txt").exists(), "{ran}");
            front.close().await;
        });
    }

    /// A write root that is a link grants nothing where the link leads: the
    /// root is handed to the launcher as the workspace's real path joined
    /// with its names, never resolved, and the launcher refuses a link on
    /// the way (the early review of step 28: `build -> ~` granted writing
    /// the home folder). A confined command can make such a link unasked,
    /// and a cloned repository can hold one.
    #[test]
    fn sys_a_write_root_that_is_a_link_grants_nothing_where_it_leads() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_root_link", []);
        let outside = world.dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, world.workspace.join("out")).unwrap();
        let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\"out/**\"]");
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "sh", "args": ["-c", "echo x > out/x; echo done"]}),
        ));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Supervised).await;
            front.turn("run it").await;
            let answer = answered(&front, 0);
            assert!(!answer.contains("done"), "{answer}");
            front.close().await;
        });
        assert!(!outside.join("x").exists(), "the link's target was written");
    }

    /// A folder a reload makes writable is passed over by the next search
    /// for a program: a confined command could have planted one there
    /// (the early review of step 28: the folders avoided were the ones the
    /// session started with).
    #[test]
    fn sys_a_folder_a_reload_makes_writable_is_passed_over_by_the_search() {
        use std::os::unix::fs::PermissionsExt as _;

        use cineris_protocol::methods::{ConfigReloadParams, Op, OpResult};

        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_reload_avoid", []);
        let script = |dir: &Path, says: &str| {
            std::fs::create_dir_all(dir).unwrap();
            let path = dir.join("probe-sh");
            std::fs::write(&path, format!("#!/bin/sh\necho {says}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let planted = world.dir.join("planted");
        let real = world.dir.join("real");
        script(&planted, "planted");
        script(&real, "real");
        let probe = extension(&world, "probe", "exec = [\"probe-sh\"]");
        world
            .provider
            .push(run_call("c1", "probe", &json!({"program": "probe-sh"})));
        world.provider.push(says("Done."));
        let env = path_with(&world, &[&planted, &real], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
            options.shell_env = Some(env);
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Supervised).await;
            world.user_config(&format!("[sandbox]\nwrite = [\"{}\"]\n", planted.display()));
            let reloaded = front
                .op(Op::ConfigReload(ConfigReloadParams::default()))
                .await
                .expect("the reload answers");
            let OpResult::ConfigReload(reloaded) = reloaded else {
                panic!("not the result of config/reload");
            };
            assert!(reloaded.errors.is_empty(), "{reloaded:?}");
            front.turn("run it").await;
            let ran = ran(&answered(&front, 0));
            let stdout = ran["stdout"].as_str().unwrap();
            assert!(
                stdout.contains("real") && !stdout.contains("planted"),
                "{ran}"
            );
            front.close().await;
        });
    }

    /// A supervisor's "allow once" of a program allows that start alone:
    /// the next start of the same program in the same tool call is put
    /// again, to the supervisor and the user (the review of step 28: a
    /// host call's allow-once was remembered under the tool call's item,
    /// and allowed the next identical one unasked; a supervisor's answer is
    /// kept no more than a person's).
    #[test]
    fn sys_a_supervisors_allow_once_of_a_program_allows_that_start_alone() {
        use cineris_protocol::lease::{
            Budget, Effect, HardLimits, Level, NetworkAccess, PathSet, Rule, RuleAction,
            SandboxPolicy,
        };
        use cineris_protocol::methods::{ClientRole, Op, OpResult, SessionRef};

        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_supervised_once", []);
        world.user_config(
            "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
        );
        let probe = extension_running(
            &world,
            "probe",
            "exec = [\"make\"]",
            "cineris.tool({ name: 'twice', input: { type: 'object', properties: { args: { type: 'array', \
               items: { type: 'string' } } } }, run: async (a) => { \
               await cineris.exec('make', a.args); await cineris.exec('make', a.args); \
               return 'ran twice' } });",
        );
        world.provider.push(calls(&[(
            "c1",
            "tool_call",
            json!({"tool": "probe/twice", "arguments": {"args": echo_args(&[])}}),
        )]));
        world.provider.push(says("Done."));
        let dir = world.dir.join("tools");
        place_echo(&dir, "make");
        let env = path_with(&world, &[&dir], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
            options.sandbox = Some(cineris_sandbox::support::Support::Landlock(8));
            options.shell_env = Some(env);
        });
        // The supervisor's grant: the program `make`, where commands run
        // confined.
        let everything = PathSet {
            all: true,
            roots: Vec::new(),
        };
        let grant = cineris_policy::standing_lease(
            cineris_protocol::ids::LeaseId::new("l_test"),
            cineris_policy::StandingTerms {
                level: Level::Supervised,
                rules: vec![Rule {
                    effect: Effect::Exec,
                    pattern: "make *".to_owned(),
                    action: RuleAction::Allow,
                }],
                sandbox: SandboxPolicy {
                    read: everything.clone(),
                    write: everything,
                    protect: PathSet::default(),
                    network: NetworkAccess::Off,
                },
                models: Vec::new(),
                budget: Budget::default(),
                limits: HardLimits::default(),
            },
            cineris_policy::SandboxState::Enforcing,
        )
        .expect("a lease");
        world.run(async {
            let front = Front::start(&world, &core).await;
            let client = front.core.connect_with_grant(
                cineris_core::ClientInfo {
                    role: ClientRole::Supervisor,
                    ..cineris_core::ClientInfo::user()
                },
                Some(grant),
            );
            let opened = front
                .core
                .handle(
                    client.id,
                    Op::SessionOpen(SessionRef {
                        session_id: front.session.clone(),
                    }),
                )
                .await
                .expect("the supervisor opens the session");
            let OpResult::SessionOpen(opened) = opened else {
                panic!("not the result of session/open");
            };
            let layer = Front {
                core: front.core.clone(),
                client,
                session: front.session.clone(),
                state: opened.session,
                seen: std::cell::RefCell::new(Vec::new()),
                answered: std::cell::RefCell::new(Vec::new()),
                answering: std::cell::RefCell::new(Answering::default()),
            };
            front.prompt("run it twice").await;
            for start in 0..2 {
                let (id, request) = layer.approval().await;
                assert_eq!(request.tool, "probe/twice", "start {start}");
                let (same, _) = front.approval().await;
                assert_eq!(same, id, "start {start}");
                layer
                    .answer(&id, cineris_protocol::methods::ApprovalChoice::AllowOnce)
                    .unwrap();
            }
            front.turns(1).await;
            let answer = answered(&front, 0);
            assert!(answer.contains("ran twice"), "{answer}");
            front.close().await;
            layer.close().await;
        });
    }

    /// At `unrestricted` the shell's commands run unconfined and an
    /// extension's programs do not: a question about one says it is
    /// confined (by the extension's own launcher), not the session's word.
    #[test]
    fn sys_a_question_about_a_program_at_unrestricted_says_it_is_confined() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_confined_detail", []);
        world.user_config(
            "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"sh *\"\naction = \"ask\"\n",
        );
        let probe = extension(&world, "probe", "exec = [\"sh\"]");
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "sh", "args": ["-c", "echo hi"]}),
        ));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Unrestricted).await;
            front.turn("run it").await;
            let asked = front
                .approvals()
                .into_iter()
                .find(|approval| approval.tool == "probe/run")
                .expect("the program was asked about");
            let ApprovalDetail::Command(detail) = &asked.detail else {
                panic!("not a command: {:?}", asked.detail);
            };
            assert!(!detail.outside_sandbox, "{detail:?}");
            front.close().await;
        });
    }

    /// Without `exec_network` a program reaches no socket beyond its own
    /// processes; with it, it does. Both are started through the
    /// extension's launcher, the first unasked about the network.
    #[test]
    fn sys_a_program_has_the_network_only_with_exec_network() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_network", []);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connect = format!(
            "if (exec 3<>/dev/tcp/127.0.0.1/{port}) 2>/dev/null; then echo reached; else echo \
             refused; fi"
        );
        let closed = extension(&world, "closed", "exec = [\"bash\"]");
        let open = extension(&world, "open", "exec = [\"bash\"]\nexec_network = true");
        world.provider.push(calls(&[(
            "c1",
            "tool_call",
            json!({"tool": "closed/run", "arguments": {"program": "bash", "args": ["-c", connect]}}),
        )]));
        world.provider.push(calls(&[(
            "c2",
            "tool_call",
            json!({"tool": "open/run", "arguments": {"program": "bash", "args": ["-c", connect]}}),
        )]));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![closed, open];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(Answering::allow_all());
            front.turn("run both").await;
            let closed = ran(&answered(&front, 0));
            let open = ran(&answered(&front, 1));
            assert!(
                closed["stdout"].as_str().unwrap().contains("refused"),
                "{closed}"
            );
            assert!(
                open["stdout"].as_str().unwrap().contains("reached"),
                "{open}"
            );
            front.close().await;
        });
        drop(listener);
    }

    /// What the call at `index` says when its turn's stop ended it: a
    /// denial, not the script's own answer, which would have caught a
    /// refusal and gone on.
    fn stopped(front: &Front, index: usize) -> String {
        let results = front.results();
        let (_, outcome, text) = &results[index];
        assert_eq!(*outcome, "denied", "{results:?}");
        text.clone()
    }

    /// A program whose write root holds a protected path, and which writes
    /// one, has its call stopped with the change named, and the turn
    /// stops: the model's next call does not run. At `unrestricted` too,
    /// where the shell's commands are not looked around and the program's
    /// own launcher still confines it (the review of the extensions'
    /// process starts: no test ran the look there).
    #[test]
    fn sys_a_program_that_writes_a_protected_path_fails_and_stops_the_turn() {
        use cineris_protocol::lease::Level;

        if !enforcing_here() {
            return;
        }
        for (name, level) in [
            ("exec_protected", Level::Supervised),
            ("exec_protected_unrestricted", Level::Unrestricted),
        ] {
            let world = World::new(name, []);
            world.put(".github/README", "ci\n");
            world.user_config(
                "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \
                 \"allow\"\n",
            );
            let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\".github/**\"]");
            world.provider.push(run_call(
                "c1",
                "probe",
                &json!({"program": "sh", "args": [
                    "-c", "mkdir -p .github/workflows && echo evil > .github/workflows/ci.yml"
                ]}),
            ));
            world.provider.push(run_call(
                "c2",
                "probe",
                &json!({"program": "sh", "args": ["-c", "echo after > .github/after.txt"]}),
            ));
            world.provider.push(says("Done."));
            let core = world.core_with(|options| {
                options.extensions = vec![probe];
                options.launcher = Some(launcher());
            });
            world.run(async {
                let front = front_at(&world, &core, level.clone()).await;
                let ended = front.turn("run it").await;
                let answer = stopped(&front, 0);
                assert!(
                    answer.contains("changed protected paths")
                        && answer.contains(".github/workflows"),
                    "{level:?}: {answer}"
                );
                assert_eq!(
                    ended.outcome,
                    cineris_protocol::event::TurnOutcome::Denied,
                    "{level:?}: {ended:?}"
                );
                assert!(world.read(".github/after.txt").is_none(), "{level:?}");
                front.close().await;
            });
        }
    }
    /// A protected path that changed after the last look, by nothing of
    /// the harness's (here the test's own hand, between two turns), keeps
    /// the next program with a write root from starting, and stops the
    /// turn.
    #[test]
    fn sys_a_protected_path_changed_since_the_last_program_refuses_the_next() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_continuity", []);
        world.put(".github/README", "ci\n");
        let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\"out/**\"]");
        std::fs::create_dir_all(world.workspace.join("out")).unwrap();
        let call = json!({"program": "sh", "args": ["-c", "echo ran >> out/ran.txt"]});
        world.provider.push(run_call("c1", "probe", &call));
        world.provider.push(says("Done."));
        world.provider.push(run_call("c2", "probe", &call));
        // The turn is stopped: the model's next call does not run.
        world.provider.push(run_call("c3", "probe", &call));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Supervised).await;
            front.turn("run it").await;
            assert_eq!(world.read("out/ran.txt").as_deref(), Some("ran\n"));
            world.put(".github/workflows/new.yml", "on: push\n");
            let ended = front.turn("run it again").await;
            let answer = stopped(&front, 1);
            assert!(
                answer.contains("A protected path changed since the last command")
                    && answer.contains(".github/workflows"),
                "{answer}"
            );
            assert_eq!(world.read("out/ran.txt").as_deref(), Some("ran\n"));
            assert_eq!(
                ended.outcome,
                cineris_protocol::event::TurnOutcome::Denied,
                "{ended:?}"
            );
            front.close().await;
        });
    }

    /// At `unrestricted` the shell's commands run as they are, and the look
    /// at what one changed runs git as it is; an extension's program is
    /// still confined, by its own launcher, and the look at what it changed
    /// runs git inside the sandbox, as the program ran (the review of the
    /// extensions' process starts). A program that plants a clean filter in
    /// a trusted repository's configuration does not have the harness run
    /// it with the user's rights: the mark the filter makes, outside every
    /// write root, never appears, and the change to `.git/config`, a
    /// protected path, stops the call. `harness_git`'s tests are the
    /// control: at `unrestricted` the look after a shell command runs the
    /// same filter.
    #[test]
    fn sys_the_look_after_a_program_runs_git_confined_at_unrestricted() {
        use cineris_testkit::scratch::git;

        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_git_follows", []);
        world.put("tracked.txt", "one\n");
        git(
            &world.workspace,
            &["init", "--quiet", "--initial-branch=main"],
        );
        git(&world.workspace, &["add", "--all"]);
        git(
            &world.workspace,
            &["commit", "--quiet", "--message", "first"],
        );
        let marker = world.dir.join("filter-ran");
        let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\"**\"]");
        let plant = format!(
            "printf '[filter \"x\"]\\n\\tclean = touch {m}; cat\\n' >> .git/config; echo '* \
             filter=x' > .gitattributes; touch -d @0 tracked.txt",
            m = marker.display()
        );
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "sh", "args": ["-c", plant]}),
        ));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
            options.trust_project = true;
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Unrestricted).await;
            front.turn("run it").await;
            let answer = stopped(&front, 0);
            assert!(answer.contains(".git/config"), "{answer}");
            assert!(
                !marker.exists(),
                "the look after the program ran the filter it planted outside the sandbox: \
                 {answer}"
            );
            front.close().await;
        });
    }

    /// How a program that wrote a protected path is ended, for
    /// [`a_program_ended_early_is_looked_at`].
    enum Ending {
        /// By its own time, which the script gave.
        ItsTime,
        /// By the call's cancellation: the turn is interrupted.
        TheCallsCancel,
    }

    /// The time the script gives the program in those tests.
    const ITS_TIME: std::time::Duration = std::time::Duration::from_secs(7);

    /// A program writes a protected path, then waits; it is ended as `how`
    /// says, once the test knows the write is made. What it did is looked
    /// at all the same: the change is named, the turn stops, and undo puts
    /// it back.
    fn a_program_ended_early_is_looked_at(name: &str, how: Ending) {
        use cineris_protocol::Item;
        use cineris_protocol::methods::{Op, SessionRewindParams};

        let world = World::new(name, [says("Ready.")]);
        world.put(".github/README", "ci\n");
        world.user_config(
            "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
        );
        // The program waits on a pipe outside the workspace once its write
        // is made, so that the test ends it after the write, with no wait
        // of real time: opening the pipe to write blocks until the program
        // opens it to read.
        let ready = world.dir.join("ready");
        let made = std::process::Command::new("mkfifo")
            .arg(&ready)
            .status()
            .expect("mkfifo runs");
        assert!(made.success());
        let probe = extension(&world, "probe", "exec = [\"sh\"]\nwrite = [\".github/**\"]");
        let script = format!(
            "mkdir -p .github/workflows && echo evil > .github/workflows/x.yml && cat '{}' > \
             /dev/null; exec sleep 600",
            ready.display()
        );
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({
                "program": "sh",
                "args": ["-c", script],
                "timeout": u64::try_from(ITS_TIME.as_millis()).unwrap(),
            }),
        ));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Supervised).await;
            front.turn("hello").await;
            let keep = front
                .items()
                .iter()
                .rev()
                .find_map(|item| match item {
                    Item::AssistantMessage(message) => Some(message.id.clone()),
                    _ => None,
                })
                .unwrap();
            let mut turn = std::pin::pin!(front.turn("run it"));
            let end = async {
                // The program has started: its own time is running.
                world.timer.sleeping_for(ITS_TIME).await;
                // And it has made its write.
                drop(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&ready)
                        .expect("the pipe opens"),
                );
                match how {
                    Ending::ItsTime => world.timer.advance(ITS_TIME),
                    Ending::TheCallsCancel => front.interrupt(false).await,
                }
            };
            let mut end = std::pin::pin!(end);
            let mut ended = false;
            std::future::poll_fn(|context| {
                if !ended {
                    ended = end.as_mut().poll(context).is_ready();
                }
                turn.as_mut().poll(context).map(|_| ())
            })
            .await;
            let answer = stopped(&front, 0);
            assert!(
                answer.contains("changed protected paths") && answer.contains(".github/workflows"),
                "{answer}"
            );
            assert!(world.read(".github/workflows/x.yml").is_some());
            front
                .op(Op::SessionRewind(SessionRewindParams {
                    session_id: front.session.clone(),
                    to: Some(keep),
                    before: None,
                    restore_files: true,
                    plan: None,
                }))
                .await
                .unwrap();
            assert!(
                world.read(".github/workflows/x.yml").is_none(),
                "undo did not take away the file the program made"
            );
            front.close().await;
        });
    }

    /// A program stopped at its own time, after it wrote a protected path,
    /// is looked at (the review of the extensions' process starts: every
    /// way a run ends has what follows it done).
    #[test]
    fn sys_a_program_stopped_at_its_time_is_still_looked_at() {
        if !enforcing_here() {
            return;
        }
        a_program_ended_early_is_looked_at("exec_its_time_looked", Ending::ItsTime);
    }

    /// A program stopped by the call's cancellation, after it wrote a
    /// protected path, is looked at.
    #[test]
    fn sys_a_program_the_call_cancels_is_still_looked_at() {
        if !enforcing_here() {
            return;
        }
        a_program_ended_early_is_looked_at("exec_cancel_looked", Ending::TheCallsCancel);
    }

    /// A protected path a program changed stops the script's call, not
    /// only the turn: a script that catches the refusal and goes on does
    /// not go on, and its call ends as stopped, never with its own answer
    /// (the review of the extensions' process starts).
    #[test]
    fn sys_a_script_that_catches_a_protected_change_does_not_go_on() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_caught_stop", []);
        world.put(".github/README", "ci\n");
        world.user_config(
            "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
        );
        let probe = extension_running(
            &world,
            "probe",
            "exec = [\"sh\"]\nwrite = [\".github/**\"]",
            "cineris.tool({ name: 'run', input: { type: 'object' }, run: async () => { \
               try { await cineris.exec('sh', ['-c', 'mkdir -p .github/workflows && echo evil > \
               .github/workflows/x.yml']); } catch (e) {} \
               return 'went on'; } });",
        );
        world.provider.push(run_call("c1", "probe", &json!({})));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Supervised).await;
            front.turn("run it").await;
            let answer = stopped(&front, 0);
            assert!(
                !answer.contains("went on") && answer.contains(".github/workflows"),
                "{answer}"
            );
            front.close().await;
        });
    }

    /// While commands are confined, a program named by its path inside a
    /// writable root (here the workspace) is refused: a confined command
    /// could have written it.
    #[test]
    fn a_program_named_by_a_path_inside_a_writable_root_is_refused_while_confined() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("exec_planted_path", []);
        world.put("tools/x", "#!/bin/sh\necho ran > ran.txt\n");
        let planted = world.workspace.join("tools").join("x");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let probe = extension(
            &world,
            "probe",
            &format!("exec = [{}]", literal(&planted.to_string_lossy())),
        );
        world
            .provider
            .push(run_call("c1", "probe", &json!({"program": planted})));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.launcher = Some(launcher());
        });
        world.run(async {
            let front = front_at(&world, &core, cineris_protocol::lease::Level::Supervised).await;
            front.turn("run it").await;
            let answer = answered(&front, 0);
            assert!(
                answer.starts_with("refused:") && answer.contains("inside a writable root"),
                "{answer}"
            );
            assert!(world.read("ran.txt").is_none());
            front.close().await;
        });
    }
}

#[cfg(windows)]
mod windows {
    //! A batch script on Windows: found a directory at a time, and started
    //! through `cmd.exe` with each argument quoted, or refused.

    use super::*;

    /// A folder outside the workspace with `files` in it, each a batch
    /// script's text, and the echo beside them as `echo.exe`.
    fn bin(world: &World, folder: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = world.dir.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text.replace('\n', "\r\n")).unwrap();
        }
        place_echo(&dir, "echo");
        dir
    }

    /// An npm-style shim: its own folder found through `call :label` and
    /// `%~dp0`, and the program beside it given `%*`.
    const SHIM: &str = "@ECHO off\nGOTO start\n:find_dp0\nSET dp0=%~dp0\nEXIT /b\n:start\n\
         SETLOCAL\nCALL :find_dp0\n\"%dp0%\\echo.exe\" --exact launcher_entry --nocapture \
         cin-echo -- %*\n";

    /// A shim in a folder whose path has spaces runs (its `%~dp0` and
    /// `call :label`), and hands the program beside it each argument as
    /// written through `%*`: a trailing backslash that does not end it
    /// early, spaces, an empty one, and its input.
    #[test]
    fn sys_a_shim_passes_each_argument_and_its_input_as_written() {
        let world = World::new("exec_shim", []);
        let dir = bin(&world, "my tools", &[("tool.cmd", SHIM)]);
        let probe = extension(&world, "probe", "exec = [\"tool\"]");
        let given = [
            "x\\",
            "a --upload-pack=evil",
            "two  spaces",
            "--x=y,z;w",
            "",
        ];
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "tool", "args": given, "stdin": "from the script\r\n"}),
        ));
        world.provider.push(says("Done."));
        let env = path_with(&world, &[&dir], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.shell_env = Some(env);
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(Answering::allow_all());
            front.turn("run it").await;
            let ran = ran(&answered(&front, 0));
            assert_eq!(ran["code"], 7, "{ran}");
            let report = report(&ran);
            assert_eq!(report["args"], json!(given), "{report}");
            assert_eq!(report["stdin"], "from the script\r\n");
            front.close().await;
        });
    }

    /// In a Bash session on Windows a program's line is judged as Bash
    /// reads it, but the program is found by Windows' lookup, not Bash's:
    /// `echo` here is the `echo.exe` first on `PATH`, not Bash's own, as
    /// `sort` would be `System32\sort.exe`. What the shell calls read-only
    /// is asked about, as in PowerShell (the review of the extensions'
    /// process starts).
    #[test]
    fn sys_in_bash_on_windows_a_program_the_shell_calls_read_only_is_asked_about() {
        let world = World::new("exec_bash_windows", []);
        world.user_config(
            "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
        );
        let probe = extension(&world, "probe", "exec = [\"echo\"]");
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "echo", "args": echo_args(&[])}),
        ));
        world.provider.push(says("Done."));
        let dir = bin(&world, "bin", &[]);
        let env = path_with(&world, &[&dir], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.shell_env = Some(env);
            options.shell = Some(cineris_tools::Shell::of_kind(
                cineris_tools::ShellKind::Bash,
                "bash",
            ));
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            let asked = front.turn_denying("run it").await;
            assert_eq!(asked.len(), 1, "{asked:?}");
            let approval = &front.approvals()[0];
            assert!(
                approval
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("whose lookup is not the one that finds it")),
                "{approval:?}"
            );
            let answer = answered(&front, 0);
            assert!(answer.starts_with("refused:"), "{answer}");
            front.close().await;
        });
    }

    /// A `.exe` reads its input too, as the runner's pipe gives it.
    #[test]
    fn sys_an_exe_reads_exactly_its_input() {
        let world = World::new("exec_exe_input", []);
        let dir = bin(&world, "bin", &[]);
        let probe = extension(&world, "probe", "exec = [\"echo\"]");
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "echo", "args": echo_args(&["x"]), "stdin": "exact\ninput"}),
        ));
        world.provider.push(says("Done."));
        let env = path_with(&world, &[&dir], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.shell_env = Some(env);
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(Answering::allow_all());
            front.turn("run it").await;
            let report = report(&ran(&answered(&front, 0)));
            assert_eq!(report["stdin"], "exact\ninput");
            assert_eq!(report["args"], json!(["x"]));
            front.close().await;
        });
    }

    /// A directory at a time: `x.cmd` in an earlier folder is taken over
    /// `x.exe` in a later one; and `y.ps1`, alone, is never run.
    #[test]
    fn sys_a_shim_in_an_earlier_folder_beats_an_exe_in_a_later_one_and_no_ps1_runs() {
        let world = World::new("exec_order", []);
        let first = bin(
            &world,
            "first",
            &[
                ("x.cmd", "@echo from the shim\n"),
                ("y.ps1", "Set-Content ran.txt ran\n"),
            ],
        );
        let second = world.dir.join("second");
        std::fs::create_dir_all(&second).unwrap();
        place_echo(&second, "x");
        let probe = extension(&world, "probe", "exec = [\"x\", \"y\"]");
        world
            .provider
            .push(run_call("c1", "probe", &json!({"program": "x"})));
        world
            .provider
            .push(run_call("c2", "probe", &json!({"program": "y"})));
        world.provider.push(says("Done."));
        let env = path_with(&world, &[&first, &second], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.shell_env = Some(env);
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(Answering::allow_all());
            front.turn("run both").await;
            let x = ran(&answered(&front, 0));
            assert!(
                x["stdout"].as_str().unwrap().contains("from the shim"),
                "{x}"
            );
            let y = answered(&front, 1);
            assert!(
                y.starts_with("refused:") && y.contains("`y` was not found"),
                "{y}"
            );
            assert!(world.read("ran.txt").is_none());
            front.close().await;
        });
    }

    /// Every character `cmd.exe` cannot carry, or that a script which
    /// expands its arguments again would act on, is refused before anything
    /// is asked or started, naming the argument and the character.
    #[test]
    fn each_character_a_shim_cannot_carry_is_refused_with_its_name() {
        let world = World::new("exec_shim_refused", []);
        world.user_config(
            "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
        );
        let dir = bin(&world, "bin", &[("tool.cmd", "echo ran > ran.txt\n")]);
        let probe = extension(&world, "probe", "exec = [\"tool\"]");
        let cases = [
            ("a\"b", "a double quote"),
            ("100%", "a percent sign"),
            ("wow!", "an exclamation mark"),
            ("a&b", "an ampersand"),
            ("a|b", "a vertical bar"),
            ("a<b", "a less-than sign"),
            ("a>b", "a greater-than sign"),
            ("a^", "a caret"),
            ("--x=(a", "an opening parenthesis"),
            ("a)", "a closing parenthesis"),
            ("a\rb", "a carriage return"),
            ("a\nb", "a line feed"),
            ("a\0b", "NUL"),
        ];
        for (index, (argument, _)) in cases.iter().enumerate() {
            world.provider.push(run_call(
                &format!("c{index}"),
                "probe",
                &json!({"program": "tool", "args": ["fine", argument]}),
            ));
        }
        world.provider.push(says("Done."));
        let env = path_with(&world, &[&dir], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.shell_env = Some(env);
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("run each").await;
            assert!(front.approvals().is_empty(), "{:?}", front.approvals());
            for (index, (_, name)) in cases.iter().enumerate() {
                let answer = answered(&front, index);
                assert!(
                    answer.starts_with("refused:")
                        && answer.contains("argument 2")
                        && answer.contains(name),
                    "{index}: {answer}"
                );
            }
            assert!(world.read("ran.txt").is_none());
            front.close().await;
        });
    }

    /// A script that takes an argument's quotes off and reads it again
    /// (`set A=%~1`, then `echo %A%`) would run what follows an `&` in it:
    /// such an argument is refused, and nothing of it runs. The script
    /// itself runs with an argument it can be given.
    #[test]
    fn sys_a_script_that_expands_an_argument_again_is_never_given_an_operator() {
        let world = World::new("exec_shim_expands", []);
        let dir = bin(
            &world,
            "bin",
            &[("reread.cmd", "@echo off\nset A=%~1\necho %A%\n")],
        );
        let probe = extension(&world, "probe", "exec = [\"reread\"]");
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": "reread", "args": ["x & echo pwned> pwned.txt"]}),
        ));
        world.provider.push(run_call(
            "c2",
            "probe",
            &json!({"program": "reread", "args": ["said back"]}),
        ));
        world.provider.push(says("Done."));
        let env = path_with(&world, &[&dir], &[]);
        let core = world.core_with(|options| {
            options.extensions = vec![probe];
            options.shell_env = Some(env);
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(Answering::allow_all());
            front.turn("run both").await;
            let answer = answered(&front, 0);
            assert!(
                answer.starts_with("refused:")
                    && answer.contains("argument 1")
                    && answer.contains("an ampersand"),
                "{answer}"
            );
            assert!(world.read("pwned.txt").is_none(), "{answer}");
            let said = ran(&answered(&front, 1));
            assert_eq!(said["code"], 0, "{said}");
            assert!(
                said["stdout"]
                    .as_str()
                    .is_some_and(|out| out.contains("said back")),
                "{said}"
            );
            front.close().await;
        });
    }

    /// Git for Windows' MSYS2 `echo.exe`, where the runner has it: a
    /// program of the Cygwin family, which expands a pattern it is given
    /// unquoted against the folder it runs in, and takes `'` and braces as
    /// a shell would.
    fn msys_echo() -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|folder| PathBuf::from(folder).join(r"Git\usr\bin\echo.exe"))
            .collect();
        candidates.push(PathBuf::from(r"C:\Program Files\Git\usr\bin\echo.exe"));
        // Where `git.exe` is on `PATH`: `Git\cmd`, `Git\bin` or
        // `Git\mingw64\bin`.
        if let Some(path) = std::env::var_os("PATH") {
            for folder in std::env::split_paths(&path) {
                if folder.join("git.exe").is_file() {
                    for up in folder.ancestors().skip(1).take(2) {
                        candidates.push(up.join(r"usr\bin\echo.exe"));
                    }
                }
            }
        }
        candidates.into_iter().find(|candidate| candidate.is_file())
    }

    /// A Cygwin or MSYS2 program named by its path, given a pattern, a
    /// quote and braces, reads each as written: every argument reaches it
    /// in double quotes, so it expands nothing against the files in the
    /// folder it runs in. Without the quotes, `.en*` read `.env`.
    #[test]
    fn sys_an_msys_program_reads_a_pattern_as_written() {
        let Some(echo) = msys_echo() else {
            eprintln!(
                "skipped: no Git for Windows `usr\\bin\\echo.exe` here, the MSYS2 program this \
                 test runs"
            );
            return;
        };
        let world = World::new("exec_msys", []);
        world.put(".env", "SECRET=1\n");
        world.put("a.txt", "a\n");
        world.put("b.txt", "b\n");
        let probe = extension(
            &world,
            "probe",
            &format!("exec = [{}]", literal(&echo.to_string_lossy())),
        );
        let given = ["*", ".en*", "[ab].txt", "{a,b}.txt", "it's", "?.txt"];
        world.provider.push(run_call(
            "c1",
            "probe",
            &json!({"program": echo, "args": given}),
        ));
        world.provider.push(says("Done."));
        let core = world.core_with(|options| options.extensions = vec![probe]);
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.answering(Answering::allow_all());
            front.turn("run it").await;
            let ran = ran(&answered(&front, 0));
            assert_eq!(ran["code"], 0, "{ran}");
            let out = ran["stdout"].as_str().expect("its output");
            assert_eq!(out.trim_end(), given.join(" "), "{ran}");
            assert!(!out.contains(".env ") && !out.contains("a.txt"), "{ran}");
            front.close().await;
        });
    }
}
