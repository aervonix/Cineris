//! The harness's own git, inside the sandbox while commands are confined
//! (`docs/architecture/06-permissions-and-sandbox.md`, "The harness's own
//! git"; found by the third review of step 26).
//!
//! A confined command may write a repository's configuration: it lies in
//! the workspace. What it writes can name a program for git to run (here a
//! clean filter, which `git status` and `git diff` run on a file whose
//! times no longer match the index). The look for what a command changed,
//! the base state and a file's earlier content all ask git, so each git
//! the harness starts runs inside the sandbox, with no write root and no
//! network: the filter runs confined, as the command did.
//!
//! Every test here starts a shell or git, so each is named `sys_…`, and
//! each needs a kernel where the sandbox enforces.

#![cfg(target_os = "linux")]

use std::path::Path;

use cineris_protocol::Item;
use cineris_protocol::lease::Level;
use cineris_protocol::methods::{ConfigReloadParams, Op, OpResult};
use cineris_protocol::session::{SandboxStatus, SessionSettings};
use cineris_sandbox::support::Support;
use cineris_testkit::scratch::git;
use serde_json::{Value, json};

use crate::sandbox::{enforcing_here, launcher};
use crate::{Front, World, calls, says};

/// A session at `level`.
fn at(level: Level) -> SessionSettings {
    SessionSettings {
        level: Some(level),
        ..SessionSettings::default()
    }
}

/// The world's workspace as a repository with one commit of `tracked.txt`.
fn repository(world: &World) {
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
}

/// Names `clean` as the clean filter of every file in the repository's
/// configuration, and makes the tracked file's times no longer match the
/// index, so that git must read it again through the filter.
fn plant_filter(world: &World, clean: &str, required: bool) {
    let config = world.workspace.join(".git/config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(&format!(
        "[filter \"x\"]\n\tclean = {clean}\n\trequired = {required}\n"
    ));
    std::fs::write(&config, text).unwrap();
    world.put(".gitattributes", "* filter=x\n");
    stale(&world.workspace.join("tracked.txt"));
}

/// Sets a file's modification time to the epoch.
fn stale(file: &Path) {
    std::fs::File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH)
        .unwrap();
}

/// The payload of the first `shell` result.
fn shell_payload(front: &Front) -> Value {
    front
        .items()
        .iter()
        .find_map(|item| match item {
            Item::ToolResult(result) if result.tool == "shell" => result.payload.clone(),
            _ => None,
        })
        .expect("a shell result with a payload")
}

/// The review's case: a confined command plants a filter in a trusted
/// repository's configuration, and the look that follows it asks git what
/// changed. The filter runs, inside the sandbox, and cannot write outside;
/// the look at protected paths then stops the turn.
#[test]
fn sys_a_filter_a_confined_command_plants_runs_inside_the_sandbox() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("planted_filter", []);
    repository(&world);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    // A script, so that the command line names no protected path and
    // nothing is asked.
    world.put(
        "fix.sh",
        format!(
            "echo direct > '{o}/direct'\n\
             printf '[filter \"x\"]\\n\\tclean = touch {o}/planted; cat\\n' >> .git/config\n\
             echo '* filter=x' > .gitattributes\n\
             touch -d @0 tracked.txt\n",
            o = outside.display()
        ),
    );
    world.provider.push(says("Ready."));
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
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
        .expect("the session starts");
        assert_eq!(front.state.sandbox, SandboxStatus::Enforcing);
        front.turn("get ready").await;
        front.turn("go").await;
        let results = front.results();
        assert!(
            !outside.join("direct").exists(),
            "the confined command wrote outside: {results:?}"
        );
        assert!(
            results[0].2.contains("changed protected paths"),
            "{results:?}"
        );
        // The look asked git, and git asked the filter.
        assert_eq!(shell_payload(&front)["changes"]["method"], "git");
        assert!(
            !outside.join("planted").exists(),
            "the planted filter ran outside the sandbox: {results:?}"
        );
        front.close().await;
    });
}

