//! The sandbox as the core wires it (`docs/architecture/06-permissions-and-sandbox.md`,
//! "The sandbox"; `docs/decisions/0032-what-the-linux-sandbox-needs.md`):
//! the state a session reports, the levels it lets start, and on Linux
//! commands confined end to end. The tests that start a shell are named
//! `sys_…`.

use std::ffi::OsString;
use std::path::PathBuf;

use cineris_core::Outgoing;
use cineris_protocol::SessionEvent;
use cineris_protocol::event::{Event, Notice};
use cineris_protocol::lease::Level;
use cineris_protocol::methods::{InitializeParams, Op, OpResult};
use cineris_protocol::session::{SandboxStatus, SessionSettings};
use cineris_sandbox::support::Support;
use serde_json::json;

use crate::{Front, World};
// What only the tests that start a shell use, which run on Linux.
#[cfg(target_os = "linux")]
use crate::{calls, says};

/// This test binary as the sandbox launcher: it enters [`crate::launcher_entry`].
///
/// On Unix it is started through a shell, which tells the harness's own
/// git (the launcher's command is the git the harness found) from a
/// command: for git the test runner's first line goes nowhere and git's
/// output to standard error, which the entry gives back
/// ([`crate::LAUNCH_GIT`]). Anything else is started as it was, the shell
/// replaced by the launcher.
pub(crate) fn launcher() -> (PathBuf, Vec<OsString>) {
    let exe = std::env::current_exe().expect("the test binary has a path");
    #[cfg(unix)]
    if let Some(git) = cineris_base::program::git() {
        let script = format!(
            "case \" $* \" in \" -- {git} \"*) exec \"$0\" --exact launcher_entry --nocapture \
             {own} \"$@\" 2>&1 >/dev/null;; esac; exec \"$0\" --exact launcher_entry \
             --nocapture {launch} \"$@\"",
            git = git.display(),
            own = crate::LAUNCH_GIT,
            launch = crate::LAUNCH,
        );
        return (
            PathBuf::from("/bin/sh"),
            vec![OsString::from("-c"), OsString::from(script), exe.into()],
        );
    }
    (
        exe,
        ["--exact", "launcher_entry", "--nocapture", crate::LAUNCH]
            .map(OsString::from)
            .to_vec(),
    )
}

/// A session at `level`.
fn at(level: Level) -> SessionSettings {
    SessionSettings {
        level: Some(level),
        ..SessionSettings::default()
    }
}

/// What the `initialize` result says of the sandbox.
async fn initialized(front: &Front) -> SandboxStatus {
    let result = front
        .core
        .handle(
            front.client.id,
            Op::Initialize(
                serde_json::from_value::<InitializeParams>(json!({
                    "protocol_version": cineris_protocol::methods::PROTOCOL_VERSION,
                    "client": {"name": "test", "version": "0"},
                    "role": "user",
                }))
                .expect("initialize parameters"),
            ),
        )
        .await
        .expect("initialize");
    let OpResult::Initialize(result) = result else {
        panic!("not the result of initialize");
    };
    result.capabilities.sandbox
}

/// Waits for the session's notices until one says what `wanted` looks for,
/// and returns them all. The notices are sent when the session opens.
async fn notices_until(front: &Front, wanted: impl Fn(&str) -> bool) -> Vec<String> {
    fn notices_in(seen: &[Outgoing]) -> Vec<String> {
        seen.iter()
            .filter_map(|outgoing| match outgoing {
                Outgoing::Event(SessionEvent {
                    event: Event::Notice(Notice::Info(info)),
                    ..
                }) => Some(info.message.clone()),
                Outgoing::Event(SessionEvent {
                    event: Event::Notice(Notice::Sandbox(sandbox)),
                    ..
                }) => Some(sandbox.message.clone()),
                _ => None,
            })
            .collect()
    }
    front
        .until(|seen| notices_in(seen).iter().any(|notice| wanted(notice)))
        .await;
    notices_in(&front.seen.borrow())
}

#[test]
fn a_kernel_without_landlock_is_reported_and_refuses_autonomous() {
    let world = World::new("no_landlock", []);
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::NoLandlock);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        assert_eq!(initialized(&front).await, SandboxStatus::Off);
        let notices = notices_until(&front, |notice| {
            notice.starts_with("sandbox: off.") && notice.contains("no Landlock")
        })
        .await;
        assert!(!notices.is_empty());
        let refused = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .err()
        .expect("autonomous is refused");
        assert_eq!(refused.code, "sandbox_required");
        assert!(
            refused.message.contains("no Landlock"),
            "{}",
            refused.message
        );
    });
}

#[test]
fn an_old_landlock_does_not_count_and_says_what_it_lacks() {
    let world = World::new("old_landlock", []);
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::Landlock(2));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        notices_until(&front, |notice| {
            notice.contains("version 2") && notice.contains("empty any file")
        })
        .await;
        let refused = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .err()
        .expect("autonomous is refused");
        assert_eq!(refused.code, "sandbox_required");
    });
}

#[test]
fn the_sandbox_turned_off_or_without_a_launcher_is_off() {
    let world = World::new("turned_off", []);
    world.user_config("[sandbox]\nenabled = false\n");
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::Landlock(8));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        notices_until(&front, |notice| notice.contains("sandbox.enabled = false")).await;
    });
    // A core given no launcher has nothing to confine with.
    let world = World::new("no_launcher", []);
    let core = world.core_with(|options| options.sandbox = Some(Support::Landlock(8)));
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        assert_eq!(initialized(&front).await, SandboxStatus::Off);
    });
}

#[cfg(target_os = "linux")]
#[test]
fn a_landlock_with_a_gap_enforces_and_names_the_gap() {
    let world = World::new("landlock_gap", []);
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::Landlock(4));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Enforcing);
        assert_eq!(initialized(&front).await, SandboxStatus::Enforcing);
        let notices = notices_until(&front, |notice| {
            notice.starts_with("sandbox:") && notice.contains("signal")
        })
        .await;
        assert!(
            !notices
                .iter()
                .any(|notice| notice.starts_with("sandbox: off."))
        );
        Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("autonomous starts where the sandbox enforces");
    });
}

