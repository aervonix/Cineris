//! What the early review of step 28's second part found in how a session
//! finds its extensions and how the trust question lists them, each held
//! by a test: one reading of the trust store shapes the question; the
//! author's words cannot pose as the harness's; a user's folder takes its
//! name whether or not it could be used; each folder is read once; a turn
//! waits for the tools and not for the cache; and the rules the code
//! guarded with no test.

use std::path::Path;

use cineris_ext::cache::Entry;
use cineris_ext::discover::AN_EXTENSION_IS_READ;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::Registration;
use cineris_policy::{Definition, Standing};
use cineris_protocol::methods::{TrustRequest, TrustResourceKind, TrustResponse};
use cineris_protocol::model::ModelRole;
use cineris_testkit::{Hold, Response};
use serde_json::json;

use crate::extensions_found::{
    about_probe, cache, cache_file, definition, found_probe, make_link, open_answering, pin,
    project_key, projects, searches, start, state, trust, trust_store, users, write_extension, yes,
};
use crate::{MODEL, OTHER_MODEL, World, calls, says, settle};

/// Writes the extension `name` in `folder`, whose tool `echo` is described
/// as `said`.
pub(crate) fn write_named(folder: &Path, name: &str, said: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"{name}\"\nversion = \"1\"\ndescription = \"x\"\nmain = \"main.js\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        format!(
            "cineris.tool({{ name: 'echo', description: '{said}', input: {{ type: 'object' }}, \
             run: (args) => ({{ got: args }}) }});"
        ),
    )
    .unwrap();
}

/// The definition of the extension `name` in `folder`, as it is now.
pub(crate) fn named_definition(folder: &Path, name: &str) -> Definition {
    Definition {
        name: name.to_owned(),
        ..definition(folder)
    }
}

/// A hold that stops nothing at `point` and counts who arrives there.
pub(crate) fn counting(point: &str) -> Hold {
    Hold::new(point).skipping(usize::MAX)
}

pub(crate) fn not_now() -> TrustResponse {
    TrustResponse::default()
}

pub(crate) fn keep() -> TrustResponse {
    TrustResponse {
        keep: true,
        ..TrustResponse::default()
    }
}

/// The detail of the one extension the one question listed.
pub(crate) fn listed_detail(asked: &[TrustRequest]) -> String {
    let [request] = asked else {
        panic!("one question: {asked:?}");
    };
    let listed: Vec<_> = request
        .resources
        .iter()
        .filter(|resource| resource.kind == TrustResourceKind::Extension)
        .collect();
    let [resource] = listed.as_slice() else {
        panic!("one extension: {:?}", request.resources);
    };
    resource.detail.clone()
}

/// A turn that looks for the tools of `probe` and of `tracker`.
pub(crate) fn searches_both() -> [Response; 2] {
    [
        calls(&[
            ("s1", "tool_search", json!({"query": "probe/"})),
            ("s2", "tool_search", json!({"query": "tracker/"})),
        ]),
        says("Done."),
    ]
}

// ------------------------------------------------------------ one reading

/// The trust question is shaped by the one reading of the store that the
/// session's trust was worked out from: trust taken away between that
/// reading and the question turns on nothing the question did not show
/// (the early review: a second reading made the question one of a project
/// not trusted, listing none of the settings held back, and a yes turned
/// them on).
#[test]
fn a_trust_taken_away_while_the_question_is_shaped_turns_on_nothing_unshown() {
    let mut world = World::new("found_trust_revoked", searches());
    // A model of the project's, which waits for trust.
    world.project_config(&format!("[model]\nmain = \"{OTHER_MODEL}\"\n"));
    write_extension(&projects(&world), "", "the projects");
    trust(&world, &[]);
    // The core's blocking work on threads of their own, so that the look
    // can be held while the test goes on.
    world._order = None;
    let core = world.core();
    let shaped = Hold::new(cineris_core::ext::THE_QUESTIONS_TRUST_IS_READ);
    world.run(async {
        let mut opening = std::pin::pin!(open_answering(&world, &core, yes()));
        shaped.drive_until_reached(opening.as_mut()).await;
        assert!(trust_store(&world).revoke(&project_key(&world)).unwrap());
        shaped.release();
        let (front, asked) = opening.await;
        for request in &asked {
            assert!(
                request
                    .resources
                    .iter()
                    .all(|resource| resource.kind != TrustResourceKind::Settings),
                "a trusted project's question lists no setting: {request:?}"
            );
        }
        assert_eq!(
            front.state.roles[&ModelRole::Main].model.as_str(),
            MODEL,
            "the project's model, never shown, is not turned on"
        );
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
}

// ------------------------------------------------------------ in words

// ------------------------------------------------------------ the user's names

/// A user's extension left out (a link in its folder) still takes its
/// name, in any case: a trusted project's `probe` is not offered, not
/// loaded, and the session says why (the early review: the user's rules
/// about `probe` would have applied to the project's).
#[test]
fn a_users_extension_left_out_keeps_its_name_from_a_projects() {
    let world = World::new("found_name_left_out", searches());
    let mine = world
        .dirs()
        .config
        .unwrap()
        .join("extensions")
        .join("Probe");
    write_named(&mine, "Probe", "one of mine");
    let elsewhere = world.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let linked = make_link(&elsewhere, &mine.join("link"));
    // The project's, trusted and approved as it is, and in the cache.
    let theirs = projects(&world);
    write_extension(&theirs, "", "one of theirs");
    trust(&world, &[definition(&theirs)]);
    cache(&world, &theirs);
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].contains("is not used"), "{told:?}");
        if linked {
            assert!(told[0].contains("though it was left out"), "{told:?}");
            assert!(
                front
                    .notices()
                    .iter()
                    .any(|notice| notice.contains("Probe") && notice.contains("link")),
                "the user's own was left out, and said: {:?}",
                front.notices()
            );
        }
        front.close().await;
    });
}