/// The control for the shell path: where the sandbox does not enforce,
/// where the after-look git is not confined, the same planted filter does
/// run and writes outside. That is what shows the filter is one the
/// harness's own git runs, so that its absence in the confined tests means
/// the sandbox stopped it, not that the filter never ran. (Until the third
/// security fix the control was `unrestricted`: there the harness's git now
/// runs inside the sandbox too.)
#[test]
fn sys_where_the_sandbox_does_not_enforce_the_after_look_git_runs_the_planted_filter() {
    let world = World::new("planted_filter_unconfined", []);
    repository(&world);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.put(
        "fix.sh",
        format!(
            "printf '[filter \"x\"]\\n\\tclean = touch {o}/planted; cat\\n' >> .git/config\n\
             echo '* filter=x' > .gitattributes\n\
             touch -d @0 tracked.txt\n",
            o = outside.display()
        ),
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.sandbox = Some(Support::NoLandlock);
        options.trust_project = true;
    });
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
        front.turn("go").await;
        assert_eq!(shell_payload(&front)["changes"]["method"], "git");
        assert!(
            outside.join("planted").exists(),
            "the after-look git did not run the filter even unconfined: {:?}",
            front.results()
        );
        front.close().await;
    });
}

/// Moves the session to `level`.
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

/// The script that plants the clean filter `touch <outside>/planted` in the
/// repository's configuration (or in `config`, a file it names), sends
/// every file through it, and makes the tracked file stale, so that the
/// next `git status` runs the filter.
fn planting(outside: &Path, config: &str) -> String {
    format!(
        "printf '[filter \"x\"]\\n\\tclean = touch {o}/planted; cat\\n' >> {config}\n\
         echo '* filter=x' > .gitattributes\n\
         touch -d @0 tracked.txt\n",
        o = outside.display()
    )
}

/// The open item's case, end to end (the third security fix;
/// `outstanding`'s `a-filter-a-confined-program-plants-runs-unconfined-later`):
/// a confined command plants a clean filter in a trusted repository's
/// configuration. The look after it puts the configuration back and stops
/// the turn. The user then moves the session to `unrestricted`, and the
/// next turn runs a command, whose looks ask git, and the model's own `git
/// status`, which runs with the user's rights: the filter's program never
/// runs, because nothing names it any more.
/// `sys_where_the_sandbox_does_not_enforce_the_after_look_git_runs_the_planted_filter`
/// shows the same filter is one git runs.
#[test]
fn sys_a_filter_a_confined_command_plants_never_runs_afterwards() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("planted_then_unrestricted", []);
    repository(&world);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.put("fix.sh", planting(&outside, ".git/config"));
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "echo two > two.txt"}),
    )]));
    world
        .provider
        .push(calls(&[("c3", "shell", json!({"command": "git status"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
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
        .expect("the session starts");
        front.turn("fix it").await;
        let results = front.results();
        assert!(
            results[0].2.contains("`.git/config` (modified)")
                && results[0].2.contains("They were put back"),
            "{results:?}"
        );
        set_level(&front, Level::Unrestricted).await;
        front.turn("go on").await;
        let results = front.results();
        assert_eq!(results.len(), 3, "{results:?}");
        assert!(
            !outside.join("planted").exists(),
            "the filter a confined command planted ran with the user's rights: {results:?}"
        );
        assert!(
            !world
                .read(".git/config")
                .unwrap_or_default()
                .contains("filter"),
            "{:?}",
            world.read(".git/config")
        );
        front.close().await;
    });
}

/// Since the third security fix the harness's own git runs inside the
/// sandbox wherever it enforces, at every level, `unrestricted` included,
/// whoever wrote what its configuration names. Here a filter is written at
/// `unrestricted` itself, by a command that runs as it is (nothing is
/// looked at or put back there: the user gave those rights), and the looks
/// around it ask git: the filter git runs for them runs inside the sandbox,
/// and cannot write outside. Before the fix the look after ran it with the
/// user's rights (`sys_where_the_sandbox_does_not_enforce_the_after_look_git_runs_the_planted_filter`
/// is that control); what a confined start writes is put back besides
/// (`sys_a_filter_a_confined_command_plants_never_runs_afterwards`).
#[test]
fn sys_at_unrestricted_the_harness_git_still_runs_inside_the_sandbox() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("harness_git_unrestricted", []);
    repository(&world);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    world.put("fix.sh", planting(&outside, ".git/config"));
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
        options.trust_project = true;
    });
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
        front.turn("go").await;
        // The look after asked git, and git asked the filter.
        assert_eq!(shell_payload(&front)["changes"]["method"], "git");
        assert!(
            !outside.join("planted").exists(),
            "the harness's git ran, at unrestricted, a filter with the user's rights: {:?}",
            front.results()
        );
        front.close().await;
    });
}