/// Whether the sandbox enforces on this machine. On CI's Linux runners it
/// must: there a kernel without it fails the test rather than skipping it.
#[cfg(target_os = "linux")]
pub(crate) fn enforcing_here() -> bool {
    let enforcing = matches!(
        cineris_sandbox::support::detect().assess(),
        cineris_sandbox::support::Assessment::Enforcing { .. }
    );
    assert!(
        enforcing || std::env::var_os("CI").is_none(),
        "CI must run the sandbox's tests on a kernel where it enforces"
    );
    enforcing
}

/// A command that writes inside the workspace, into its temporary
/// directory and outside both, and says where its temporary directory is.
#[cfg(target_os = "linux")]
fn writes(outside: &std::path::Path) -> String {
    format!(
        "echo in > inside.txt; printf %s \"$TMPDIR\" > tmpdir.txt; echo t > \"$TMPDIR/t\" && \
         cat \"$TMPDIR/t\" > from-tmp.txt; echo x > '{}/f'",
        outside.display()
    )
}

#[cfg(target_os = "linux")]
#[test]
fn sys_commands_are_confined_where_the_sandbox_enforces() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("confined", []);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": writes(&outside)}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    let temp = world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Enforcing);
        assert_eq!(initialized(&front).await, SandboxStatus::Enforcing);
        front.turn_allowing("write three files").await;

        // The approval said the command would run inside the sandbox.
        let approvals = front.approvals();
        assert_eq!(approvals.len(), 1, "{approvals:?}");
        let cineris_protocol::methods::ApprovalDetail::Command(detail) = &approvals[0].detail
        else {
            panic!("not a command's approval: {:?}", approvals[0].detail);
        };
        assert!(!detail.outside_sandbox);

        // Inside the workspace and the temporary directory: written.
        assert_eq!(world.read("inside.txt").as_deref(), Some("in\n"));
        assert_eq!(world.read("from-tmp.txt").as_deref(), Some("t\n"));
        let temp = PathBuf::from(world.read("tmpdir.txt").expect("the command said"));
        assert!(
            temp.ends_with(format!("cineris-{}", front.session)),
            "{}",
            temp.display()
        );
        assert!(temp.is_dir());
        assert!(cineris_base::private::is_private(&temp).unwrap());
        // Outside: refused, and the model is told which write.
        assert!(!outside.join("f").exists());
        let results = front.results();
        assert!(
            results[0].2.contains("The sandbox refused a write to")
                && results[0].2.contains(&outside.display().to_string()),
            "{results:?}"
        );

        // `autonomous` can start.
        Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("autonomous starts where the sandbox enforces");
        core.shutdown().await;
        temp
    });
    // The temporary directory goes with the session.
    assert!(!temp.exists(), "{}", temp.display());
}

#[cfg(target_os = "linux")]
#[test]
fn sys_at_unrestricted_a_command_is_not_confined() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("unrestricted", []);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": writes(&outside)}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Unrestricted),
        )
        .await
        .expect("the session starts");
        // The machine's sandbox enforces; this session's commands are not
        // put in it, and its state says so.
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        front.turn_allowing("write three files").await;
        assert_eq!(
            std::fs::read_to_string(outside.join("f")).ok().as_deref(),
            Some("x\n")
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn sys_a_command_leaves_the_sandbox_only_once_the_user_allows_it() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("leave", []);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    // `tee` names the file as an argument: the analysis lets it run at
    // `autonomous`, and the sandbox is what refuses it.
    let write = format!("echo x | tee '{}/f'", outside.display());
    // At `autonomous`: the same command runs unasked in the sandbox and is
    // refused there, then is asked about outside it and runs.
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": write}))]));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": write, "sandbox": false}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        let asked = front.turn_allowing("write outside").await;
        assert_eq!(
            asked,
            vec!["shell".to_owned()],
            "only the call that leaves is asked"
        );
        let approvals = front.approvals();
        let cineris_protocol::methods::ApprovalDetail::Command(detail) = &approvals[0].detail
        else {
            panic!("not a command's approval: {:?}", approvals[0].detail);
        };
        assert!(detail.outside_sandbox);
        assert_eq!(
            approvals[0].choices,
            vec![
                cineris_protocol::methods::ApprovalChoice::AllowOnce,
                cineris_protocol::methods::ApprovalChoice::Deny
            ]
        );
        let results = front.results();
        assert!(
            results[0].2.contains("\"sandbox\": false"),
            "the model is told the way out: {results:?}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("f")).ok().as_deref(),
            Some("x\n")
        );
    });
}

/// The first assistant message's id: where a rewind goes back to.
#[cfg(target_os = "linux")]
fn first_message(front: &Front) -> cineris_protocol::ItemId {
    front
        .items()
        .iter()
        .find_map(|item| match item {
            cineris_protocol::Item::AssistantMessage(message) => Some(message.id.clone()),
            _ => None,
        })
        .expect("an assistant message")
}

/// What a confined command changes among the protected paths is put back
/// at once, before anything else runs, and the turn stops (since the third
/// security fix: before it the change stayed until the turn was undone).
/// The model is told how a change the user wants is made: outside the
/// sandbox, which the user allows once, or by the user.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_protected_path_a_command_changes_is_put_back_and_stops_the_turn() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("protected", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    // A script that writes git's hook and configuration without its
    // command line naming either: the analysis cannot see it.
    world.put(
        "fix.sh",
        "echo evil > .git/hooks/pre-commit\nprintf '[core]\\n\\tfsmonitor = ./run\\n' > .git/config\n",
    );
    world.provider.push(says("Ready."));
    // The command is the last call of its response.
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    // What the model would do next if it were asked again.
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo after > after.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("get ready").await;
        let ended = front.turn("fix it").await;
        // The turn ended at the command: the model was not asked again.
        assert_eq!(world.provider.remaining(), 2, "the model was asked again");
        assert!(
            front.approvals().is_empty(),
            "nothing was asked at autonomous"
        );

        // Found, put back and shown.
        let results = front.results();
        let shown = &results[0].2;
        assert!(
            shown.contains("changed protected paths")
                && shown.contains("`.git/hooks/pre-commit` (created)")
                && shown.contains("`.git/config` (modified)")
                && shown.contains("They were put back as they were before it")
                && shown.contains("`sandbox: false`"),
            "{results:?}"
        );
        assert_eq!(world.read(".git/config").as_deref(), Some("[core]\n"));
        assert!(world.read(".git/hooks/pre-commit").is_none());
        assert!(world.read(".git/hooks/pre-commit.sample").is_some());
        // The turn, and the `turn.completed` front ends were sent, say
        // `denied`.
        assert_eq!(
            ended.outcome,
            cineris_protocol::event::TurnOutcome::Denied,
            "{ended:?}"
        );
        assert!(world.read("after.txt").is_none());
        front.close().await;
    });
}

