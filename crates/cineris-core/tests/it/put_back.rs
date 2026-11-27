//! A file a rewind or an undo makes again has the mode it had
//! (`docs/architecture/07-sessions-and-context.md`, "Snapshots and undo";
//! the step 25 fixes, which found every file made again with the default
//! mode: a script a command removed came back not executable, and a
//! private file the `write` tool had edited came back readable by others).
//!
//! The modes are Unix ones; on Windows a private file made again is
//! owner-only, which `cineris-session`'s own tests show.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use cineris_core::Outgoing;
use cineris_protocol::item::Delivery;
use cineris_protocol::methods::{
    Op, OpResult, QuestionResponse, ServerRequest, ServerResponse, SessionRewindParams,
    SessionUndoParams,
};
use cineris_protocol::{Item, ItemId};
use serde_json::json;

use crate::{Front, World, calls, says};

/// Runs git in `dir`, and fails the test when it fails: a test that went
/// on without its repository would pass for the wrong reason.
fn git(dir: &Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
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
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Makes `top` a repository with everything in it committed.
fn commit_all(top: &Path) {
    git(top, &["init", "-q"]);
    git(top, &["add", "."]);
    git(top, &["commit", "-q", "-m", "first"]);
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn set(path: &Path, bits: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap();
}

/// What the process's umask takes from a file it makes: what a directory,
/// made asking for every bit, lacks. A file made again is made as any new
/// file is, less it.
fn umask(world: &World) -> u32 {
    let probe = world.dir.join("umask-probe");
    std::fs::create_dir(&probe).unwrap();
    let mask = !mode(&probe) & 0o777;
    std::fs::remove_dir(&probe).unwrap();
    mask
}

/// `wanted` less the umask, checked to differ from what a new file gets
/// with no mode given: a test whose expected mode the umask alone would
/// give could pass with nothing put back.
fn expected(world: &World, wanted: u32) -> u32 {
    let umask = umask(world);
    let expected = wanted & !umask;
    assert_ne!(
        expected,
        0o666 & !umask,
        "under the umask {umask:o} a file made with mode {wanted:o} cannot be told from one made \
         with the default mode"
    );
    expected
}

/// Runs a turn of `command`, approved, and undoes it with its files.
fn undo_a_command(world: &World, core: &cineris_core::Core) {
    world.run(async {
        let front = Front::start(world, core).await;
        front.turn("hello").await;
        front.turn_allowing("run it").await;
        front
            .op(Op::SessionUndo(SessionUndoParams {
                session_id: front.session.clone(),
                restore_files: true,
            }))
            .await
            .expect("the undo");
        front.close().await;
    });
}

fn a_world_running(test: &str, command: &str) -> World {
    World::new(
        test,
        [
            says("Ready."),
            calls(&[("c1", "shell", json!({ "command": command }))]),
            says("Done."),
        ],
    )
}

fn trusting(world: &World) -> cineris_core::Core {
    world.core_with(|options| {
        options.trust_project = true;
        options.shell = None;
    })
}

/// A tracked script a command removed, with the folder it was in, put back
/// by an undo, executable. `repository` is where git's repository is: the
/// workspace, or the folder above it, so that git is asked about a
/// workspace that is not the top of its repository (git names its paths
/// from the workspace then too).
fn an_undo_puts_back_a_removed_script(test: &str, repository: fn(&World) -> &Path) {
    let world = a_world_running(test, "rm -r bin ./tools/-dash.sh");
    world.put("bin/run.sh", "#!/bin/sh\necho hi\n");
    world.put("tools/-dash.sh", "#!/bin/sh\necho dash\n");
    world.put("notes.txt", "kept\n");
    let script = world.workspace.join("bin/run.sh");
    let dash = world.workspace.join("tools/-dash.sh");
    for executable in [&script, &dash] {
        set(executable, 0o755);
    }
    commit_all(repository(&world));
    let core = trusting(&world);
    // Git keeps the execute bit and nothing for the group or others.
    let wanted = expected(&world, 0o700);
    undo_a_command(&world, &core);
    assert_eq!(
        world.read("bin/run.sh").as_deref(),
        Some("#!/bin/sh\necho hi\n"),
        "the content was not put back"
    );
    assert_eq!(
        world.read("tools/-dash.sh").as_deref(),
        Some("#!/bin/sh\necho dash\n")
    );
    for executable in [&script, &dash] {
        let now = mode(executable);
        assert_eq!(
            now,
            wanted,
            "{} was put back with mode {now:o}: it is no longer executable",
            executable.display()
        );
    }
}

/// An undo puts back a tracked script a command removed: it comes back
/// with its content, and as the executable it was (found by the review of
/// step 25: it came back at `0644`).
#[test]
fn sys_an_undo_puts_back_a_removed_script_with_its_execute_bit() {
    an_undo_puts_back_a_removed_script("put_back_script", |world| &world.workspace);
}

/// The same where the workspace is a folder inside its repository.
#[test]
fn sys_an_undo_puts_back_a_removed_script_in_a_workspace_below_its_repository() {
    an_undo_puts_back_a_removed_script("put_back_script_below", |world| {
        world.workspace.parent().unwrap()
    });
}

/// What git does not keep is not given (the full review of the step 25
/// fixes): a tracked file the person made private after it was committed
/// (`chmod 600`, which git does not see) and a command removed comes back
/// private, not as git's `100644` would have it; and where the repository
/// keeps no execute bit (`core.fileMode=false`), a file git calls
/// `100755` that was not executable here comes back without one.
#[test]
fn sys_an_undo_never_makes_a_tracked_file_wider_than_git_can_vouch_for() {
    let world = a_world_running("put_back_unvouched", "rm secret.txt tool.txt");
    world.put("secret.txt", "token\n");
    world.put("tool.txt", "not a program\n");
    let secret = world.workspace.join("secret.txt");
    let tool = world.workspace.join("tool.txt");
    set(&tool, 0o755);
    commit_all(&world.workspace);
    set(&secret, 0o600);
    git(&world.workspace, &["config", "core.fileMode", "false"]);
    set(&tool, 0o644);
    let core = trusting(&world);
    let private = umask(&world);
    undo_a_command(&world, &core);
    assert_eq!(world.read("secret.txt").as_deref(), Some("token\n"));
    assert_eq!(world.read("tool.txt").as_deref(), Some("not a program\n"));
    let now = mode(&secret);
    assert_eq!(
        now,
        0o600 & !private,
        "the private file was put back with mode {now:o}: other users can read it"
    );
    let now = mode(&tool);
    assert_eq!(
        now,
        0o600 & !private,
        "the file was put back with mode {now:o}: git's execute bit is not this repository's"
    );
}

/// A file only its owner and group may read (mode `0640`, which no umask
/// gives a new file by accident) that the turn edited with `write` and the
/// person removed after it, put back by a rewind the person said yes to:
/// it comes back as it was (found by the review of step 25: it came back
/// with the process's default mode, readable by others). The path the
/// model names is not the canonical one: through `.` and a folder and back.
#[test]
fn a_private_file_put_back_after_it_was_removed_is_private_again() {
    let world = World::new(
        "put_back_private",
        [
            calls(&[("c1", "read", json!({"path": "./notes/../private.txt"}))]),
            calls(&[(
                "c2",
                "write",
                json!({"path": "notes/../private.txt", "content": "kept to myself, changed\n"}),
            )]),
            says("Changed."),
        ],
    );
    world.put("notes/plain.txt", "plain\n");
    world.put("private.txt", "kept to myself\n");
    let private = world.workspace.join("private.txt");
    set(&private, 0o640);
    let wanted = expected(&world, 0o640);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn_allowing("change it").await;
        assert_eq!(
            world.read("private.txt").as_deref(),
            Some("kept to myself, changed\n")
        );
        assert_eq!(mode(&private), 0o640, "the write kept the mode");
        let prompt: ItemId = front
            .items()
            .into_iter()
            .find_map(|item| match item {
                Item::UserMessage(message) if message.delivery != Some(Delivery::Steer) => {
                    Some(message.id)
                }
                _ => None,
            })
            .expect("the prompt");
        // The person removes it.
        std::fs::remove_file(&private).unwrap();

        let (asking, client) = (core.clone(), front.client.id);
        let op = Op::SessionRewind(SessionRewindParams {
            session_id: front.session.clone(),
            to: None,
            before: Some(prompt),
            restore_files: true,
            plan: None,
        });
        let rewinding = tokio::spawn(async move { asking.handle(client, op).await });
        let question = |seen: &[Outgoing]| {
            seen.iter().find_map(|outgoing| match outgoing {
                Outgoing::Request {
                    id,
                    request: ServerRequest::Question(_),
                } => Some(id.clone()),
                _ => None,
            })
        };
        front.until(|seen| question(seen).is_some()).await;
        front.drain();
        let id = question(&front.seen.borrow()).expect("the question");
        front
            .core
            .respond(
                front.client.id,
                &id,
                ServerResponse::Question(QuestionResponse {
                    answer: Some("Put them back".to_owned()),
                }),
            )
            .unwrap();
        let done = rewinding.await.unwrap().expect("the rewind");
        assert!(matches!(done, OpResult::SessionRewind(_)));
        assert_eq!(
            world.read("private.txt").as_deref(),
            Some("kept to myself\n")
        );
        let now = mode(&private);
        assert_eq!(
            now, wanted,
            "the file was put back with mode {now:o}: other users can read it"
        );
        // The log says so, in the version the rewind went back to.
        let log = world.log();
        assert!(log.contains(r#""mode":"640""#), "{log}");
        front.close().await;
    });
}