/// The same filter, already in the configuration when the session starts:
/// the base state asks git about every changed file, and its git is the
/// sandbox's too, at every level where the sandbox enforces, `unrestricted`
/// included (since the third security fix). Where the sandbox does not
/// enforce, the same filter does run: that is what shows it is one git
/// runs.
#[test]
fn sys_the_base_state_is_taken_by_a_git_inside_the_sandbox() {
    if !enforcing_here() {
        return;
    }
    let ran = |level: Level, name: &str, enforcing: bool| -> bool {
        let world = World::new(name, []);
        repository(&world);
        let marker = world.dir.join("filter-ran");
        plant_filter(&world, &format!("touch {}; cat", marker.display()), false);
        world.provider.push(says("Hello."));
        let core = world.core_with(|options| {
            options.launcher = Some(launcher());
            options.trust_project = true;
            options.base_capture = true;
            if !enforcing {
                options.sandbox = Some(Support::NoLandlock);
            }
        });
        let capture = crate::BaseCapture::watch();
        world.run(async {
            let front = Front::start_as(&world, &core, cineris_core::ClientInfo::user(), at(level))
                .await
                .expect("the session starts");
            front.turn("hello").await;
            // Closing waits for the base state only so long: it is waited
            // for here.
            capture.taken().await;
            front.close().await;
            let lines = front.log(&world);
            let base = lines
                .iter()
                .find(|line| line["kind"] == "base")
                .expect("the base state was recorded");
            assert_eq!(base["trusted"], true, "{base}");
            // The base state was taken, not given up: git answered inside
            // the sandbox (the filter failed to write but `cat` passed the
            // content, so `git status` and the patch still answered). A
            // reading launcher that always failed would leave it incomplete.
            assert_eq!(base["complete"], true, "{base}");
        });
        marker.exists()
    };
    assert!(
        !ran(Level::Autonomous, "base_filter_confined", true),
        "the base state's git ran a filter outside the sandbox"
    );
    assert!(
        !ran(Level::Unrestricted, "base_filter_unrestricted", true),
        "at `unrestricted` the base state's git ran a filter outside the sandbox"
    );
    assert!(
        ran(Level::Unrestricted, "base_filter_unconfined", false),
        "the control: where the sandbox does not enforce, git runs the filter"
    );
}

/// A filter that must write (as git-lfs's does) fails inside the sandbox,
/// so git gives no answer: the list of what the command changed then says
/// it is incomplete, and why, in the result's payload and its text.
#[test]
fn sys_a_git_that_fails_inside_the_sandbox_leaves_the_list_incomplete_and_says_why() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("failing_filter", []);
    repository(&world);
    plant_filter(&world, "touch .git/filter-wrote && cat", true);
    world.provider.push(calls(&[(
        "c1",
        "shell",
        json!({"command": "echo new > new.txt"}),
    )]));
    world.provider.push(says("Done."));
    let core = world.core_with(|options| {
        options.launcher = Some(launcher());
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
        .expect("the session starts");
        front.turn("go").await;
        let results = front.results();
        let changes = &shell_payload(&front)["changes"];
        assert_eq!(changes["complete"], false, "{changes}");
        assert_eq!(
            changes["incomplete"],
            cineris_tools::shell::Incomplete::GitFailedConfined.as_str(),
            "{changes}"
        );
        assert!(
            results[0]
                .2
                .contains("The list of files the command changed is incomplete: git exited"),
            "{results:?}"
        );
        assert_eq!(world.read("new.txt").as_deref(), Some("new\n"));
        assert!(
            !world.workspace.join(".git/filter-wrote").exists(),
            "the harness's git wrote in the workspace"
        );
        front.close().await;
    });
}