#[cfg(target_os = "linux")]
#[test]
fn sys_past_its_bound_the_look_at_protected_paths_warns_and_does_not_stop() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("protected_bound", []);
    world.put(".git/config", "[core]\n");
    world.put("a/b/c/file.txt", "x\n");
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo one > one.txt"}),
    )]));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo two > two.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.protect_limits = Some(cineris_sandbox::protect::Limits {
            directories: 1,
            ..crate::protect_limits()
        });
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        let ended = front.turn("write two files").await;
        let results = front.results();
        assert!(
            results[0].2.contains("looked at only in part"),
            "{results:?}"
        );
        // A warning, not a stop: the next call ran, and the turn ended as
        // the model ended it.
        assert_eq!(world.read("two.txt").as_deref(), Some("two\n"));
        assert_eq!(
            ended.outcome,
            cineris_protocol::event::TurnOutcome::Completed,
            "{ended:?}"
        );
    });
}

/// The workspace, and its temporary folder, cannot be left out of the
/// grant: a workspace that holds the harness's own files (here the program
/// started as the launcher) still turns the sandbox off for the session,
/// and says why (step 26; what is still true since the third security fix,
/// which leaves out a `sandbox.write` entry that does instead:
/// [`a_write_entry_that_holds_the_harness_files_is_left_out_and_the_sandbox_stays_on`]).
#[test]
fn a_workspace_that_holds_the_harness_files_is_not_confined() {
    let world = World::new("own_files", []);
    let program = world.workspace.join("tools").join("cineris");
    std::fs::create_dir_all(program.parent().unwrap()).unwrap();
    std::fs::write(&program, "the launcher").unwrap();
    let core = world.core_with(|options| {
        options.launcher = Some((program.clone(), Vec::new()));
        options.sandbox = Some(Support::Landlock(8));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        notices_until(&front, |notice| notice.contains("own configuration")).await;
        let refused = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .err()
        .expect("autonomous is refused");
        assert_eq!(refused.code, "sandbox_required");
    });
}

#[cfg(target_os = "linux")]
#[test]
fn sys_with_no_landlock_a_command_runs_unconfined() {
    let world = World::new("unconfined_no_landlock", []);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": writes(&outside)}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::NoLandlock);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn_allowing("write three files").await;
        assert_eq!(
            std::fs::read_to_string(outside.join("f")).ok().as_deref(),
            Some("x\n"),
            "{:?}",
            front.results()
        );
    });
}

#[cfg(target_os = "linux")]
#[test]
fn sys_a_change_of_level_takes_the_sandbox_with_it() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("level_change", []);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": writes(&outside)}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Unrestricted),
        )
        .await
        .expect("the session starts");
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        let result = front
            .op(Op::SessionSet(
                cineris_protocol::methods::SessionSetParams {
                    session_id: front.session.clone(),
                    settings: at(Level::Autonomous),
                },
            ))
            .await
            .expect("the level is raised");
        let OpResult::SessionSet(result) = result else {
            panic!("not the result of session/set");
        };
        assert_eq!(result.session.sandbox, SandboxStatus::Enforcing);
        front.turn_allowing("write three files").await;
        // Confined now: the write outside is refused.
        assert!(!outside.join("f").exists(), "{:?}", front.results());
    });
}

/// After a change to a protected path is put back, the look kept for the
/// next confined command is the one taken after the put-back: the next
/// command runs, and the put-back is not taken for a change from outside
/// (the third security fix).
#[cfg(target_os = "linux")]
#[test]
fn sys_after_a_put_back_the_next_command_runs() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("after_put_back", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    world.put("fix.sh", "echo evil > .git/hooks/pre-commit\n");
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo two > two.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("fix it").await;
        assert!(world.read(".git/hooks/pre-commit").is_none());
        let ended = front.turn("go on").await;
        let results = front.results();
        assert_eq!(
            world.read("two.txt").as_deref(),
            Some("two\n"),
            "{results:?}"
        );
        assert!(
            !results[1].2.contains("changed since the last command"),
            "{results:?}"
        );
        assert_eq!(
            ended.outcome,
            cineris_protocol::event::TurnOutcome::Completed,
            "{ended:?}"
        );
        front.close().await;
    });
}

/// A confined command plants a hook and makes `.git/hooks` searchable and
/// not readable, which git still finds the hook in by its name: the hook is
/// found, named, and put back, and the folder gets its mode back (found by
/// the early review of the third security fix).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_hook_a_command_hides_in_a_folder_it_made_unreadable_is_put_back() {
    use std::os::unix::fs::PermissionsExt;
    if !enforcing_here() {
        return;
    }
    let world = World::new("unreadable_hooks", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    let hooks = world.workspace.join(".git/hooks");
    let was = std::fs::metadata(&hooks).unwrap().permissions().mode() & 0o7777;
    world.put(
        "fix.sh",
        "printf '#!/bin/sh\\necho ran > ran.txt\\n' > .git/hooks/pre-commit\n\
         chmod 755 .git/hooks/pre-commit\n\
         chmod 333 .git/hooks\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    let shown = world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("fix it").await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    let mode = std::fs::metadata(&hooks).unwrap().permissions().mode() & 0o7777;
    let _ = std::fs::set_permissions(&hooks, std::fs::Permissions::from_mode(0o755));
    assert!(
        shown.contains("`.git/hooks/pre-commit` (created)") && shown.contains("They were put back"),
        "{shown}"
    );
    assert!(!hooks.join("pre-commit").exists(), "{shown}");
    assert_eq!(mode, was, "{shown}");
}

/// While a confined command runs, a protected path outside every root it
/// may write changes (the user's own `sandbox.protect` folder, by the
/// user's hand or a program of theirs): it is not the command's, so it is
/// not put back, and it is told as a change the command could not have made
/// (found by the early review of the third security fix).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_change_outside_the_commands_reach_is_not_put_back() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("outside_reach", []);
    world.put(".git/config", "[core]\n");
    let dotfiles = world.dir.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::write(dotfiles.join("config"), "mine, as it was\n").unwrap();
    world.user_config(&format!(
        "[sandbox]\nprotect = [\"{}\"]\n",
        write_entry(&dotfiles)
    ));
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo hi > hi.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    let looks = cineris_testkit::Hold::new(cineris_core::BEFORE_THE_LOOK_AFTER);
    let shown = world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        let mut turn = std::pin::pin!(front.turn("go"));
        looks.drive_until_reached(turn.as_mut()).await;
        std::fs::write(dotfiles.join("config"), "mine, edited meanwhile\n").unwrap();
        std::fs::write(dotfiles.join("installed"), "a program of the user's\n").unwrap();
        looks.release();
        turn.await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    assert_eq!(
        std::fs::read_to_string(dotfiles.join("config")).unwrap(),
        "mine, edited meanwhile\n",
        "{shown}"
    );
    assert!(dotfiles.join("installed").exists(), "{shown}");
    assert!(
        shown.contains("protected paths it could not write changed")
            && !shown.contains("They were put back"),
        "{shown}"
    );
}

