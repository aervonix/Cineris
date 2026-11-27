//! The three examples under `examples/` (`docs/steps/28/answered.md`, point
//! 10, "3c"), run as their READMEs say, on one machine: each extension added
//! with `cineris ext add` at a terminal, signed with the person's approvals
//! password (the signed approvals step), the command file copied into the
//! user's `commands/`, and a workspace that is a git repository with a tag
//! and two commits after it.
//!
//! One release, in two headless runs against the scripted model, each at a
//! terminal of its own, where it asks for the approvals' password before
//! its session opens (a headless run with no terminal uses no approvals),
//! with the
//! rules the READMEs of `release` and `md-outline` offer in the user's
//! configuration, as they are written there: `/changelog:draft v0.1.0` (the
//! script command, its `git log` allowed by the README's rule for its line),
//! then `/release-notes 0.2.0` (the command file), whose model outlines the
//! changelog with `md-outline/outline`, is refused a file outside its
//! manifest, and then cleans up and pushes through `shell-gate`'s gate.
//! Each rule is read from what the model was sent, and judged by its own
//! name. The forms the gate reads, and the revision the command refuses,
//! are `cineris-ext`'s `examples` tests, through the engine alone.

use std::path::{Path, PathBuf};

use cineris_testkit::scratch::{git, git_with_input};
use serde_json::{Value, json};

use crate::{Machine, Run, shown};

/// A path of the repository, named with no `..`, as a person would name
/// it.
fn repository(path: &str) -> PathBuf {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    crates.parent().unwrap().join(path)
}

/// The rule a README offers, as its `toml` block writes it.
fn offered_rule(readme: &str) -> String {
    let text = std::fs::read_to_string(repository(readme)).unwrap();
    let start = text
        .find("```toml\n")
        .unwrap_or_else(|| panic!("{readme} offers no rule"))
        + "```toml\n".len();
    let length = text[start..].find("```").unwrap();
    text[start..start + length].to_owned()
}

/// The changelog the model outlines: a `#` line in a fenced block is not a
/// heading.
const CHANGELOG: &str = "# Changelog\n\n## 0.2.0\n\n### Added\n\n- An outline of a \
                         document:\n\n  ```\n  # not a heading\n  ```\n\n### Fixed\n\n- The \
                         outline of an empty file.\n\n## 0.1.0\n\n- The first release.\n";

/// The shell calls of the release's last step, each with what the gate is
/// to make of it. The forms the gate reads are `cineris-ext`'s `examples`
/// tests.
const SHELL: [(&str, &str); 5] = [
    ("s1", "rm -rf build"),
    ("s2", "git push --force origin main"),
    ("s3", "git fetch origin && git push origin +main"),
    ("s4", "git push --force-with-lease origin main"),
    ("s5", "echo released"),
];

/// The machine, and what each run left: its own streams, and every request
/// the model was sent during it.
struct Release {
    machine: Machine,
    draft: Run,
    draft_sent: Vec<Value>,
    notes: Run,
    notes_sent: Vec<Value>,
}

impl Release {
    fn run() -> Self {
        // The repository map is background work this test does not need.
        let config = format!(
            "index.map = false\n{}{}",
            offered_rule("examples/release/README.md"),
            offered_rule("examples/md-outline/README.md")
        );
        let mut machine = Machine::with_config("sys_examples", &config);
        machine.quick_dialogs();
        machine.person();
        for folder in [
            "examples/md-outline",
            "examples/shell-gate",
            "examples/release/extensions/changelog",
        ] {
            add(&machine, &repository(folder));
        }
        let commands = machine.dirs().config.unwrap().join("commands");
        std::fs::create_dir_all(&commands).unwrap();
        std::fs::copy(
            repository("examples/release/commands/release-notes.md"),
            commands.join("release-notes.md"),
        )
        .unwrap();
        let workspace = &machine.workspace;
        std::fs::write(workspace.join("CHANGELOG.md"), CHANGELOG).unwrap();
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(workspace.join("build")).unwrap();
        std::fs::write(workspace.join("build/out.txt"), "built\n").unwrap();
        // The history in one start of git after the first: a start costs
        // about 50 ms on a hosted Windows runner.
        git(workspace, &["init", "--quiet"]);
        git_with_input(workspace, &["fast-import", "--quiet"], &history());

        machine.reply_text("Drafted.");
        let before = machine.server.requests().len();
        let draft = at_a_terminal(&machine, &["-p", "/changelog:draft v0.1.0"]);
        let draft_sent = sent(&machine, before);

        // A response for each step, its calls together.
        let outline =
            |path: &str| json!({"tool": "md-outline/outline", "arguments": {"path": path}});
        reply_calls(
            &machine,
            &[
                ("o1", "tool_call", outline("CHANGELOG.md")),
                ("o2", "tool_call", outline("src/main.rs")),
            ],
        );
        let shell: Vec<(&str, &str, Value)> = SHELL
            .iter()
            .map(|(id, command)| (*id, "shell", json!({"command": command})))
            .collect();
        reply_calls(&machine, &shell);
        machine.reply_text("Released.");
        let before = machine.server.requests().len();
        let notes = at_a_terminal(&machine, &["-p", "/release-notes 0.2.0"]);
        let notes_sent = sent(&machine, before);
        Self {
            machine,
            draft,
            draft_sent,
            notes,
            notes_sent,
        }
    }
}