/// What the repository's configuration names for git to run or read is
/// looked at with the protected paths, and what a confined command changes
/// there is put back (step 28's review of protected writes; the third
/// security fix): the folder for hooks (`core.hooksPath`, here
/// `scripts/hooks`), where a hook planted or changed would run at the next
/// `git commit` outside the sandbox; a filter's program in the workspace
/// (`./tools/strip.sh`), which the model's own `git status` would run; and
/// a file the configuration includes (`include.path`). A folder the same
/// command names through the included file is looked at by the look after
/// it, what the configuration names being read again first: the included
/// file is put back, so nothing names the folder any more, and the hook
/// planted there, in a place the look before never watched, is named to
/// put back by hand and never removed (the full review of the third
/// security fix).
#[test]
fn sys_what_the_configuration_names_that_a_confined_command_changes_is_put_back() {
    use std::os::unix::fs::PermissionsExt;
    if !enforcing_here() {
        return;
    }
    let world = World::new("hooks_path_put_back", []);
    repository(&world);
    git(
        &world.workspace,
        &["config", "core.hooksPath", "scripts/hooks"],
    );
    git(
        &world.workspace,
        &["config", "filter.x.clean", "./tools/strip.sh"],
    );
    world.put("tools/strip.sh", "#!/bin/sh\ncat\n");
    let config = world.workspace.join(".git/config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[include]\n\tpath = ../shared.gitconfig\n");
    std::fs::write(&config, text).unwrap();
    world.put("shared.gitconfig", "");
    world.put("scripts/hooks/pre-commit", "#!/bin/sh\necho ours\n");
    let hook = world.workspace.join("scripts/hooks/pre-commit");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A script, so that the line names nothing.
    world.put(
        "plant.sh",
        "printf '#!/bin/sh\\nevil\\n' > scripts/hooks/pre-commit\n\
         printf '#!/bin/sh\\nevil\\n' > scripts/hooks/pre-push\n\
         chmod +x scripts/hooks/pre-push\n\
         printf '#!/bin/sh\\nevil\\n' > tools/strip.sh\n",
    );
    world.put(
        "include.sh",
        "printf '[core]\\n\\thooksPath = elsewhere\\n' > shared.gitconfig\n\
         mkdir -p elsewhere\n\
         printf '#!/bin/sh\\nevil\\n' > elsewhere/pre-commit\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh plant.sh"}))]));
    world.provider.push(calls(&[(
        "c2",
        "shell",
        json!({"command": "sh include.sh"}),
    )]));
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
        front.turn("plant").await;
        let results = front.results();
        assert!(
            results[0]
                .2
                .contains("`scripts/hooks/pre-commit` (modified)")
                && results[0].2.contains("`scripts/hooks/pre-push` (created)")
                && results[0].2.contains("`tools/strip.sh` (modified)")
                && results[0].2.contains("They were put back"),
            "{results:?}"
        );
        assert_eq!(
            world.read("scripts/hooks/pre-commit").as_deref(),
            Some("#!/bin/sh\necho ours\n")
        );
        assert!(world.read("scripts/hooks/pre-push").is_none());
        assert_eq!(
            world.read("tools/strip.sh").as_deref(),
            Some("#!/bin/sh\ncat\n")
        );
        front.turn("include").await;
        let results = front.results();
        assert!(
            results[1].2.contains("`shared.gitconfig` (modified)")
                && results[1]
                    .2
                    .contains("These were put back as they were before it: `shared.gitconfig`")
                && results[1]
                    .2
                    .contains("must be put back by hand: `elsewhere/pre-commit` (modified)"),
            "{results:?}"
        );
        assert_eq!(world.read("shared.gitconfig").as_deref(), Some(""));
        assert!(world.read("elsewhere/pre-commit").is_some());
        front.close().await;
    });
}

/// A confined command names, in the repository's configuration, a folder
/// for hooks and a file to include that the repository already keeps (`git
/// config core.hooksPath .githooks`, `git config include.path
/// ../team.gitconfig`). The look before never looked at them: what is there
/// is not taken for made, and nothing of it is deleted (found by the full
/// review of the third security fix, where both were deleted and said to be
/// put back). The configuration itself, which the command changed, is put
/// back.
#[test]
fn sys_what_a_command_names_that_was_already_there_is_not_deleted() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("named_already_there", []);
    world.put("tracked.txt", "one\n");
    world.put(".githooks/pre-commit", "#!/bin/sh\ncargo fmt --check\n");
    world.put("team.gitconfig", "[core]\n\tautocrlf = false\n");
    git(
        &world.workspace,
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(&world.workspace, &["add", "--all"]);
    git(
        &world.workspace,
        &["commit", "--quiet", "--message", "first"],
    );
    world.put(
        "setup.sh",
        "git config core.hooksPath .githooks\ngit config include.path ../team.gitconfig\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh setup.sh"}))]));
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
        front.turn("set up the hooks").await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    assert_eq!(
        world.read(".githooks/pre-commit").as_deref(),
        Some("#!/bin/sh\ncargo fmt --check\n"),
        "{shown}"
    );
    assert_eq!(
        world.read("team.gitconfig").as_deref(),
        Some("[core]\n\tautocrlf = false\n"),
        "{shown}"
    );
    assert!(
        shown.contains("`.git/config` (modified)") && !shown.contains("`team.gitconfig` (created)"),
        "{shown}"
    );
}

