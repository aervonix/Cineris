//! The private store on disk: nothing learned is lost to a hand edit, a
//! stop between two writes or two writers.

use std::fs::TryLockError;

use cineris_base::private;
use cineris_knowledge::store::{STORE_NAME, Source};
use cineris_knowledge::{Kind, Proposed, Status, StoreError};
use cineris_protocol::project::EntryAction;
use cineris_testkit::scratch;

use crate::{MIXED, World, change, in_use, unasked};

fn fact(text: &str) -> Proposed {
    Proposed {
        kind: Kind::Fact,
        text: text.to_owned(),
        evidence: "the user said so".to_owned(),
        check: None,
        session: None,
    }
}

#[test]
fn a_project_touches_nothing_until_it_is_asked() {
    let world = World::new(scratch!("store_untouched"));
    let project = world.project();
    assert!(!world.scratch.join("state").exists());
    // A load of a project with no store makes nothing either.
    let snapshot = project.load(None);
    assert!(snapshot.entries().is_empty());
    assert!(snapshot.problems().is_empty());
    assert!(project.is_current(&snapshot));
    assert!(!world.scratch.join("state").exists());
}

#[test]
fn a_change_is_written_owner_only_with_its_good_copy_beside_it() {
    let world = World::new(scratch!("store_private"));
    world
        .project()
        .propose(fact("The docs are in docs/."), None)
        .unwrap();
    let file = world.store_file();
    assert!(
        private::is_private(&file).unwrap(),
        "the store is owner-only"
    );
    assert!(private::is_private(&world.good_copy()).unwrap());
    assert!(private::is_private(file.parent().unwrap()).unwrap());
    assert_eq!(
        std::fs::read(&file).unwrap(),
        std::fs::read(world.good_copy()).unwrap(),
        "the good copy is the store as it was last written"
    );
    assert!(private::lock_path(&file).exists());
    // No temporary file is left beside them.
    let mut names = world
        .scratch
        .listing("state/projects/0123456789abcdef0123456789abcdef");
    names.sort();
    assert_eq!(
        names,
        [
            "knowledge.toml",
            "knowledge.toml.good",
            "knowledge.toml.lock"
        ]
    );
}

/// The lock is held from before the read until after the write: while a
/// change is stopped before its file takes its name, no other open of the
/// lock file can take it.
#[test]
fn a_change_holds_the_lock_while_it_writes() {
    let world = World::new(scratch!("store_lock_held"));
    world.project().propose(fact("One."), None).unwrap();
    let hold = cineris_testkit::Hold::new(cineris_base::atomic::BEFORE_THE_RENAME);
    let project = world.project();
    let writer = std::thread::spawn(move || project.propose(fact("Two."), None).map(|_| ()));
    hold.reach();
    let other = std::fs::File::open(private::lock_path(&world.store_file())).unwrap();
    assert!(
        matches!(other.try_lock(), Err(TryLockError::WouldBlock)),
        "the store's lock is held while the change is written"
    );
    drop(other);
    hold.release();
    writer.join().unwrap().unwrap();
    assert_eq!(world.load().entries().len(), 2);
}