/// The user saves an edit to a protected file inside the workspace (a CI
/// workflow, in their editor) while a confined command runs. The put-back
/// cannot tell it from the command's and puts the file back, but keeps what
/// the file held in the session's folder, owner-only, and the note says
/// where (the maintainers' settlement after the full review of the third
/// security fix: the put-back never destroys what it replaces).
#[cfg(target_os = "linux")]
#[test]
fn sys_what_the_put_back_writes_over_is_kept_in_the_sessions_folder() {
    use std::os::unix::fs::PermissionsExt;
    if !enforcing_here() {
        return;
    }
    let world = World::new("kept_copy", []);
    world.put(".git/config", "[core]\n");
    world.put(".github/workflows/ci.yml", "on: push\n");
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo hi > hi.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    let looks = cineris_testkit::Hold::new(cineris_core::BEFORE_THE_LOOK_AFTER);
    let shown = world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        let mut turn = std::pin::pin!(front.turn("go"));
        looks.drive_until_reached(turn.as_mut()).await;
        std::fs::write(
            world.workspace.join(".github/workflows/ci.yml"),
            "on: [push, pull_request]\n",
        )
        .unwrap();
        looks.release();
        turn.await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    assert_eq!(
        world.read(".github/workflows/ci.yml").as_deref(),
        Some("on: push\n")
    );
    assert!(
        shown.contains("kept in the session's folder")
            && shown.contains("`.github/workflows/ci.yml` as `put-back/001-ci.yml`"),
        "{shown}"
    );
    fn find(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find(&path, name) {
                    return Some(found);
                }
            } else if path.ends_with(name) {
                return Some(path);
            }
        }
        None
    }
    let copy = find(&world.sessions_dir(), "put-back/001-ci.yml").expect("the kept copy");
    assert_eq!(
        std::fs::read_to_string(&copy).unwrap(),
        "on: [push, pull_request]\n"
    );
    assert_eq!(
        std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

/// A session kept in memory only (`--no-session`), with `settings`.
#[cfg(target_os = "linux")]
async fn start_kept_in_memory(
    world: &World,
    core: &cineris_core::Core,
    settings: SessionSettings,
) -> Front {
    let client = core.connect(cineris_core::ClientInfo::user());
    let open = core.handle(
        client.id,
        Op::SessionNew(cineris_protocol::methods::SessionNewParams {
            cwd: None,
            workspace: world.workspace.to_str().unwrap().to_owned(),
            settings,
            title: None,
            config: serde_json::Map::new(),
            ephemeral: true,
        }),
    );
    let (result, read) = crate::not_now(core, &client, open).await;
    let OpResult::SessionNew(result) = result.expect("the session starts") else {
        panic!("not the result of session/new");
    };
    Front {
        core: core.clone(),
        client,
        session: result.session.session_id.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::new(Vec::new()),
        answering: std::cell::RefCell::new(crate::Answering::default()),
    }
}

/// A session kept in memory (`--no-session`) has nowhere to keep what a
/// file holds, so a protected file a confined command changed is not put
/// back. The stop says so first, plainly; and the next turn's start says it
/// again while the change is still there, at `unrestricted` too, where the
/// model's own git runs outside the sandbox, until the file is put back by
/// hand (the maintainers' settlement after the full review of the third
/// security fix).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_session_kept_in_memory_says_what_it_could_not_put_back_and_says_it_again() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("kept_nowhere", []);
    world.put(".git/config", "[core]\n");
    world.put(
        "plant.sh",
        "printf '[core]\\n\\tfsmonitor = ./evil.sh\\n' > .git/config\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh plant.sh"}))]));
    world.provider.push(says("Next."));
    world.provider.push(says("Again."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = start_kept_in_memory(&world, &core, at(Level::Autonomous)).await;
        front.turn("plant").await;
        let shown = front.results()[0].2.clone();
        let first = "[`.git/config` was changed by a command run in the sandbox and was not put \
                     back: this session keeps nothing on disk, so what the file holds now could \
                     not be kept first. Git, and so the model's own git commands, will run what \
                     it now names, as will any other tool that reads it. Put it back by hand \
                     before continuing.]";
        let said = shown.find(first).expect("the note says it");
        let rest = shown
            .find("[The command changed protected paths")
            .expect("and then the rest");
        assert!(said < rest, "{shown}");
        assert!(
            world
                .read(".git/config")
                .unwrap_or_default()
                .contains("fsmonitor")
        );
        // The next turn, at `unrestricted`, is told again at its start.
        let again = |notices: Vec<String>| -> usize {
            notices
                .iter()
                .filter(|notice| {
                    notice.starts_with("Still not put back. `.git/config` was changed")
                })
                .count()
        };
        set_level(&front, Level::Unrestricted).await;
        front.turn("go on").await;
        assert_eq!(again(front.notices()), 1, "{:?}", front.notices());
        // Put back by hand: nothing more is said.
        world.put(".git/config", "[core]\n");
        front.turn("and again").await;
        assert_eq!(again(front.notices()), 1, "{:?}", front.notices());
        front.close().await;
    });
}

/// Sets the session's level, as a front end does.
#[cfg(target_os = "linux")]
async fn set_level(front: &Front, level: Level) {
    front
        .op(Op::SessionSet(
            cineris_protocol::methods::SessionSetParams {
                session_id: front.session.clone(),
                settings: at(level),
            },
        ))
        .await
        .expect("the level is set");
}