/// The same with the top of the working tree named as the folder for
/// hooks: only git's hook names in it are watched, so nothing of the
/// workspace is taken for made, and nothing is deleted.
#[test]
fn sys_naming_the_working_tree_for_hooks_deletes_nothing() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("named_top", []);
    world.put("tracked.txt", "one\n");
    world.put("src/main.rs", "fn main() {}\n");
    git(
        &world.workspace,
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(&world.workspace, &["add", "--all"]);
    git(
        &world.workspace,
        &["commit", "--quiet", "--message", "first"],
    );
    world.put("setup.sh", "git config core.hooksPath .\n");
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh setup.sh"}))]));
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
        front.turn("set up the hooks").await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    let gone: Vec<&str> = ["tracked.txt", "src/main.rs", ".git/HEAD", "setup.sh"]
        .into_iter()
        .filter(|path| world.read(path).is_none())
        .collect();
    assert!(gone.is_empty(), "the put-back deleted {gone:?}\n{shown}");
}

/// A confined command makes a folder of its own under `.git/hooks` that no
/// look can list, and plants a hook in the folder `core.hooksPath` names:
/// the look goes on past the folder, so the hook is found and put back,
/// and the folder, the command's, is opened and taken away with what is in
/// it (found by the full review of the third security fix, where the folder
/// ended the look and the hook was never named).
#[test]
fn sys_a_hook_planted_beside_a_folder_no_look_can_list_is_put_back() {
    use std::os::unix::fs::PermissionsExt;
    if !enforcing_here() {
        return;
    }
    let world = World::new("hidden_beside", []);
    repository(&world);
    git(
        &world.workspace,
        &["config", "core.hooksPath", "scripts/hooks"],
    );
    world.put("scripts/hooks/README", "hooks\n");
    world.put(
        "plant.sh",
        "mkdir -p .git/hooks/zz && echo x > .git/hooks/zz/x && chmod 000 .git/hooks/zz\n\
         printf '#!/bin/sh\\nevil\\n' > scripts/hooks/pre-commit\n\
         chmod +x scripts/hooks/pre-commit\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh plant.sh"}))]));
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
        front.turn("plant").await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    let zz = world.workspace.join(".git/hooks/zz");
    let _ = std::fs::set_permissions(&zz, std::fs::Permissions::from_mode(0o755));
    assert!(world.read("scripts/hooks/pre-commit").is_none(), "{shown}");
    assert!(
        shown.contains("`scripts/hooks/pre-commit` (created)"),
        "{shown}"
    );
    assert!(!zz.exists(), "{shown}");
}