/// The workspace's history, as `git fast-import` reads it: a commit the tag
/// `v0.1.0` names, and two after it.
fn history() -> Vec<u8> {
    let mut stream = String::new();
    for (mark, subject) in [
        "Start the project",
        "Add an outline of a document",
        "Fix the outline of an empty file",
    ]
    .iter()
    .enumerate()
    {
        stream.push_str(&format!(
            "commit refs/heads/main\nmark :{}\ncommitter Cineris Test <test@cineris.invalid> \
             1767225600 +0000\ndata {}\n{subject}\n",
            mark + 1,
            subject.len()
        ));
        if mark > 0 {
            stream.push_str(&format!("from :{mark}\n"));
        }
        stream.push('\n');
    }
    stream.push_str("reset refs/tags/v0.1.0\nfrom :1\n\n");
    stream.into_bytes()
}

/// The model calls each of `calls`, in one response.
fn reply_calls(machine: &Machine, calls: &[(&str, &str, Value)]) {
    let mut events = vec![crate::message_start()];
    for (index, (id, tool, input)) in calls.iter().enumerate() {
        events.extend([
            json!({"type": "content_block_start", "index": index,
                "content_block": {"type": "tool_use", "id": id, "name": tool, "input": {}}}),
            json!({"type": "content_block_delta", "index": index,
                "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
            json!({"type": "content_block_stop", "index": index}),
        ]);
    }
    events.extend([
        crate::message_delta("tool_use"),
        json!({"type": "message_stop"}),
    ]);
    machine
        .server
        .push(cineris_testkit::ServerReply::sse().send(crate::sse(&events)));
}

/// Adds the extension in `folder` as its README says: shown at a terminal,
/// and approved with the person's approvals password.
fn add(machine: &Machine, folder: &Path) {
    let path = folder.to_string_lossy().into_owned();
    machine
        .signing(&["ext", "add", &path], &[], crate::approvals::PASSWORD)
        .expect_code(0);
}

/// A headless run at a terminal of its own, which asks for the approvals'
/// password before its session opens: the person's is typed.
fn at_a_terminal(machine: &Machine, arguments: &[&str]) -> Run {
    let mut session = machine.tui(arguments);
    session.wait_for(crate::tui::LIMIT, "the approvals' password", |t| {
        t.shows("to use this session's approvals")
    });
    crate::approvals::type_password(&mut session, crate::approvals::PASSWORD);
    crate::approvals::ended(&mut session).as_run()
}

/// The bodies of the requests the model was sent after the first `before`.
fn sent(machine: &Machine, before: usize) -> Vec<Value> {
    machine.server.requests()[before..]
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

/// The text of each part of a message's content.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part.get("text") {
                Some(Value::String(text)) => text.clone(),
                _ => text_of(part.get("content").unwrap_or(&Value::Null)),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The first prompt of a run, as the model was sent it.
fn prompt(sent: &[Value]) -> String {
    let first = sent.first().expect("the model was sent a request");
    text_of(&first["messages"][0]["content"])
}

/// What the model was given back for its call `id`: the result's text, and
/// whether it is an error.
fn result(sent: &[Value], id: &str) -> (String, bool) {
    let last = sent.last().expect("the model was sent a request");
    for message in last["messages"].as_array().into_iter().flatten() {
        for part in message["content"].as_array().into_iter().flatten() {
            if part["type"] == "tool_result" && part["tool_use_id"] == id {
                return (
                    text_of(&part["content"]),
                    part["is_error"].as_bool().unwrap_or(false),
                );
            }
        }
    }
    panic!("no result for `{id}` in {last}");
}

/// The examples run as their READMEs say: each rule below, judged by its
/// own name, from one release's two runs on one machine.
#[test]
fn sys_the_examples_run_as_their_readmes_say() {
    let world = Release::run();
    let rules: [crate::Rule<Release>; 7] = [
        (
            "changelog_draft_sends_the_commits_since_the_tag_from_git_log",
            changelog_draft_sends_the_commits_since_the_tag_from_git_log,
        ),
        (
            "release_notes_sends_its_body_with_the_arguments_put_in",
            release_notes_sends_its_body_with_the_arguments_put_in,
        ),
        (
            "md_outline_lists_the_headings_of_a_markdown_file",
            md_outline_lists_the_headings_of_a_markdown_file,
        ),
        (
            "md_outline_is_refused_a_file_outside_its_read_capability",
            md_outline_is_refused_a_file_outside_its_read_capability,
        ),
        ("shell_gate_refuses_rm_rf", shell_gate_refuses_rm_rf),
        (
            "shell_gate_refuses_a_forced_push",
            shell_gate_refuses_a_forced_push,
        ),
        (
            "shell_gate_lets_a_lease_and_other_commands_through",
            shell_gate_lets_a_lease_and_other_commands_through,
        ),
    ];
    crate::judge(&world, &rules);
}

/// `/changelog:draft v0.1.0` runs `git log` through the extension's `exec`,
/// allowed by the rule its README offers for its line alone, and sends the
/// commits after the tag, newest first, under a line that says the text is
/// the extension's; the commit the tag names is not among them.
fn changelog_draft_sends_the_commits_since_the_tag_from_git_log(world: &Release) {
    world.draft.expect_code(0);
    let prompt = prompt(&world.draft_sent);
    let fix = prompt.find(" Fix the outline of an empty file\n");
    let add = prompt.find(" Add an outline of a document\n");
    assert!(
        prompt.contains("Draft the changelog entry for the changes since v0.1.0.")
            && fix.is_some()
            && add.is_some()
            && fix < add
            && !prompt.contains("Start the project"),
        "{prompt}\n{}",
        shown(&world.draft)
    );
    assert!(
        prompt.contains("of the extension `changelog`, written by its author"),
        "{prompt}"
    );
}

/// `/release-notes 0.2.0` sends the command file's body, with the version
/// typed and the audience's default put in, and nothing of its front
/// matter.
fn release_notes_sends_its_body_with_the_arguments_put_in(world: &Release) {
    world.notes.expect_code(0);
    let prompt = prompt(&world.notes_sent);
    assert!(
        prompt.contains(
            "Write the release notes for version 0.2.0 of this project, for the people who use it."
        ) && prompt.contains("the changelog's entry for 0.2.0")
            && !prompt.contains("{{")
            && !prompt.contains("description:"),
        "{prompt}\n{}",
        shown(&world.notes)
    );
}

/// `md-outline/outline` reads the changelog through `cineris.fs.read` and
/// gives its headings, each with its level and line, leaving out a `#` line
/// in a fenced block.
fn md_outline_lists_the_headings_of_a_markdown_file(world: &Release) {
    let (text, error) = result(&world.notes_sent, "o1");
    assert!(!error, "{text}");
    for heading in [
        json!({"level": 1, "line": 1, "text": "Changelog"}),
        json!({"level": 2, "line": 3, "text": "0.2.0"}),
        json!({"level": 3, "line": 5, "text": "Added"}),
        json!({"level": 3, "line": 13, "text": "Fixed"}),
        json!({"level": 2, "line": 17, "text": "0.1.0"}),
    ] {
        assert!(text.contains(&heading.to_string()), "{heading}: {text}");
    }
    assert!(!text.contains("not a heading"), "{text}");
}

/// A path outside the manifest's `read` globs is refused by the manifest,
/// naming `capabilities.read`, though the README's rule allows the tool:
/// nothing of the file is read.
fn md_outline_is_refused_a_file_outside_its_read_capability(world: &Release) {
    let (text, error) = result(&world.notes_sent, "o2");
    assert!(
        error && text.contains("`capabilities.read`") && !text.contains("fn main"),
        "{text}"
    );
}

/// The gate's refusal names the hook and gives its reason.
fn refused_by_the_guard(world: &Release, id: &str, reason: &str) {
    let (text, error) = result(&world.notes_sent, id);
    assert!(
        error && text.contains("shell-gate") && text.contains(reason),
        "{id}: {text}"
    );
}

/// `rm -rf build` is refused by the gate, before anything is asked or run:
/// the folder is still there.
fn shell_gate_refuses_rm_rf(world: &Release) {
    refused_by_the_guard(world, "s1", "deletes a whole tree without asking");
    assert!(world.machine.workspace.join("build/out.txt").exists());
}

/// `git push --force` and a push of a `+` refspec are refused by the gate,
/// the second after another command in the same line.
fn shell_gate_refuses_a_forced_push(world: &Release) {
    refused_by_the_guard(world, "s2", "A forced push replaces the remote's history");
    refused_by_the_guard(world, "s3", "A forced push replaces the remote's history");
}

/// `git push --force-with-lease` passes the gate and reaches the permission
/// engine, which, with nobody to ask in this run, refuses it for its own
/// reason; `echo released` passes the gate and runs.
fn shell_gate_lets_a_lease_and_other_commands_through(world: &Release) {
    let (lease, _) = result(&world.notes_sent, "s4");
    assert!(
        lease.contains("nobody can be asked")
            && !lease.contains("shell-gate")
            && !lease.contains("A forced push"),
        "{lease}"
    );
    let (echoed, error) = result(&world.notes_sent, "s5");
    assert!(!error && echoed.contains("released"), "{echoed}");
}
