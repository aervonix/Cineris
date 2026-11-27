//! A session's extensions after it opens (step 28): what they were found
//! in is looked at again at each turn's start, a change has them found and
//! judged again, an extension that no longer stands approved is withdrawn
//! and one that now does is loaded; and none of it is on the path that
//! opens the session.

use std::path::{Path, PathBuf};

use cineris_ext::discover::{A_NAME_IS_LOOKED_AT, A_ROOT_IS_RESOLVED, AN_EXTENSION_IS_READ};
use cineris_ext::folder::Snapshot;
use cineris_ext::skills;
use cineris_policy::{Definition, PinStore};
use cineris_protocol::methods::{CommandsListParams, Op, OpResult, TrustResourceKind};
use cineris_testkit::Hold;
use serde_json::json;

use crate::extensions_found::{
    about_probe, cache, clock, found_probe, make_link, pin, searches, searches_times, start, state,
    users, write_extension,
};
use crate::{Answering, World, calls, says};

// ------------------------------------------------------------ a change

/// A change during a session is seen at the next turn's start, one
/// session's turns (folded for Windows' tier 2): an extension approved in
/// it (its pin written by another process, as `cineris ext add` writes it)
/// is loaded, registered first since the cache misses it; changed then, it
/// is withdrawn, its tools no longer offered, the session told why and what
/// approves it, and the changed script never evaluated.
#[test]
fn an_extension_approved_and_changed_in_a_session_is_loaded_and_withdrawn_at_the_next_turns() {
    let world = World::new("reload_approved_changed", searches_times(3));
    let folder = users(&world);
    write_extension(&folder, "", "Gives its arguments back");
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        pin(&world, &folder);
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        let made = cineris_ext::engine::runtimes_made();
        write_extension(&folder, "", "the second, and longer");
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        let told = about_probe(&front);
        assert!(
            told.iter().any(|notice| notice.contains("is withdrawn")),
            "{told:?}"
        );
        assert!(
            told.iter()
                .any(|notice| notice.contains("has changed since you approved it")),
            "{told:?}"
        );
        // The changed script was never evaluated.
        assert_eq!(cineris_ext::engine::runtimes_made(), made);
        front.close().await;
    });
}

/// The user's folder is found, and a change in it seen, wherever the
/// configuration directory is given: with `~`, relative to the workspace,
/// with `..`, and through a link, which is followed.
#[test]
fn a_change_is_seen_however_the_configuration_directory_is_given() {
    let world = World::new("reload_forms", searches_times(8));
    let real = world.home.join("cfg");
    let folder = real.join("extensions").join("probe");
    let mut forms = vec![
        PathBuf::from("~/cfg"),
        PathBuf::from("../../home/cfg"),
        world.dir.join("work").join("..").join("home").join("cfg"),
    ];
    let linked = world.home.join("linked-cfg");
    if make_link(&real, &linked) {
        forms.push(linked);
    }
    // Pinned and cached once: each round puts the folder back as it was.
    write_extension(&folder, "", "as approved");
    pin(&world, &folder);
    cache(&world, &folder);
    for given in forms {
        write_extension(&folder, "", "as approved");
        let core = world.core_with(|options| {
            let mut dirs = world.dirs();
            dirs.config = Some(given.clone());
            options.dirs = Some(dirs);
        });
        world.run(async {
            let front = start(&world, &core).await;
            front.turn("look for the probe").await;
            assert!(found_probe(&front), "{given:?}: {:?}", front.results());
            write_extension(&folder, "", "changed since it was approved");
            front.turn("look for the probe").await;
            assert!(!found_probe(&front), "{given:?}: {:?}", front.results());
            front.close().await;
        });
    }
}

// ------------------------------------------------------------ start-up

/// With no extension a session reads nothing of one: a look at each folder
/// that is not there, nothing resolved, no store or cache read or made,
/// and no runtime. The look is the trust question's at the project's (one
/// name, as there is no `.cineris`), and the session's at the user's, once
/// its loop runs; a `.cineris` with no `extensions` in it costs the
/// question a second look. With no skill, the same: the session's finding
/// looks at the first name of each of the six roots of skills, and one
/// more under a `.cineris` that is there, and resolves and reads nothing.
#[test]
fn with_no_extension_a_session_looks_once_a_folder_and_reads_nothing() {
    let world = World::new("startup_no_extension", [says("Done."), says("Done.")]);
    let looks = Hold::new(A_NAME_IS_LOOKED_AT).skipping(usize::MAX);
    let resolved = Hold::new(A_ROOT_IS_RESOLVED).skipping(usize::MAX);
    let reads = Hold::new(AN_EXTENSION_IS_READ).skipping(usize::MAX);
    let skill_looks = Hold::new(skills::A_NAME_IS_LOOKED_AT).skipping(usize::MAX);
    let skill_roots = Hold::new(skills::A_ROOT_IS_RESOLVED).skipping(usize::MAX);
    let skill_reads = Hold::new(skills::A_SKILL_IS_READ).skipping(usize::MAX);
    let made = cineris_ext::engine::runtimes_made();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    assert_eq!(
        looks.arrivals(),
        2,
        "the question's look, and the session's"
    );
    assert_eq!(skill_looks.arrivals(), 6, "one look a root of skills");
    world.project_config("");
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    assert_eq!(
        looks.arrivals(),
        5,
        "two on the way to the project's, one at the user's"
    );
    assert_eq!(resolved.arrivals(), 0);
    assert_eq!(reads.arrivals(), 0);
    assert_eq!(
        skill_looks.arrivals(),
        6 + 7,
        "and `.cineris/skills` the second time"
    );
    assert_eq!(skill_roots.arrivals(), 0);
    assert_eq!(skill_reads.arrivals(), 0);
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
    let state = state(&world);
    assert!(!state.pins_file().exists());
    assert!(!state.registration_cache_file().exists());
}

