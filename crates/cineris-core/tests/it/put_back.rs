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

fn git(dir: &Path, arguments: &[&str]) -> bool {
    std::process::Command::new("git")
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

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
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

/// A tracked script a command removed, with the folder it was in, put back
/// by an undo: as git makes it on a checkout, executable. `repository` is
/// where git's repository is: the workspace, or the folder above it, so
/// that git is asked about a workspace that is not the top of its
/// repository (git names its paths from the workspace then too).
fn an_undo_puts_back_a_removed_script(test: &str, repository: fn(&World) -> &Path) {
    let world = World::new(
        test,
        [
            says("Ready."),
            calls(&[(
                "c1",
                "shell",
                json!({"command": "rm -r bin ./tools/-dash.sh"}),
            )]),
            says("Removed."),
        ],
    );
    world.put("bin/run.sh", "#!/bin/sh\necho hi\n");
    world.put("tools/-dash.sh", "#!/bin/sh\necho dash\n");
    world.put("notes.txt", "kept\n");
    let script = world.workspace.join("bin/run.sh");
    let dash = world.workspace.join("tools/-dash.sh");
    for executable in [&script, &dash] {
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let top = repository(&world);
    if !(git(top, &["init", "-q"])
        && git(top, &["add", "."])
        && git(top, &["commit", "-q", "-m", "first"]))
    {
        return;
    }
    let core = world.core_with(|options| {
        options.trust_project = true;
        options.shell = None;
    });
    let umask = umask(&world);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        front.turn_allowing("remove them").await;
        assert!(!script.exists() && !dash.exists(), "not removed");
        front
            .op(Op::SessionUndo(SessionUndoParams {
                session_id: front.session.clone(),
                restore_files: true,
            }))
            .await
            .expect("the undo");
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
                0o777 & !umask,
                "{} was put back with mode {now:o}: it is no longer executable",
                executable.display()
            );
        }
        front.close().await;
    });
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

/// A private file (mode `0600`) the turn edited with `write` and the person
/// removed after it, put back by a rewind the person said yes to: it comes
/// back private (found by the review of step 25: it came back with the
/// process's default mode, readable by others). The path the model names
/// is not the canonical one: through `.` and a folder and back.
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
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn_allowing("change it").await;
        assert_eq!(
            world.read("private.txt").as_deref(),
            Some("kept to myself, changed\n")
        );
        assert_eq!(mode(&private), 0o600, "the write kept the mode");
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
            now, 0o600,
            "the private file was put back with mode {now:o}: other users can read it"
        );
        // The log says so, in the version the rewind went back to.
        let log = world.log();
        assert!(log.contains(r#""mode":"600""#), "{log}");
        front.close().await;
    });
}