/// `core.hooksPath` names a folder that is not there yet (`.githooks`): its
/// hooks are watched by name. One confined command writes a hook where
/// nothing watches (`tools/pre-commit`); the next makes the folder a link
/// to it (`ln -s tools .githooks`). The link on the way to the watched
/// names is recorded, taken for made and removed, so the next `git commit`
/// runs nothing of it (found by the narrow check of the third security
/// fix, where both turns ended with no note).
///
/// `tools/pre-commit` is first watched by the second command's look after,
/// through the link, and that look names it only when its change time is
/// at or after the look before began, less a slack: whether a file the
/// first command wrote passes that depends on the clock and the file
/// system, and on runners it has. So the order is made: its change
/// time is moved while the second command waits to start, its look before
/// taken, and the file is always named, as one first watched during the
/// command, to be put back by hand. It is never taken for made: nothing of
/// it is removed or written over.
#[test]
fn sys_a_hooks_folder_made_a_link_to_a_planted_hook_is_taken_away() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("hooks_folder_link", []);
    world.put("tracked.txt", "one\n");
    git(
        &world.workspace,
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(&world.workspace, &["config", "core.hooksPath", ".githooks"]);
    // Scripts, so that no line names a path the engine asks about.
    world.put(
        "one.sh",
        "mkdir -p tools\nprintf '#!/bin/sh\\nevil\\n' > tools/pre-commit\nchmod +x tools/pre-commit\n",
    );
    world.put("two.sh", "ln -s tools .githooks\n");
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh one.sh"}))]));
    world.provider.push(says("One."));
    world
        .provider
        .push(calls(&[("c2", "shell", json!({"command": "sh two.sh"}))]));
    world.provider.push(says("Two."));
    let core = world.core_with(|options| options.launcher = Some(launcher()));
    // The first command passes; the second waits there, its look before
    // taken.
    let starts = cineris_testkit::Hold::new(cineris_core::BEFORE_THE_COMMAND_STARTS).skipping(1);
    let hook = world.workspace.join("tools/pre-commit");
    let (shown, ended) = world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Autonomous),
        )
        .await
        .expect("the session starts");
        front.turn("one").await;
        let mut two = std::pin::pin!(front.turn("two"));
        starts.drive_until_reached(two.as_mut()).await;
        // Its mode set as it is: only its change time moves.
        let mode = std::fs::metadata(&hook).unwrap().permissions();
        std::fs::set_permissions(&hook, mode).unwrap();
        starts.release();
        let ended = two.await;
        let shown: Vec<String> = front.results().iter().map(|one| one.2.clone()).collect();
        front.close().await;
        (shown, ended)
    });
    // The link is gone, so git's hook name leads nowhere.
    assert!(
        std::fs::symlink_metadata(world.workspace.join(".githooks")).is_err(),
        "{shown:#?}"
    );
    assert!(
        !world.workspace.join(".githooks/pre-commit").exists(),
        "{shown:#?}"
    );
    // The turn stopped at the command: the model was not asked again.
    assert_eq!(
        ended.outcome,
        cineris_protocol::event::TurnOutcome::Denied,
        "{shown:#?}"
    );
    assert_eq!(world.provider.remaining(), 1, "{shown:#?}");
    // The link is named as put back, and the hook as to be put back by
    // hand: never taken for made.
    assert!(
        shown[1].contains("These were put back as they were before it: `.githooks` (created).")
            && shown[1].contains("must be put back by hand: `tools/pre-commit` (modified)."),
        "{shown:#?}"
    );
    assert_eq!(
        world.read("tools/pre-commit").as_deref(),
        Some("#!/bin/sh\nevil\n"),
        "{shown:#?}"
    );
}

/// A confined command plants a filter in a submodule's configuration and
/// then closes `.git/modules`, which the look lists and does not record, to
/// every look (`chmod 311`): nothing else changed, so nothing would be put
/// back. The folder the look before listed is given its owner's right to
/// list it back, looked at again, and the configuration is put back (found
/// by the narrow check of the third security fix).
#[test]
fn sys_a_filter_behind_a_folder_the_command_closed_is_put_back() {
    use std::os::unix::fs::PermissionsExt;
    if !enforcing_here() {
        return;
    }
    let world = World::new("closed_modules", []);
    world.put(".git/config", "[core]\n");
    world.put(".git/modules/sub/config", "[core]\n");
    world.put(".git/modules/sub/HEAD", "ref: refs/heads/main\n");
    world.put(
        "plant.sh",
        "printf '[core]\\n\\tfsmonitor = /tmp/evil\\n' > .git/modules/sub/config\n\
         chmod 311 .git/modules\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh plant.sh"}))]));
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
        front.turn("plant").await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    let modules = world.workspace.join(".git/modules");
    let mode = std::fs::metadata(&modules).unwrap().permissions().mode() & 0o777;
    std::fs::set_permissions(&modules, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(mode, 0o711, "{shown}");
    assert_eq!(
        world.read(".git/modules/sub/config").as_deref(),
        Some("[core]\n"),
        "{shown}"
    );
    assert!(
        shown.contains("given their owner's right to list them back")
            && shown.contains("`.git/modules/sub/config` (modified)"),
        "{shown}"
    );
}

/// A hooks folder that is a link from before the session (`.githooks ->
/// tools`, a common set-up): what the configuration names is watched where
/// it really is too, so a hook a confined command changes through the
/// folder's real name is found and put back.
#[test]
fn sys_a_hook_in_a_hooks_folder_that_is_a_link_is_watched_where_it_is() {
    if !enforcing_here() {
        return;
    }
    let world = World::new("hooks_folder_is_a_link", []);
    world.put("tracked.txt", "one\n");
    world.put("tools/pre-commit", "#!/bin/sh\ncargo fmt --check\n");
    std::os::unix::fs::symlink("tools", world.workspace.join(".githooks")).unwrap();
    git(
        &world.workspace,
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(&world.workspace, &["config", "core.hooksPath", ".githooks"]);
    world.put(
        "plant.sh",
        "printf '#!/bin/sh\\nevil\\n' > tools/pre-commit\n",
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh plant.sh"}))]));
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
        front.turn("plant").await;
        let shown = front.results()[0].2.clone();
        front.close().await;
        shown
    });
    assert_eq!(
        world.read("tools/pre-commit").as_deref(),
        Some("#!/bin/sh\ncargo fmt --check\n"),
        "{shown}"
    );
    assert!(shown.contains("`tools/pre-commit` (modified)"), "{shown}");
}