/// With an extension of the user's, nothing of it is read on the path that
/// opens the session: the session opens while every read of an extension's
/// folder is held, and the read comes after, from the session's own
/// finding, which the first turn waits for. A skill's `SKILL.md` the same:
/// read after the extensions, by the same finding.
#[test]
fn with_an_extension_nothing_of_it_is_read_until_the_session_is_open() {
    let mut world = World::new("startup_with_extension", searches());
    write_extension(&users(&world), "", "Gives its arguments back");
    let skill = world.home.join(".claude/skills/review");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Review a change.\n---\n",
    )
    .unwrap();
    pin(&world, &users(&world));
    cache(&world, &users(&world));
    // The core's blocking work on threads of their own: a read held on the
    // path that opens the session would hold the opening.
    world._order = None;
    let core = world.core();
    let reads = Hold::new(AN_EXTENSION_IS_READ);
    let skill_reads = Hold::new(skills::A_SKILL_IS_READ);
    world.run(async {
        let front = start(&world, &core).await;
        let mut waiting = std::pin::pin!(std::future::pending::<()>());
        reads.drive_until_reached(waiting.as_mut()).await;
        assert_eq!(reads.timed_out(), 0, "nothing waited for the read");
        reads.release();
        skill_reads.drive_until_reached(waiting.as_mut()).await;
        assert_eq!(skill_reads.timed_out(), 0, "nothing waited for the read");
        skill_reads.release();
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    assert_eq!(reads.arrivals(), 1);
    assert_eq!(skill_reads.arrivals(), 1);
}

/// With the three examples approved (`examples/`, each a user's extension,
/// pinned and in the registration cache, as `cineris ext add` leaves it), a
/// session starts no engine until one of them is used: opening it, a turn
/// whose call none of their hooks is on (`shell-gate`'s gate is on the
/// shell tool alone), and listing its commands, `changelog:draft` among
/// them, make no runtime; the first call of `md-outline`'s tool makes one,
/// its own. With no extension, see
/// `with_no_extension_a_session_looks_once_a_folder_and_reads_nothing`.
#[test]
fn with_the_examples_approved_no_engine_starts_until_one_is_used() {
    let world = World::new(
        "startup_examples",
        [
            calls(&[("c1", "list", json!({"path": "."}))]),
            says("Listed."),
            calls(&[(
                "c2",
                "tool_call",
                json!({"tool": "md-outline/outline", "arguments": {"path": "notes.md"}}),
            )]),
            says("Outlined."),
        ],
    );
    world.put("notes.md", "# Notes\n\n## One\n");
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for example in [
        "examples/md-outline",
        "examples/shell-gate",
        "examples/release/extensions/changelog",
    ] {
        let source = crates.parent().unwrap().join(example);
        let name = source.file_name().unwrap().to_string_lossy().into_owned();
        let folder = world.dirs().config.unwrap().join("extensions").join(&name);
        std::fs::create_dir_all(&folder).unwrap();
        for file in std::fs::read_dir(&source).unwrap() {
            let file = file.unwrap().path();
            std::fs::copy(&file, folder.join(file.file_name().unwrap())).unwrap();
        }
        crate::person().pin(
            &PinStore::new(state(&world).pins_file(), clock(&world)),
            &Definition {
                kind: TrustResourceKind::Extension,
                name,
                hash: Snapshot::read(&folder).unwrap().hash(),
            },
        );
        // One throwaway runtime each, as approving makes.
        cache(&world, &folder);
    }
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = start(&world, &core).await;
        assert_eq!(cineris_ext::engine::runtimes_made(), made, "opened");
        front.turn("list the files").await;
        assert_eq!(front.results()[0].1, "success", "{:?}", front.results());
        assert_eq!(
            cineris_ext::engine::runtimes_made(),
            made,
            "a call no hook is on"
        );
        let Ok(OpResult::CommandsList(listed)) = front
            .op(Op::CommandsList(CommandsListParams {
                session_id: front.session.clone(),
            }))
            .await
        else {
            panic!("no commands listed");
        };
        assert!(
            listed
                .commands
                .iter()
                .any(|command| command.name == "changelog:draft"),
            "{listed:?}"
        );
        assert_eq!(
            cineris_ext::engine::runtimes_made(),
            made,
            "commands listed"
        );
        front.answering(Answering::allow_all());
        front.turn("outline the notes").await;
        assert_eq!(front.results()[1].1, "success", "{:?}", front.results());
        front.close().await;
    });
    assert_eq!(
        cineris_ext::engine::runtimes_made(),
        made + 1,
        "the tool's own"
    );
}