/// One command in a session at `first`, whose level is set to `between`
/// once the command's confinement is decided and its look before taken, and
/// set back to `first` once it has run, before the look after it: each
/// change made while the command is held at the core's point. The command,
/// a script that names no path, writes a file outside the workspace and a
/// git hook. What it gives: whether the file outside was written, what the
/// command's result said, and the hook as it is at the end.
#[cfg(target_os = "linux")]
fn level_changed_twice(
    test: &str,
    first: &Level,
    between: &Level,
) -> (bool, String, Option<String>) {
    let world = World::new(test, []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.put(
        "act.sh",
        format!(
            "echo x > '{}/f'\necho evil > .git/hooks/pre-commit\n",
            outside.display()
        ),
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh act.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    let starts = cineris_testkit::Hold::new(cineris_core::BEFORE_THE_COMMAND_STARTS);
    let looks = cineris_testkit::Hold::new(cineris_core::BEFORE_THE_LOOK_AFTER);
    let shown = world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(first.clone()),
        )
        .await
        .expect("the session starts");
        let mut turn = std::pin::pin!(front.turn("go"));
        starts.drive_until_reached(turn.as_mut()).await;
        set_level(&front, between.clone()).await;
        starts.release();
        looks.drive_until_reached(turn.as_mut()).await;
        set_level(&front, first.clone()).await;
        looks.release();
        turn.await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    (
        outside.join("f").exists(),
        shown,
        world.read(".git/hooks/pre-commit"),
    )
}

/// The open item's order: `unrestricted` when the command's looks before it
/// are taken, lowered before it starts, raised again before the look after.
/// It runs as it was decided, as it is, and is looked at as one that ran as
/// it is: its start does not take the lowered level's launcher, which would
/// have run it confined with no look at the protected paths it then wrote
/// (the third security fix; `outstanding`'s
/// `a-command-started-confined-between-two-changes-of-level`).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_command_decided_as_it_is_runs_as_it_is_when_the_level_is_lowered_then_raised() {
    if !enforcing_here() {
        return;
    }
    let (wrote_outside, shown, hook) =
        level_changed_twice("decided_as_it_is", &Level::Unrestricted, &Level::Autonomous);
    assert!(
        wrote_outside,
        "the command ran confined, where its looks were taken as for one that runs as it is: \
         {shown}"
    );
    assert!(!shown.contains("protected paths"), "{shown}");
    // At `unrestricted` what a command writes with the user's rights is not
    // looked at: the user gave those rights.
    assert_eq!(hook.as_deref(), Some("evil\n"));
}

/// The other order: confined when the looks before it are taken, raised to
/// `unrestricted` before it starts, lowered again before the look after. It
/// runs confined, as it was decided, and what it wrote in a protected path
/// is found and put back.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_command_decided_confined_runs_confined_when_the_level_is_raised_then_lowered() {
    if !enforcing_here() {
        return;
    }
    let (wrote_outside, shown, hook) =
        level_changed_twice("decided_confined", &Level::Autonomous, &Level::Unrestricted);
    assert!(
        !wrote_outside,
        "the command ran as it is, where it was decided confined: {shown}"
    );
    assert!(
        shown.contains("changed protected paths") && shown.contains("They were put back"),
        "{shown}"
    );
    assert_eq!(hook, None);
}

#[cfg(target_os = "linux")]
#[test]
fn sys_a_stopped_turn_runs_no_more_commands_of_the_same_response() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("stopped_in_one_response", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    world.put("fix.sh", "echo evil > .git/hooks/pre-commit\n");
    // The calls in one response: the later ones are decided before the
    // first runs, and wait for it.
    world.provider.push(calls(&[
        ("c1", "shell", json!({"command": "sh fix.sh"})),
        ("c2", "shell", json!({"command": "echo after > after.txt"})),
        (
            "c3",
            "write",
            json!({"path": "written.txt", "content": "x\n"}),
        ),
    ]));
    world.provider.push(says("Done."));
    // What the turn reaches, as the adversarial fixture sees it.
    let reach = crate::adversary::Reach::start(&world);
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.shell_env = Some(world.shell_env(&reach.env()));
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("fix it").await;
        // Found and put back at once (since the third security fix).
        assert!(
            world.read(".git/hooks/pre-commit").is_none(),
            "{:?}",
            front.results()
        );
        assert!(world.read("after.txt").is_none(), "{:?}", front.results());
        assert!(world.read("written.txt").is_none(), "{:?}", front.results());
    });
    let reached = reach.reached(&world);
    assert_eq!(reached.protected, Vec::<String>::new(), "{reached:#?}");
    assert!(reached.nothing_outside(), "{reached:#?}");
}

/// A command swaps a protected directory and a protected file for links to
/// a folder outside the workspace. The put-back removes each link itself,
/// never what it leads to, and makes the directory and the file again; and
/// an undo after it writes nothing outside either (step 26's finding 1;
/// since the third security fix, the put-back comes first).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_link_a_command_planted_is_put_back_and_nothing_is_written_through_it() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("undo_through_link", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    let elsewhere = world.dir.join("elsewhere");
    std::fs::create_dir_all(elsewhere.join("hooks")).unwrap();
    std::fs::write(elsewhere.join("config"), "theirs\n").unwrap();
    // The command swaps the repository's directory for a link to a
    // directory outside the workspace.
    world.put(
        "swap.sh",
        format!(
            "mv .git/hooks .git/hooks.old && ln -s '{}' .git/hooks && ln -sf '{}' .git/config\n",
            elsewhere.join("hooks").display(),
            elsewhere.join("config").display()
        ),
    );
    world.provider.push(says("Ready."));
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh swap.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("get ready").await;
        let keep = first_message(&front);
        front.turn("swap it").await;
        let results = front.results();
        assert!(
            results[0].2.contains("`.git/hooks` (modified)")
                && results[0]
                    .2
                    .contains("`.git/hooks/pre-commit.sample` (deleted)"),
            "{results:?}"
        );
        assert!(
            results[0]
                .2
                .contains("They were put back as they were before it"),
            "{results:?}"
        );
        // The links are gone, and what they stood in place of is back.
        let hooks = std::fs::symlink_metadata(world.workspace.join(".git/hooks")).unwrap();
        assert!(hooks.is_dir(), "{hooks:?}");
        assert!(world.read(".git/hooks/pre-commit.sample").is_some());
        let config = std::fs::symlink_metadata(world.workspace.join(".git/config")).unwrap();
        assert!(config.is_file(), "{config:?}");
        assert_eq!(world.read(".git/config").as_deref(), Some("[core]\n"));
        front
            .op(Op::SessionRewind(
                cineris_protocol::methods::SessionRewindParams {
                    session_id: front.session.clone(),
                    to: Some(keep),
                    before: None,
                    restore_files: true,
                    plan: None,
                },
            ))
            .await
            .expect("the rewind runs");
        // Nothing outside the workspace was written by the undo.
        assert_eq!(
            std::fs::read_to_string(elsewhere.join("config")).unwrap(),
            "theirs\n"
        );
        assert!(!elsewhere.join("hooks/pre-commit.sample").exists());
        front.close().await;
    });
}