// ------------------------------------------------------------ read once

/// Each extension's folder is read once on a session's way: the project's
/// by the trust question, which the session takes on, and the user's by
/// the session alone, the question only listing the user's folder for the
/// names it takes (the early review: both were read whole twice). Each
/// skill's `SKILL.md` is read once too, the user's and the project's, by
/// the session's finding alone.
#[test]
fn each_folder_is_read_once_when_a_session_opens() {
    let world = World::new("found_read_once", searches_both());
    for root in [
        world.home.join(".claude/skills/review"),
        world.workspace.join(".cineris/skills/pdf"),
    ] {
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            "---\nname: a skill\ndescription: One.\n---\n",
        )
        .unwrap();
    }
    let skill_reads = counting(cineris_ext::skills::A_SKILL_IS_READ);
    write_extension(&users(&world), "", "one of mine");
    pin(&world, &users(&world));
    cache(&world, &users(&world));
    let theirs = world.workspace.join(".cineris/extensions/tracker");
    write_named(&theirs, "tracker", "one of theirs");
    trust(&world, &[named_definition(&theirs, "tracker")]);
    cache(&world, &theirs);
    let core = world.core();
    let reads = counting(AN_EXTENSION_IS_READ);
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.turn("look for both").await;
        let results = front.results();
        assert!(results[0].2.contains("probe/echo"), "{results:?}");
        assert!(results[1].2.contains("tracker/echo"), "{results:?}");
        front.close().await;
    });
    assert_eq!(reads.arrivals(), 2, "one reading of each folder");
    assert_eq!(skill_reads.arrivals(), 2, "one reading of each skill");
}

// ------------------------------------------------------------ never, not now

/// The question's look is below the never and the not now (folded for
/// Windows' tier 2, one world): after "not now", another session of the
/// same core asks nothing and does not look in the project's folder of
/// extensions; and a project marked never is not looked in.
#[test]
fn the_question_does_not_look_below_a_never_or_a_not_now() {
    let world = World::new("found_never_not_now", []);
    write_extension(&projects(&world), "", "the projects");
    let reads = counting(AN_EXTENSION_IS_READ);
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert_eq!(asked.len(), 1);
        front.close().await;
        assert_eq!(reads.arrivals(), 1, "read for the question");
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.close().await;
    });
    assert_eq!(reads.arrivals(), 1, "not read again");
    trust_store(&world)
        .refuse(&project_key(&world), "~/work/app")
        .unwrap();
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.close().await;
    });
    assert_eq!(
        reads.arrivals(),
        1,
        "a project marked never is not looked in"
    );
}

// ------------------------------------------------------------ the pin store