/// Two sessions of one project that learn at the same moment both keep
/// what they learned: every change is made to the file as it is read
/// under the lock, never to a copy a session held.
#[test]
fn writers_at_once_all_keep_their_changes() {
    const WRITERS: usize = 4;
    const EACH: usize = 12;
    let world = World::new(scratch!("store_writers"));
    let writers: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let project = world.project();
            std::thread::spawn(move || {
                // Each writer holds a snapshot from before any of them
                // wrote, as a session does between its turns.
                let stale = project.load(None);
                for index in 0..EACH {
                    project
                        .propose(
                            fact(&format!("Writer {writer} learned {index}.")),
                            Some(&stale),
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let snapshot = world.load();
    assert_eq!(snapshot.entries().len(), WRITERS * EACH);
    let mut ids: Vec<&str> = snapshot
        .entries()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), WRITERS * EACH, "no id was given twice");
    for writer in 0..WRITERS {
        for index in 0..EACH {
            let text = format!("Writer {writer} learned {index}.");
            assert!(
                snapshot.entries().iter().any(|entry| entry.text == text),
                "{text} was lost"
            );
        }
    }
}

#[test]
fn an_invalid_store_is_reported_and_never_written_over() {
    let world = World::new(scratch!("store_invalid"));
    world.write_store(MIXED);
    // A first load is of a valid store, and keeps a good copy of it.
    let before = world.load();
    assert_eq!(before.entries().len(), 5);
    assert_eq!(std::fs::read_to_string(world.good_copy()).unwrap(), MIXED);

    // A person's edit goes wrong.
    let broken = MIXED.replace("status = \"disabled\"", "status = \"disbled\"");
    world.write_store(&broken);

    // A process that starts now: the old state from the good copy, with
    // the problem and where it is.
    let fresh = world.load();
    assert!(fresh.is_degraded());
    assert_eq!(fresh.entries().len(), 5);
    assert_eq!(fresh.problems().len(), 1);
    let problem = &fresh.problems()[0];
    assert!(problem.contains(STORE_NAME), "{problem}");
    assert!(problem.contains("`status` is `disbled`"), "{problem}");
    assert!(problem.contains("knowledge.toml.good"), "{problem}");
    // The prompt has what it had. The safe list has nothing: the file that
    // cannot be read may hold the edit that turned a command off.
    assert_eq!(in_use(&world, &fresh), ["k_1", "k_2", "k_4"]);
    assert_eq!(unasked(&world, &before), ["cargo test"]);
    assert_eq!(unasked(&world, &fresh), Vec::<String>::new());

    // Nothing writes over it: not a proposal, not a user's change, not the
    // clock.
    let project = world.project();
    let refused = project.propose(fact("New."), Some(&fresh)).unwrap_err();
    assert!(matches!(refused, StoreError::Invalid(_)), "{refused}");
    assert_eq!(
        cineris_knowledge::error_code(&refused),
        "project_store_invalid"
    );
    let refused = project
        .update(&[change("k_3", EntryAction::Accept)], &[], Some(&fresh))
        .unwrap_err();
    assert!(matches!(refused, StoreError::Invalid(_)));
    assert!(matches!(
        project.lapse(Some(&fresh)),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        world.store_text(),
        broken,
        "the file is as the person left it"
    );
    assert_eq!(std::fs::read_to_string(world.good_copy()).unwrap(), MIXED);

    // Put right, it is taken up, with the edit that was meant.
    world.write_store(&MIXED.replace("status = \"disabled\"", "status = \"active\""));
    let fixed = world.load();
    assert!(fixed.problems().is_empty());
    assert!(!fixed.is_degraded());
    assert_eq!(in_use(&world, &fixed), ["k_1", "k_2", "k_4", "k_5"]);
    assert_eq!(unasked(&world, &fixed), ["cargo test"]);
}

/// A session that had the store loaded keeps what it had when a later
/// edit goes wrong: its snapshot, not the good copy.
#[test]
fn a_session_keeps_what_it_had_when_an_edit_goes_wrong() {
    let world = World::new(scratch!("store_previous"));
    world.write_store(MIXED);
    let project = world.project();
    let held = project.load(None);
    world.write_store("version = 1\n[[entry]\n");
    assert!(
        !project.is_current(&held),
        "the edit is seen at the next look"
    );
    let after = project.load(Some(&held));
    assert_eq!(after.entries(), held.entries());
    assert!(after.is_degraded());
    assert!(
        after.problems()[0].contains("line 2"),
        "{:?}",
        after.problems()
    );
    assert!(after.problems()[0].contains("What was read of it before is still used"));
    assert_eq!(unasked(&world, &after), Vec::<String>::new());
}

/// The reviewer's case: the user turns an accepted command off by hand,
/// and a later edit goes wrong. The good copy must not bring the command
/// back.
#[test]
fn the_good_copy_follows_a_valid_hand_edit() {
    let world = World::new(scratch!("store_good_follows"));
    world.write_store(MIXED);
    // The harness writes: the good copy has `cargo test` accepted.
    world
        .project()
        .update(&[change("k_3", EntryAction::Accept)], &[], None)
        .unwrap();
    let written = world.store_text();
    assert_eq!(std::fs::read_to_string(world.good_copy()).unwrap(), written);
    // The user turns the command off by hand. A load takes the edit up,
    // and the good copy with it.
    let edited = written.replacen("status = \"active\"", "status = \"disabled\"", 1);
    assert_ne!(edited, written);
    world.write_store(&edited);
    let taken_up = world.load();
    assert_eq!(
        taken_up.find(&crate::id("k_1")).unwrap().status,
        Status::Disabled
    );
    assert_eq!(std::fs::read_to_string(world.good_copy()).unwrap(), edited);
    // Then a typo. A new process falls back to the edit, not to before it.
    world.write_store(&format!("{edited}\n[[entry]\n"));
    let fresh = world.load();
    assert!(fresh.is_degraded());
    assert_eq!(
        fresh.find(&crate::id("k_1")).unwrap().status,
        Status::Disabled
    );
    assert!(!in_use(&world, &fresh).contains(&"k_1".to_owned()));
    assert_eq!(unasked(&world, &fresh), Vec::<String>::new());
}

/// What a stop between the store's write and its good copy's leaves: the
/// store has the change, and the next load brings the copy up to it.
#[test]
fn a_stop_between_the_store_and_its_good_copy_loses_nothing() {
    let world = World::new(scratch!("store_between"));
    world.project().propose(fact("One."), None).unwrap();
    let old = std::fs::read(world.good_copy()).unwrap();
    world.project().propose(fact("Two."), None).unwrap();
    // As if the process had stopped before the second copy was written.
    std::fs::write(world.good_copy(), &old).unwrap();
    let snapshot = world.load();
    assert_eq!(snapshot.entries().len(), 2);
    assert!(snapshot.problems().is_empty());
    assert_eq!(
        std::fs::read(world.good_copy()).unwrap(),
        std::fs::read(world.store_file()).unwrap()
    );
    // A good copy that is itself unreadable is no obstacle to a valid
    // store, and is made again.
    std::fs::write(world.good_copy(), b"\xff\xfe not text").unwrap();
    assert_eq!(world.load().entries().len(), 2);
    assert_eq!(
        std::fs::read(world.good_copy()).unwrap(),
        std::fs::read(world.store_file()).unwrap()
    );
}

/// Removing the file is how a user starts again: a store that is not
/// there is empty, never the good copy, and the copy goes too, so that a
/// file made by hand afterwards cannot fall back to what was removed.
#[test]
fn a_store_that_is_removed_is_empty_and_its_good_copy_goes() {
    let world = World::new(scratch!("store_removed"));
    world.write_store(MIXED);
    assert_eq!(world.load().entries().len(), 5);
    assert!(world.good_copy().exists());
    std::fs::remove_file(world.store_file()).unwrap();
    let snapshot = world.load();
    assert!(snapshot.entries().is_empty());
    assert!(snapshot.problems().is_empty());
    assert!(!snapshot.is_degraded());
    assert!(!world.good_copy().exists());
    // A broken file made by hand now has nothing to fall back to.
    world.write_store("version = 1\n[[entry]\n");
    let broken = world.load();
    assert!(broken.entries().is_empty());
    assert!(broken.problems()[0].contains("Nothing of it is used"));
    let loaded = world.project().store().load();
    assert_eq!(loaded.source, Source::Nothing);
}

#[test]
fn a_store_that_is_too_large_or_not_text_is_invalid() {
    let world = World::new(scratch!("store_bounds"));
    let mut large = String::from("version = 1\n");
    while large.len() <= 256 * 1024 {
        large.push_str("# padding to pass the cap, a line at a time, and no entry at all\n");
    }
    world.write_store(&large);
    let snapshot = world.load();
    assert!(snapshot.is_degraded());
    assert!(
        snapshot.problems()[0].contains("may be up to 262144"),
        "{:?}",
        snapshot.problems()
    );
    std::fs::write(world.store_file(), b"version = 1\n\xff\n").unwrap();
    assert!(world.load().problems()[0].contains("not UTF-8"));
    // A directory where the file should be.
    std::fs::remove_file(world.store_file()).unwrap();
    std::fs::create_dir(world.store_file()).unwrap();
    let snapshot = world.load();
    assert!(snapshot.is_degraded());
    assert!(matches!(
        world.project().propose(fact("New."), None),
        Err(StoreError::Invalid(_))
    ));
}

/// A link where the store should be is not read through, and not written
/// through.
#[test]
fn a_link_at_the_store_is_not_followed() {
    let world = World::new(scratch!("store_link"));
    let elsewhere = world.scratch.file("elsewhere.toml", MIXED);
    std::fs::create_dir_all(world.store_file().parent().unwrap()).unwrap();
    if !crate::link(&elsewhere, &world.store_file(), false) {
        return;
    }
    let snapshot = world.load();
    assert!(
        snapshot.entries().is_empty(),
        "nothing is read through the link"
    );
    assert!(snapshot.is_degraded());
    assert!(world.project().propose(fact("New."), None).is_err());
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), MIXED);
    assert!(
        std::fs::symlink_metadata(world.store_file())
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// The review's first blocker: the cap on a file's size is held when the
/// store is written, not only when it is read. A change that would take
/// the file past it is refused, and the store stays one every load reads.
#[test]
fn a_change_that_would_pass_the_size_cap_is_refused_and_the_store_stays_readable() {
    let world = World::new(scratch!("store_size_cap"));
    // A store a person wrote to within 100 bytes of the cap. Written back
    // it is larger, by the first lines and the counter the harness adds.
    let store = crate::store_of(256 * 1024 - 100);
    world.write_store(&store);
    let before = world.load();
    assert!(before.problems().is_empty(), "{:?}", before.problems());
    assert!(
        before.entries().len() < 500,
        "the size binds before the count"
    );
    let project = world.project();
    // Even a change that only turns an entry off does not fit.
    let refused = project
        .update(&[change("k_1", EntryAction::Disable)], &[], None)
        .unwrap_err();
    assert!(matches!(refused, StoreError::Full(_)), "{refused}");
    assert_eq!(
        cineris_knowledge::error_code(&refused),
        "project_store_full"
    );
    assert_eq!(world.store_text(), store, "the store is as it was");
    // Every process still reads it.
    let after = world.load();
    assert!(after.problems().is_empty(), "{:?}", after.problems());
    assert_eq!(after.entries(), before.entries());
}

/// A store keeps room for the changes that only narrow or decide: a new
/// entry is refused within the reserve, and turning one off, accepting one
/// and deleting one still go through.
#[test]
fn a_store_near_its_cap_takes_no_new_entry_and_can_still_be_told_to_use_less() {
    let world = World::new(scratch!("store_reserve"));
    let reserve = usize::try_from(cineris_knowledge::project::RESERVED_BYTES).unwrap();
    world.write_store(&crate::store_of(256 * 1024 - reserve + 2_000));
    let project = world.project();
    let count = project.load(None).entries().len();
    // Nothing new: not a user's entry, not a model's proposal.
    let added = project
        .update(
            &[],
            &[cineris_protocol::project::NewEntry {
                kind: cineris_protocol::project::EntryKind::Fact,
                text: "One more.".to_owned(),
                command: None,
                check: None,
            }],
            None,
        )
        .unwrap_err();
    assert_eq!(cineris_knowledge::error_code(&added), "project_store_full");
    assert!(added.to_string().contains("takes no new entry"), "{added}");
    let proposed = project.propose(fact("One more."), None).unwrap_err();
    assert_eq!(
        cineris_knowledge::error_code(&proposed),
        "project_store_full"
    );
    // What only narrows or decides goes through.
    project
        .update(&[change("k_1", EntryAction::Disable)], &[], None)
        .unwrap();
    project
        .update(&[change("k_1", EntryAction::Accept)], &[], None)
        .unwrap();
    let updated = project
        .update(&[change("k_2", EntryAction::Delete)], &[], None)
        .unwrap();
    assert_eq!(updated.snapshot.entries().len(), count - 1);
    assert!(world.load().problems().is_empty());
}

/// A store whose counter is at its bound takes no new entry: the id would
/// leave a counter this build does not read back, and what is not read
/// back is not written. Everything else about the store still works.
#[test]
fn a_change_that_could_not_be_read_back_is_not_written() {
    let world = World::new(scratch!("store_read_back"));
    let store = "version = 1\nnext = 9007199254740992\n[[entry]]\nid = \"k_1\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
    world.write_store(store);
    let project = world.project();
    let refused = project.propose(fact("One more."), None).unwrap_err();
    assert!(matches!(refused, StoreError::Unreadable(_)), "{refused}");
    assert_eq!(
        cineris_knowledge::error_code(&refused),
        "project_store_unwritable"
    );
    assert_eq!(world.store_text(), store);
    assert!(world.load().problems().is_empty());
    project
        .update(&[change("k_1", EntryAction::Disable)], &[], None)
        .unwrap();
    assert!(world.load().problems().is_empty());
}

/// A person's edit takes no lock. One saved while a change is being
/// written (after the store has its new content) is not taken for the
/// state the change gives back: it shows at the next look.
#[test]
fn an_edit_saved_while_a_change_is_written_is_seen() {
    let world = World::new(scratch!("store_stamp_hand_edit"));
    world.write_store(MIXED);
    let hold = cineris_testkit::Hold::new(cineris_knowledge::store::AFTER_THE_STORE);
    let project = world.project();
    let ours = {
        let project = project.clone();
        std::thread::spawn(move || {
            project
                .update(&[change("k_3", EntryAction::Accept)], &[], None)
                .unwrap()
        })
    };
    hold.reach();
    // The store has our change. A person saves the file now, with the
    // accepted command turned off.
    let edited = world
        .store_text()
        .replacen("status = \"active\"", "status = \"disabled\"", 1);
    std::fs::write(world.store_file(), &edited).unwrap();
    hold.release();
    let updated = ours.join().unwrap();
    assert!(
        !project.is_current(&updated.snapshot),
        "the edit must show at the next look"
    );
    let reloaded = project.load(Some(&updated.snapshot));
    assert_eq!(
        reloaded.find(&crate::id("k_1")).unwrap().status,
        Status::Disabled
    );
    assert_eq!(unasked(&world, &reloaded), Vec::<String>::new());
}

/// The review's second blocker: the snapshot a change gives back holds the
/// store's stamp from while the change held the lock. Another process's
/// change, made the moment the lock is let go, is then seen at the next
/// look, and a command it turned off does not go on running unasked.
#[test]
fn a_change_made_by_another_right_after_ours_is_seen() {
    let world = World::new(scratch!("store_stamp_under_lock"));
    world.write_store(MIXED);
    let hold = cineris_testkit::Hold::new(cineris_knowledge::project::BEFORE_THE_SNAPSHOT);
    let project = world.project();
    let ours = {
        let project = project.clone();
        std::thread::spawn(move || {
            project
                .update(&[change("k_3", EntryAction::Accept)], &[], None)
                .unwrap()
        })
    };
    // Our change is written and its lock let go; its snapshot is not put
    // together yet.
    hold.reach();
    // Another process turns the accepted command off.
    world
        .project()
        .store()
        .change(|state, _| {
            state.entries[0].status = Status::Disabled;
            state.entries[0].accepted = None;
            Ok::<_, cineris_knowledge::ChangeError>(((), true))
        })
        .unwrap();
    hold.release();
    let updated = ours.join().unwrap();
    assert_eq!(
        unasked(&world, &updated.snapshot),
        ["cargo test"],
        "as we wrote it"
    );
    assert!(
        !project.is_current(&updated.snapshot),
        "the other's change must show at the next look"
    );
    let reloaded = project.load(Some(&updated.snapshot));
    assert_eq!(unasked(&world, &reloaded), Vec::<String>::new());
}

/// A load never waits for the store's lock: bringing the good copy up to
/// date is tried, and left for the next load when a writer holds the lock.
/// A writer that is stuck must not hang every session's turn.
#[test]
fn a_load_does_not_wait_for_a_writer() {
    let world = World::new(scratch!("store_load_no_wait"));
    // A valid store with no good copy yet, and a writer holding the lock.
    let held = private::hold_lock(&world.store_file()).unwrap();
    assert!(held.is_some(), "this file system can lock");
    world.write_store(MIXED);
    let snapshot = world.load();
    assert_eq!(snapshot.entries().len(), 5);
    assert!(snapshot.problems().is_empty());
    assert!(!world.good_copy().exists(), "the copy waits for the lock");
    drop(held);
    assert_eq!(world.load().entries().len(), 5);
    assert_eq!(std::fs::read_to_string(world.good_copy()).unwrap(), MIXED);
}