/// A `sandbox.write` entry as the configuration file writes it.
fn write_entry(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

/// A `sandbox.write` entry that holds the harness's own files is left out
/// of the grant, with a notice that names it, and the session keeps the
/// sandbox enforcing (the third security fix; `outstanding`'s
/// `a-projects-write-root-holding-the-harness-files-turns-the-sandbox-off`;
/// decision 0042). Before it, each of these turned the sandbox off for the
/// whole session (step 26's second review and its findings 9 and 1). One
/// world and a core for each case, folded for Windows' tier 2:
/// - the user's own entry `~`, which holds the configuration, state and
///   data directories;
/// - the user's own entry `~/.cargo`, with the launcher in `~/.cargo/bin`;
/// - the user's own entry inside the configuration directory;
/// - a trusted project's own entry `~/.cargo`, with the launcher there: the
///   open item's case, where a project's file switched the sandbox off;
/// - on Unix, a dotfiles folder that the user's configuration file is a
///   link into.
#[test]
fn a_write_entry_that_holds_the_harness_files_is_left_out_and_the_sandbox_stays_on() {
    #[derive(Clone, Copy)]
    enum Case {
        Home,
        Launcher,
        InsideConfig,
        ProjectsLauncher,
        LinkedConfig,
    }
    let cases: &[Case] = if cfg!(unix) {
        &[
            Case::Home,
            Case::Launcher,
            Case::InsideConfig,
            Case::ProjectsLauncher,
            Case::LinkedConfig,
        ]
    } else {
        &[
            Case::Home,
            Case::Launcher,
            Case::InsideConfig,
            Case::ProjectsLauncher,
        ]
    };
    for (index, case) in cases.iter().enumerate() {
        let world = World::new(&format!("left_out_{index}"), []);
        let program = world.home.join(".cargo").join("bin").join("cineris");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, "the launcher").unwrap();
        let (entry, shown) = match case {
            Case::Home => ("~".to_owned(), "`~`".to_owned()),
            Case::Launcher | Case::ProjectsLauncher => {
                ("~/.cargo".to_owned(), "`~/.cargo`".to_owned())
            }
            Case::InsideConfig => {
                let inside = world
                    .dirs()
                    .config
                    .expect("a configuration directory")
                    .join("inside");
                std::fs::create_dir_all(&inside).unwrap();
                (write_entry(&inside), "inside`".to_owned())
            }
            Case::LinkedConfig => {
                let dotfiles = world.dir.join("dotfiles");
                std::fs::create_dir_all(&dotfiles).unwrap();
                (write_entry(&dotfiles), "dotfiles`".to_owned())
            }
        };
        let sandbox = format!("[sandbox]\nwrite = [\"{entry}\"]\n");
        if matches!(case, Case::ProjectsLauncher) {
            world.project_config(&sandbox);
        } else {
            world.user_config(&sandbox);
        }
        #[cfg(unix)]
        if matches!(case, Case::LinkedConfig) {
            let named = world.dirs().user_config.expect("a configuration file");
            let target = world.dir.join("dotfiles").join("config.toml");
            std::fs::rename(&named, &target).unwrap();
            std::os::unix::fs::symlink(&target, &named).unwrap();
        }
        let core = world.core_with(|options| {
            options.launcher = Some((program.clone(), Vec::new()));
            options.sandbox = Some(Support::Landlock(8));
            options.trust_project = true;
        });
        world.run(async {
            let front = Front::start_as(
                &world,
                &core,
                cineris_core::ClientInfo::user(),
                at(Level::Autonomous),
            )
            .await
            .expect("the session starts at autonomous: the sandbox enforces");
            assert_eq!(front.state.sandbox, SandboxStatus::Enforcing, "{entry}");
            let notices = notices_until(&front, |notice| notice.contains("is left out")).await;
            let said = notices
                .iter()
                .find(|notice| notice.contains("is left out"))
                .unwrap();
            assert!(
                said.starts_with("`sandbox.write` names `")
                    && said.contains(&shown)
                    && said.contains("the sandbox stays on"),
                "{entry}: {said}"
            );
            front.close().await;
        });
    }
}

/// Where the sandbox does not enforce (here a kernel without Landlock, as on
/// Windows), nothing is confined for an entry to undo: nothing is left out,
/// and nothing is said of it, as before the third security fix.
#[test]
fn where_the_sandbox_does_not_enforce_no_write_entry_is_left_out() {
    let world = World::new("left_out_not_enforcing", [crate::says("Hello.")]);
    world.user_config("[sandbox]\nwrite = [\"~\"]\n");
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::NoLandlock);
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert_eq!(front.state.sandbox, SandboxStatus::Off);
        notices_until(&front, |notice| notice.contains("Landlock")).await;
        // A turn, so that every notice of the session's start has been sent.
        front.turn("hello").await;
        let notices = notices_until(&front, |notice| notice.contains("Landlock")).await;
        assert!(
            !notices.iter().any(|notice| notice.contains("is left out")),
            "{notices:?}"
        );
        front.close().await;
    });
}

