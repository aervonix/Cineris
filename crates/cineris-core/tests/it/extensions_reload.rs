//! A session's extensions after it opens (step 28): what they were found
//! in is looked at again at each turn's start, a change has them found and
//! judged again, an extension that no longer stands approved is withdrawn
//! and one that now does is loaded; and none of it is on the path that
//! opens the session.

use std::path::PathBuf;

use cineris_ext::discover::{A_NAME_IS_LOOKED_AT, A_ROOT_IS_RESOLVED, AN_EXTENSION_IS_READ};
use cineris_protocol::methods::TrustResponse;
use cineris_testkit::Hold;

use crate::extensions_found::{
    about_probe, cache, definition, found_probe, make_link, open_answering, pin, projects,
    searches, searches_times, state, trust, users, write_extension,
};
use crate::{Front, World, says};

// ------------------------------------------------------------ a change

/// The user's extension changed during a session is withdrawn at the next
/// turn's start: its tools are no longer offered, and the session is told
/// why and what approves it.
#[test]
fn a_users_extension_changed_in_a_session_is_withdrawn_at_the_next_turn() {
    let world = World::new("reload_changed", searches_times(2));
    let folder = users(&world);
    write_extension(&folder, "", "the first");
    pin(&world, &folder);
    cache(&world, &folder);
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
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
        front.close().await;
    });
    // The changed script was never evaluated.
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
}

/// An extension approved during a session (its pin written by another
/// process, as `cineris ext add` writes it) is loaded at the next turn's
/// start, registered first since the cache misses it.
#[test]
fn an_extension_approved_in_a_session_is_loaded_at_the_next_turn() {
    let world = World::new("reload_approved", searches_times(2));
    let folder = users(&world);
    write_extension(&folder, "", "Gives its arguments back");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        pin(&world, &folder);
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
}

/// A change is seen wherever the configuration directory is given: with
/// `~`, relative to the workspace, with `..`, and through a link.
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
    for (round, given) in forms.into_iter().enumerate() {
        write_extension(&folder, "", &format!("round {round}"));
        pin(&world, &folder);
        cache(&world, &folder);
        let core = world.core_with(|options| {
            let mut dirs = world.dirs();
            dirs.config = Some(given.clone());
            options.dirs = Some(dirs);
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("look for the probe").await;
            assert!(found_probe(&front), "{given:?}: {:?}", front.results());
            write_extension(&folder, "", &format!("round {round}, changed since"));
            front.turn("look for the probe").await;
            assert!(!found_probe(&front), "{given:?}: {:?}", front.results());
            front.close().await;
        });
    }
}

/// A project's folder of extensions swapped during a session for a link
/// out of the workspace is not followed: the extension is withdrawn, and
/// the session says the folder leads out.
#[test]
fn a_projects_folder_swapped_for_a_link_out_is_withdrawn() {
    let world = World::new("reload_link_out", searches_times(2));
    let folder = projects(&world);
    write_extension(&folder, "", "the projects");
    trust(&world, &[definition(&folder)]);
    cache(&world, &folder);
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, TrustResponse::default()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        let inside = world.workspace.join(".cineris/extensions");
        let outside = world.dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::rename(&inside, outside.join("extensions")).unwrap();
        if make_link(&outside.join("extensions"), &inside) {
            front.turn("look for the probe").await;
            assert!(!found_probe(&front), "{:?}", front.results());
            assert!(
                front
                    .notices()
                    .iter()
                    .any(|notice| notice.contains("leads out of the workspace")),
                "{:?}",
                front.notices()
            );
            assert!(
                about_probe(&front)
                    .iter()
                    .any(|notice| notice.contains("is withdrawn")),
                "{:?}",
                front.notices()
            );
        }
        front.close().await;
    });
}

// ------------------------------------------------------------ start-up

/// With no extension a session reads nothing of one: a look at each folder
/// that is not there, nothing resolved, no store or cache read or made,
/// and no runtime. The look is the trust question's at the project's (one
/// name, as there is no `.cineris`), and the session's at the user's, once
/// its loop runs; a `.cineris` with no `extensions` in it costs the
/// question a second look.
#[test]
fn with_no_extension_a_session_looks_once_a_folder_and_reads_nothing() {
    let world = World::new("startup_no_extension", [says("Done."), says("Done.")]);
    let looks = Hold::new(A_NAME_IS_LOOKED_AT).skipping(usize::MAX);
    let resolved = Hold::new(A_ROOT_IS_RESOLVED).skipping(usize::MAX);
    let reads = Hold::new(AN_EXTENSION_IS_READ).skipping(usize::MAX);
    let made = cineris_ext::engine::runtimes_made();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    assert_eq!(
        looks.arrivals(),
        2,
        "the question's look, and the session's"
    );
    world.project_config("");
    world.run(async {
        let front = Front::start(&world, &core).await;
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
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
    let state = state(&world);
    assert!(!state.pins_file().exists());
    assert!(!state.registration_cache_file().exists());
}

/// With an extension of the user's, nothing of it is read on the path that
/// opens the session: the session opens while every read of an extension's
/// folder is held, and the read comes after, from the session's own
/// finding, which the first turn waits for.
#[test]
fn with_an_extension_nothing_of_it_is_read_until_the_session_is_open() {
    let mut world = World::new("startup_with_extension", searches());
    write_extension(&users(&world), "", "Gives its arguments back");
    pin(&world, &users(&world));
    cache(&world, &users(&world));
    // The core's blocking work on threads of their own: a read held on the
    // path that opens the session would hold the opening.
    world._order = None;
    let core = world.core();
    let reads = Hold::new(AN_EXTENSION_IS_READ);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let mut waiting = std::pin::pin!(std::future::pending::<()>());
        reads.drive_until_reached(waiting.as_mut()).await;
        assert_eq!(reads.timed_out(), 0, "nothing waited for the read");
        reads.release();
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    assert_eq!(reads.arrivals(), 1);
}