/// A filter named by a bare name is never one in the workspace, even
/// through an absolute directory of `PATH` (an activated `.venv/bin`):
/// the harness's git, unasked, would run a file a command wrote. The same
/// name further down `PATH`, outside the workspace, is the one that runs.
/// Found by the early review of the harness's own processes' fixes.
#[test]
fn sys_a_filter_the_harness_git_runs_is_not_found_in_the_workspace() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new("harness_git_path", []);
    repository(&world);
    let name = "cineris-planted-filter";
    let outside = world.dir.join("outside");
    let seen = outside.join("seen");
    let bin = outside.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let filter = |said: &str| format!("#!/bin/sh\necho {said} >> {}\ncat\n", seen.display());
    std::fs::write(bin.join(name), filter("outside")).unwrap();
    std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    let venv = world.workspace.join(".venv").join("bin");
    world.put(
        "fix.sh",
        format!(
            "mkdir -p .venv/bin\nprintf '#!/bin/sh\\necho planted >> {seen}\\ncat\\n' > \
             .venv/bin/{name}\nchmod 755 .venv/bin/{name}\nprintf '[filter \"x\"]\\n\\tclean = \
             {name}\\n' >> .git/config\necho '* filter=x' > .gitattributes\ntouch -d @0 \
             tracked.txt\n",
            seen = seen.display()
        ),
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let path = format!(
        "{}:{}:{}",
        venv.display(),
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // With no sandbox, as on Windows: the workspace is kept out of all the
    // same, not only where the write roots hold it.
    world.user_config("[sandbox]\nenabled = false\n");
    let core = world.core_with(|options| {
        options.trust_project = true;
        options.shell_env = Some(world.shell_env(&[("PATH", &path)]));
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Unrestricted),
        )
        .await
        .expect("the session starts");
        front.turn("go").await;
        assert_eq!(shell_payload(&front)["changes"]["method"], "git");
        let text = std::fs::read_to_string(&seen).unwrap_or_else(|_| {
            panic!(
                "the harness's git ran neither filter: {:?}",
                front.results()
            )
        });
        assert!(
            !text.contains("planted"),
            "the harness's git ran the workspace's {name}, through an absolute PATH entry inside \
             the workspace: {text}"
        );
        assert!(text.contains("outside"), "{text}");
        front.close().await;
    });
}

/// A folder a reload makes a write root is kept out of the harness's git's
/// `PATH` from then on, as the roots the session started with are: a
/// filter a command could plant there is not the one git runs (step 28,
/// which gave the harness's lookups the roots the sandbox holds now).
///
/// The filter is required, and says which one ran through git's own
/// answer: the one outside passes the content on, the planted one fails,
/// and none found fails too, so a complete list of changes means the one
/// outside ran. (Its git runs inside the sandbox at `unrestricted` since the
/// third security fix, where a filter can write nothing to say so.)
#[test]
fn sys_a_folder_a_reload_makes_writable_is_kept_out_of_the_harness_gits_path() {
    use std::os::unix::fs::PermissionsExt;
    if !enforcing_here() {
        return;
    }
    let world = World::new("harness_git_reload_avoid", []);
    repository(&world);
    let name = "cineris-planted-filter";
    let outside = world.dir.join("outside");
    let bin = outside.join("bin");
    let extra = world.dir.join("extra");
    for (dir, script) in [(&bin, "#!/bin/sh\ncat\n"), (&extra, "#!/bin/sh\nexit 1\n")] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), script).unwrap();
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    world.put(
        "fix.sh",
        format!(
            "printf '[filter \"x\"]\\n\\tclean = {name}\\n\\trequired = true\\n' >> .git/config\n\
             echo '* filter=x' > .gitattributes\ntouch -d @0 tracked.txt\n"
        ),
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let path = format!(
        "{}:{}:{}",
        extra.display(),
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let core = world.core_with(|options| {
        options.trust_project = true;
        options.launcher = Some(launcher());
        options.shell_env = Some(world.shell_env(&[("PATH", &path)]));
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Unrestricted),
        )
        .await
        .expect("the session starts");
        world.user_config(&format!("[sandbox]\nwrite = [\"{}\"]\n", extra.display()));
        let reloaded = front
            .op(Op::ConfigReload(ConfigReloadParams::default()))
            .await
            .expect("the reload answers");
        let OpResult::ConfigReload(reloaded) = reloaded else {
            panic!("not the result of config/reload");
        };
        assert!(reloaded.errors.is_empty(), "{reloaded:?}");
        front.turn("go").await;
        let changes = &shell_payload(&front)["changes"];
        assert_eq!(changes["method"], "git", "{changes}");
        assert_eq!(
            changes["complete"], true,
            "the harness's git ran the {name} in a folder the reload made writable, or found \
             none: {changes}"
        );
        front.close().await;
    });
}