/// A pin store that cannot be read (here of a newer version) loads none of
/// the user's extensions, says the store is why (with its file under `~`),
/// and prunes nothing from the cache.
#[test]
fn an_unreadable_pin_store_loads_none_of_the_users_says_why_and_prunes_nothing() {
    let world = World::new("found_pins_unreadable", searches_both());
    write_extension(&users(&world), "", "one of mine");
    let mine = cache(&world, &users(&world));
    std::fs::write(state(&world).pins_file(), "version = 9\n").unwrap();
    // A trusted project's extension, approved and cached: the cache is read
    // and the session would prune it.
    let theirs = world.workspace.join(".cineris/extensions/tracker");
    write_named(&theirs, "tracker", "one of theirs");
    trust(&world, &[named_definition(&theirs, "tracker")]);
    cache(&world, &theirs);
    let manifest = Manifest::parse(
        "api = 1\nname = \"gone\"\nversion = \"1\"\ndescription = \"x\"\nmain = \"main.js\"\n",
    )
    .unwrap();
    let gone = Definition::of(TrustResourceKind::Extension, "gone", ["old"]).hash;
    cache_file(&world)
        .put(&gone, Entry::new(&manifest, Registration::default(), None))
        .unwrap();
    let core = world.core();
    world.run(async {
        let (front, _) = open_answering(&world, &core, not_now()).await;
        front.turn("look for both").await;
        let results = front.results();
        assert!(!results[0].2.contains("probe/echo"), "{results:?}");
        assert!(results[1].2.contains("tracker/echo"), "{results:?}");
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].contains("the pin store"), "{told:?}");
        assert!(!told[0].contains("you have not approved it"), "{told:?}");
        let home = world.home.display().to_string();
        let store: Vec<String> = front
            .notices()
            .into_iter()
            .filter(|notice| notice.starts_with("The pin store"))
            .collect();
        assert_eq!(store.len(), 1, "{:?}", front.notices());
        assert!(store[0].contains("`~/"), "{store:?}");
        assert!(
            front.notices().iter().all(|notice| !notice.contains(&home)),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
    let kept = cache_file(&world).load().cache;
    assert!(kept.get(&mine).is_some() && kept.get(&gone).is_some());
}

// ------------------------------------------------------------ keep, the flag

/// Keep trusting it, without what is new: a changed extension stays off,
/// its pin as it was, and the core asks no more; the next start asks again,
/// since the store keeps no record of a keep of an extension.
#[test]
fn keeping_the_trust_without_a_changed_extension_leaves_it_off_until_the_next_start() {
    let world = World::new("found_keep", searches());
    let folder = projects(&world);
    write_extension(&folder, "", "the first");
    let first = definition(&folder);
    trust(&world, std::slice::from_ref(&first));
    write_extension(&folder, "", "the second");
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, keep()).await;
        assert_eq!(asked.len(), 1);
        assert!(asked[0].trusted);
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        front.close().await;
        let (front, asked) = open_answering(&world, &core, keep()).await;
        assert!(asked.is_empty(), "this core asks no more: {asked:?}");
        front.close().await;
    });
    let stored = trust_store(&world).project(&project_key(&world)).unwrap();
    assert_eq!(
        stored.standing(&first),
        Standing::Approved,
        "the pin as it was"
    );
    assert_eq!(stored.standing(&definition(&folder)), Standing::Changed);
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert_eq!(asked.len(), 1, "the next start asks again");
        front.close().await;
    });
}

/// `--trust-project` approves no extension that is not pinned, and the
/// session names the command that approves one; a pinned one loads.
#[test]
fn trust_for_the_run_approves_no_unpinned_extension_and_names_the_command() {
    let world = World::new("found_trust_project", searches_both());
    write_extension(&projects(&world), "", "the projects");
    let theirs = world.workspace.join(".cineris/extensions/tracker");
    write_named(&theirs, "tracker", "one of theirs");
    trust(&world, &[named_definition(&theirs, "tracker")]);
    cache(&world, &theirs);
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for both").await;
        let results = front.results();
        assert!(!results[0].2.contains("probe/echo"), "{results:?}");
        assert!(results[1].2.contains("tracker/echo"), "{results:?}");
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(
            told[0].contains("`cineris permissions trust`") && !told[0].contains("question"),
            "{told:?}"
        );
        front.close().await;
    });
}

/// A project's `[extensions.<name>] enabled = false` does not turn off the
/// user's extension of that name: the section is the user's alone.
#[test]
fn a_projects_entry_does_not_turn_off_the_users_extension() {
    let world = World::new("found_project_entry", searches());
    world.project_config("[extensions.probe]\nenabled = false\n");
    write_extension(&users(&world), "", "one of mine");
    pin(&world, &users(&world));
    cache(&world, &users(&world));
    trust(&world, &[]);
    let core = world.core();
    world.run(async {
        let (front, _) = open_answering(&world, &core, not_now()).await;
        front.turn("look for the probe").await;
        assert!(
            found_probe(&front),
            "{:?} {:?}",
            front.results(),
            front.notices()
        );
        front.close().await;
    });
}