/// A reload that adds a `sandbox.write` entry holding the harness's own
/// files leaves that entry out, says so, and applies the rest: the session
/// keeps the sandbox enforcing (the third security fix; before it the
/// reload was refused whole, `harness_files_writable`).
#[test]
fn a_reload_that_adds_a_write_entry_holding_the_harness_files_leaves_it_out() {
    let world = World::new("left_out_reload", []);
    let program = world.home.join(".cargo").join("bin").join("cineris");
    std::fs::create_dir_all(program.parent().unwrap()).unwrap();
    std::fs::write(&program, "the launcher").unwrap();
    let core = world.core_with(|options| {
        options.launcher = Some((program.clone(), Vec::new()));
        options.sandbox = Some(Support::Landlock(8));
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        world.user_config("[sandbox]\nwrite = [\"~/.cargo\", \"build\"]\n");
        let reloaded = front
            .op(Op::ConfigReload(
                cineris_protocol::methods::ConfigReloadParams::default(),
            ))
            .await
            .expect("the reload answers");
        let OpResult::ConfigReload(reloaded) = reloaded else {
            panic!("not the result of config/reload");
        };
        assert!(reloaded.errors.is_empty(), "{reloaded:?}");
        assert!(
            reloaded.applied.iter().any(|key| key == "sandbox.write"),
            "{reloaded:?}"
        );
        let notices = notices_until(&front, |notice| notice.contains("is left out")).await;
        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("`~/.cargo`") && notice.contains("is left out")),
            "{notices:?}"
        );
        front.close().await;
    });
}

/// The open item's case end to end, where the sandbox enforces: of two
/// `sandbox.write` entries, the one inside the harness's configuration
/// directory is left out and the other granted. A confined command writes
/// in the second and is refused in the first, which a session that turned
/// the sandbox off (before the third security fix) would have let it
/// write.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_write_entry_left_out_grants_nothing_and_the_rest_is_granted() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("left_out_grant", []);
    let inside = world
        .dirs()
        .config
        .expect("a configuration directory")
        .join("inside");
    let extra = world.dir.join("extra");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&extra).unwrap();
    world.user_config(&format!(
        "[sandbox]\nwrite = [\"{}\", \"{}\"]\n",
        write_entry(&inside),
        write_entry(&extra)
    ));
    // A script, so that the line names neither folder: the engine refuses a
    // line that names the harness's own files before the sandbox is reached.
    world.put(
        "act.sh",
        format!(
            "echo g > '{}/g'\necho f > '{}/f'\n",
            extra.display(),
            inside.display()
        ),
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh act.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        assert_eq!(front.state.sandbox, SandboxStatus::Enforcing);
        front.turn_allowing("write both").await;
        let results = front.results();
        assert_eq!(
            std::fs::read_to_string(extra.join("g")).ok().as_deref(),
            Some("g\n"),
            "the entry kept was not granted: {results:?}"
        );
        assert!(
            !inside.join("f").exists(),
            "a confined command wrote in an entry that holds the harness's files: {results:?}"
        );
        front.close().await;
    });
}

#[cfg(target_os = "linux")]
#[test]
fn sys_a_command_that_makes_the_look_stop_short_stops_the_turn() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("look_made_short", []);
    world.put(".git/config", "[core]\n");
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "for i in $(seq 60); do mkdir -p \"junk/d$i\"; done"}),
    )]));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo after > after.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.protect_limits = Some(cineris_sandbox::protect::Limits {
            directories: 40,
            ..crate::protect_limits()
        });
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("make directories").await;
        let results = front.results();
        assert!(
            results[0].2.contains("where the look before it did not"),
            "{results:?}"
        );
        assert!(world.read("after.txt").is_none(), "{results:?}");
    });
}

/// While commands are confined, a command may stop the turn while a
/// question is open, and the answer is then to stop: nothing after the
/// question that changes anything starts until it is answered.
#[test]
fn while_commands_are_confined_a_later_call_waits_for_the_open_question() {
    assert!(
        !crate::approvals::later_write_ran_while_the_question_was_open(
            "confined_question",
            |options| {
                options.launcher = Some(launcher());
                options.sandbox = Some(Support::Landlock(8));
            }
        )
    );
}

/// A protected path that changes between two confined commands — from a
/// process an earlier command left running that the kill could not clear,
/// or from the user's own hand — is found by the next command's look,
/// compared with the look after the last command, and stops the turn, even
/// though the kill reported no escape (found by the third review of step
/// 26). The change from outside is simulated by writing the hook between
/// the two turns, which is what an escaped process would do; this holds
/// whichever way the kill race went.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_change_between_two_commands_is_found_and_stops_the_turn() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("between_commands", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    world.provider.push(says("Ready."));
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo one > one.txt"}),
    )]));
    world.provider.push(says("First done."));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo after > after.txt"}),
    )]));
    world.provider.push(calls(&[(
        "c3",
        "shell",
        json!({"command": "echo more > more.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("get ready").await;
        // The first command runs and is looked at; nothing is wrong yet.
        front.turn("first").await;
        assert_eq!(world.read("one.txt").as_deref(), Some("one\n"));

        // Between the turns, a process an earlier command left running — or
        // the user — writes a hook.
        world.put(".git/hooks/pre-commit", "evil\n");

        // The next command's look is compared with the look after the
        // first: the hook is found, and the turn stops before the command
        // runs.
        let ended = front.turn("second").await;
        let results = front.results();
        let shown = results.last().map(|one| one.2.clone()).unwrap_or_default();
        assert!(
            shown.contains("changed since the last command")
                && shown.contains("`.git/hooks/pre-commit` (created)")
                && shown.contains("none of the harness's own tools changed it"),
            "{results:?}"
        );
        assert_eq!(
            ended.outcome,
            cineris_protocol::event::TurnOutcome::Denied,
            "{ended:?}"
        );
        // Neither command of the stopped turn ran, and the turn ended at the
        // refused command, the last of its response: the model was not
        // asked again.
        assert!(world.read("after.txt").is_none(), "the first command ran");
        assert!(world.read("more.txt").is_none(), "a later command ran");
        assert_eq!(world.provider.remaining(), 2, "the model was asked again");
        front.close().await;
    });
}

/// A protected path the harness's own tools changed between two confined
/// commands — an `edit` or `write` in one turn, a confined command in the
/// next — is not reported by the continuity check: it is the harness's, and
/// already handled where the tool changed it (found by the third review of
/// step 26).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_harness_edit_between_commands_is_not_reported() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("harness_edit", []);
    world.put(".git/config", "[core]\n");
    // The workflow directory already exists, so the harness's `write` adds
    // only the file to it, not the directory.
    world.put(".github/workflows/keep.yml", "on: push\n");
    world.provider.push(says("Ready."));
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo one > one.txt"}),
    )]));
    world.provider.push(says("First done."));
    // The harness's own `write` changes a protected path.
    world.provider.push(calls(&[(
        "w1",
        "write",
        json!({"path": ".github/workflows/ci.yml", "content": "on: push\njobs: {}\n"}),
    )]));
    world.provider.push(says("Wrote."));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo after > after.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("get ready").await;
        front.turn("first command").await;
        // The edit is approved, and changes the protected file.
        front.turn_allowing("edit the workflow").await;
        assert_eq!(
            world.read(".github/workflows/ci.yml").as_deref(),
            Some("on: push\njobs: {}\n"),
            "the edit did not land"
        );
        // The next confined command: the continuity check finds the config
        // changed, but the harness's own tool changed it, so it is not
        // reported and the command runs.
        front.turn("next command").await;
        let results = front.results();
        let shown = results.last().map(|one| one.2.clone()).unwrap_or_default();
        assert!(
            !shown.contains("changed since the last command"),
            "the harness's own edit was reported from outside: {results:?}"
        );
        assert_eq!(world.read("after.txt").as_deref(), Some("after\n"));
        front.close().await;
    });
}