/// The providers' key variables never reach a filter the harness's git
/// runs, before a reload or after one: the variable the session started
/// with as a key, and the one a reload made a key, through the one list
/// the shell's commands are kept from (the harness's own processes' fixes;
/// before them git was given the harness's whole environment). At
/// `unrestricted`, where nothing confines, so the filter can write what it
/// saw; and the filter is planted by the command, so the only git that
/// runs it is the one after the reload.
#[test]
fn sys_a_reload_withholds_the_keys_from_a_filter_the_harness_git_runs() {
    const KEY: &str = "sk-reload-git-key";
    let world = World::new("harness_git_keys", []);
    repository(&world);
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let seen = outside.join("seen");
    let filter = outside.join("filter.sh");
    std::fs::write(
        &filter,
        format!(
            "#!/bin/sh\necho \"${{OLD_PROVIDER_KEY-unset}} ${{NEW_PROVIDER_KEY-unset}} \
             ${{KEPT-unset}}\" >> {seen}\ncat\n",
            seen = seen.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&filter, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    world.put(
        "fix.sh",
        format!(
            "printf '[filter \"x\"]\\n\\tclean = {filter}\\n' >> .git/config\n\
             echo '* filter=x' > .gitattributes\n\
             touch -d @0 tracked.txt\n",
            filter = filter.display()
        ),
    );
    world
        .provider
        .push(calls(&[("c1", "shell", json!({"command": "sh fix.sh"}))]));
    world.provider.push(says("Done."));
    let provider = |variable: &str| {
        format!(
            "[providers.mine]\nprotocol = \"anthropic-messages\"\nbase_url = \
             \"https://llm.example.invalid\"\nkey_env = \"{variable}\"\n"
        )
    };
    world.user_config(&provider("OLD_PROVIDER_KEY"));
    let core = world.core_with(|options| {
        options.trust_project = true;
        options.shell_env = Some(world.shell_env(&[
            ("OLD_PROVIDER_KEY", KEY),
            ("NEW_PROVIDER_KEY", KEY),
            ("KEPT", "yes"),
        ]));
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::user(),
            at(Level::Unrestricted),
        )
        .await
        .expect("the session starts");
        world.user_config(&provider("NEW_PROVIDER_KEY"));
        let reloaded = front
            .op(Op::ConfigReload(ConfigReloadParams::default()))
            .await
            .expect("the reload answers");
        let OpResult::ConfigReload(reloaded) = reloaded else {
            panic!("not the result of config/reload");
        };
        assert!(reloaded.errors.is_empty(), "{reloaded:?}");
        front.turn("go").await;
        assert_eq!(shell_payload(&front)["changes"]["method"], "git");
        let text = std::fs::read_to_string(&seen).unwrap_or_else(|_| {
            panic!(
                "the harness's git did not run the filter: {:?}",
                front.results()
            )
        });
        for line in text.lines() {
            let [old, new, kept] = line.split(' ').collect::<Vec<_>>()[..] else {
                panic!("{text}");
            };
            assert_eq!(
                kept, "yes",
                "the filter was not given the environment the harness's git inherits: {text}"
            );
            assert_eq!(
                new, "unset",
                "the variable a reload made a key reached a filter the harness's git ran: the \
                 list the reload sets did not reach git\n{text}"
            );
            assert_eq!(
                old, "unset",
                "the variable that was a key when the session started reached a filter the \
                 harness's git ran after a reload\n{text}"
            );
        }
        front.close().await;
    });
}