// ------------------------------------------------------------ a yes not kept

/// A yes the trust store cannot keep approves the extensions the question
/// listed, by the hashes shown, for that session alone: loaded, said, and
/// asked about again by a new core.
#[test]
fn a_yes_the_store_cannot_keep_loads_the_extensions_shown_for_the_session() {
    let world = World::new("found_yes_unkept", searches());
    write_extension(&projects(&world), "", "the projects");
    std::fs::create_dir_all(state(&world).root().join("trust.toml.lock")).unwrap();
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert_eq!(asked.len(), 1);
        front.turn("look for the probe").await;
        assert!(
            found_probe(&front),
            "{:?} {:?}",
            front.results(),
            front.notices()
        );
        let said: Vec<String> = front
            .notices()
            .into_iter()
            .filter(|notice| notice.contains("could not be kept"))
            .collect();
        assert_eq!(said.len(), 1, "{:?}", front.notices());
        assert!(
            said[0].contains("its rules and the extensions you were shown")
                && said[0].contains("asked about again when a session opens"),
            "{said:?}"
        );
        let home = world.home.display().to_string();
        assert!(!said[0].contains(&home), "{said:?}");
        front.close().await;
    });
    assert!(
        !trust_store(&world)
            .project(&project_key(&world))
            .unwrap()
            .is_trusted()
    );
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert_eq!(asked.len(), 1, "asked again");
        front.close().await;
    });
}

// ------------------------------------------------------------ what waits

/// A turn waits for the tools and not for the cache: the first turn runs
/// and ends while the cache's writing is held.
#[test]
fn a_turn_does_not_wait_for_the_cache_to_be_written() {
    let world = World::new("found_cache_later", searches());
    write_extension(&users(&world), "", "Gives its arguments back");
    pin(&world, &users(&world));
    let hash = definition(&users(&world)).hash;
    let core = world.core();
    let keeping = Hold::new(cineris_core::ext::BEFORE_THE_CACHE_IS_KEPT);
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        assert_eq!(keeping.stopped(), 1, "the cache waits");
        assert!(cache_file(&world).load().cache.get(&hash).is_none());
        // Written once it is let go (`an_approved_extension_the_cache_misses_
        // is_registered_and_the_first_turn_waits_for_it` reads it then).
        keeping.release();
        front.close().await;
    });
}

/// A session closed while its extension's registration waits does not
/// wait for it, and its work ends with it: nothing of it goes on into the
/// next session of the core, which registers the extension itself.
#[test]
fn a_session_closed_while_its_registration_waits_neither_blocks_nor_leaks() {
    let world = World::new("found_closed_early", searches());
    write_extension(&users(&world), "", "Gives its arguments back");
    pin(&world, &users(&world));
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    let registering = Hold::new(cineris_core::ext::BEFORE_AN_EXTENSION_IS_REGISTERED);
    world.run(async {
        let first = start(&world, &core).await;
        let mut waiting = std::pin::pin!(std::future::pending::<()>());
        registering.drive_until_reached(waiting.as_mut()).await;
        first.close().await;
        settle().await;
        assert_eq!(registering.stopped(), 0, "the closed session's work ended");
        registering.release();
        let second = start(&world, &core).await;
        second.turn("look for the probe").await;
        assert!(found_probe(&second), "{:?}", second.results());
        second.close().await;
    });
    assert_eq!(
        cineris_ext::engine::runtimes_made(),
        made + 1,
        "registered by the second session alone"
    );
}

/// A user's folder of extensions that cannot be listed whole takes every
/// name: what it holds is not known, so no project's extension is used.
#[cfg(unix)]
#[test]
fn a_users_folder_that_cannot_be_listed_takes_every_name() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new("found_names_cut", searches());
    let root = world.dirs().config.unwrap().join("extensions");
    std::fs::create_dir_all(&root).unwrap();
    let theirs = projects(&world);
    write_extension(&theirs, "", "the projects");
    trust(&world, &[definition(&theirs)]);
    cache(&world, &theirs);
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
    let open = || std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700));
    // An account every folder is open to (root) lists it anyway.
    if std::fs::read_dir(&root).is_ok() {
        open().unwrap();
        println!("skipped: this account can list a folder closed to it");
        return;
    }
    let core = world.core();
    world.run(async {
        let (front, _) = open_answering(&world, &core, not_now()).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        let told = about_probe(&front);
        assert!(
            told.iter()
                .any(|notice| notice.contains("could not be listed whole")),
            "{told:?}"
        );
        front.close().await;
    });
    open().unwrap();
}