/// A command the user let leave the sandbox changes a protected path, with
/// approval. The next confined command must not report that approved change
/// as one from outside: the continuity baseline is dropped across an
/// unconfined command (found by the independent review of the step 26
/// fixes).
#[cfg(target_os = "linux")]
#[test]
fn sys_an_unconfined_command_does_not_trip_the_next_command() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("unconfined_between", []);
    world.put(".git/config", "[core]\n");
    world.provider.push(says("Ready."));
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo one > one.txt"}),
    )]));
    world.provider.push(says("First done."));
    // Approved, and run outside the sandbox: it changes a protected path.
    world.provider.push(calls(&[(
        "u1",
        "shell",
        json!({"command": "printf 'x\\n' >> .git/config", "sandbox": false}),
    )]));
    world.provider.push(says("Changed."));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo after > after.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("get ready").await;
        front.turn("first command").await;
        front.turn_allowing("run outside the sandbox").await;
        assert!(
            world.read(".git/config").unwrap().contains('x'),
            "the unconfined command did not change the config"
        );
        let ended = front.turn("next command").await;
        let shown = front
            .results()
            .last()
            .map(|one| one.2.clone())
            .unwrap_or_default();
        assert!(
            !shown.contains("changed since the last command")
                && !shown.contains("none of the harness's own tools changed it"),
            "the approved unconfined change was reported from outside: {shown:?}"
        );
        assert_eq!(
            ended.outcome,
            cineris_protocol::event::TurnOutcome::Completed
        );
        assert_eq!(world.read("after.txt").as_deref(), Some("after\n"));
        front.close().await;
    });
}

/// A session of this world opened again, as a second run of the program
/// does.
#[cfg(target_os = "linux")]
async fn reopened(core: &cineris_core::Core, session: &cineris_protocol::SessionId) -> Front {
    let client = core.connect(cineris_core::ClientInfo::user());
    let open = core.handle(
        client.id,
        Op::SessionOpen(cineris_protocol::methods::SessionRef {
            session_id: session.clone(),
        }),
    );
    let (opened, read) = crate::not_now(core, &client, open).await;
    let OpResult::SessionOpen(opened) = opened.expect("the session opens") else {
        panic!("not the result of session/open");
    };
    Front {
        core: core.clone(),
        client,
        session: session.clone(),
        state: opened.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::default(),
        answering: std::cell::RefCell::default(),
    }
}

/// Each close that finds a change from outside appends a notice with an id
/// of its own. Two such closes, then a compaction at once, whose summary
/// covers through the second notice: the history a later run reads back
/// begins as the one the compacting run built. The review of step 15, its
/// second note, found the notice's fixed id (`outside-change`) making the
/// read-back find the first notice, and replay the turn between them.
#[cfg(target_os = "linux")]
#[test]
fn sys_two_closes_that_find_changes_from_outside_and_a_compaction_read_back_as_built() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("outside_twice", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
    world.user_config("[permissions]\nmode = \"autonomous\"\n");
    for response in [
        calls(&[("c1", "shell", json!({"command": "echo one > one.txt"}))]),
        says("One."),
        calls(&[("c2", "shell", json!({"command": "echo two > two.txt"}))]),
        says("Two."),
        says("The summary."),
        says("Three."),
        says("Four."),
    ] {
        world.provider.push(response);
    }
    let core = || world.core_with(|options| options.launcher = Some(launcher()));
    // Two runs, each with a confined command and then a protected path
    // changed by hand before the session closes.
    let first = core();
    let session = world.run(async {
        let front = Front::start(&world, &first).await;
        front.turn("one").await;
        world.put(".git/hooks/pre-commit", "by hand\n");
        front.close().await;
        front.session.clone()
    });
    let second = core();
    world.run(async {
        let front = reopened(&second, &session).await;
        front.turn("two").await;
        world.put(".git/hooks/post-commit", "by hand\n");
        front.close().await;
    });
    let notices: Vec<String> = world
        .log()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| {
            entry["kind"] == "notice"
                && entry["item"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("none of the harness's own tools"))
        })
        .map(|entry| entry["item"]["id"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(notices.len(), 2, "a notice at each close: {notices:?}");
    assert_ne!(notices[0], notices[1], "each has an id of its own");
    // A third run compacts at once, and goes on.
    let third = core();
    let built = world.run(async {
        let front = reopened(&third, &session).await;
        front
            .op(Op::SessionCompact(
                cineris_protocol::methods::SessionCompactParams {
                    session_id: front.session.clone(),
                    focus: None,
                },
            ))
            .await
            .unwrap();
        front.turns(1).await;
        front.turn("three").await;
        front.close().await;
        world.provider.requests().pop().unwrap()
    });
    let summary = crate::texts(&built.messages[0]);
    assert!(summary.contains("The summary."), "{summary}");
    assert_eq!(built.messages.len(), 2, "the summary and the prompt");
    // A fourth reads it back.
    let fourth = core();
    let read = world.run(async {
        let front = reopened(&fourth, &session).await;
        front.turn("four").await;
        front.close().await;
        world.provider.requests().pop().unwrap()
    });
    assert_eq!(
        read.messages[..built.messages.len()],
        built.messages[..],
        "the history read back is not the one built live"
    );
}
