//! Storing what the observer saw: seen once, then active, and what the
//! user can undo.

use cortex_knowledge::learn::{MAX_SEEN_ONCE, SEEN_ONCE_DAYS};
use cortex_knowledge::{
    Basis, Check, Kind, Learnt, Outcome, Purpose, Status, StoreError, error_code, render,
};
use cortex_protocol::ids::SessionId;
use cortex_protocol::project::EntryAction;
use cortex_testkit::scratch;

use crate::{DAY, MIXED, World, change, id, in_use, unasked};

fn command(line: &str, purpose: Purpose) -> Learnt {
    Learnt {
        kind: Kind::Command,
        purpose,
        line: line.to_owned(),
        cwd: None,
        duration_ms: 4_000,
        read_only: false,
        session: Some(SessionId::new("s_7")),
    }
}

fn flip(line: &str) -> Learnt {
    Learnt {
        kind: Kind::Flaky,
        ..command(line, Purpose::Test)
    }
}

/// The done criterion: a command observed succeeding twice becomes an
/// active entry, and can be undone.
#[test]
fn a_command_seen_to_succeed_twice_becomes_active_and_can_be_undone() {
    let world = World::new(scratch!("learn_twice"));
    let project = world.project();
    let seen = command("cargo test", Purpose::Test);

    let (outcome, first) = project.learn(&seen, None).unwrap();
    assert_eq!(outcome, Outcome::Proposed);
    let entry = &first.changed[0];
    assert_eq!(entry.id.as_str(), "k_1");
    assert_eq!(
        (entry.status, entry.basis, entry.seen),
        (Status::Proposed, Some(Basis::Observed), 1)
    );
    assert_eq!(entry.text, "The project's test command.");
    assert_eq!(
        entry
            .learned
            .as_ref()
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .as_str(),
        "s_7"
    );
    assert_eq!(entry.command.as_ref().unwrap().usual_ms, Some(4_000));
    assert_eq!(entry.expires, None);
    // Seen once, it is in nothing.
    assert!(in_use(&world, &first.snapshot).is_empty());

    // The second time, in another session of the project.
    let (outcome, second) = world.project().learn(&seen, None).unwrap();
    assert_eq!(outcome, Outcome::Activated);
    let entry = &second.changed[0];
    assert_eq!((entry.status, entry.seen), (Status::Active, 2));
    assert_eq!(entry.expires, world.now().checked_add(90 * DAY));
    // Active is in the prompt, and it still asks: nobody accepted it.
    assert_eq!(in_use(&world, &second.snapshot), ["k_1"]);
    assert!(unasked(&world, &second.snapshot).is_empty());
    assert!(
        render(&second.snapshot, world.now(), 1_500)
            .text
            .ends_with("Commands:\n- test: `cargo test`")
    );
    // Accepted, it runs without asking.
    let accepted = project
        .update(&[change("k_1", EntryAction::Accept)], &[], None)
        .unwrap();
    assert_eq!(unasked(&world, &accepted.snapshot), ["cargo test"]);

    // Undone: disabled, it is out of both, and is not learned again
    // however often it is seen.
    let undone = project
        .update(&[change("k_1", EntryAction::Disable)], &[], None)
        .unwrap();
    assert!(in_use(&world, &undone.snapshot).is_empty());
    for _ in 0..3 {
        world.clock.advance(2 * DAY);
        let (outcome, after) = project.learn(&seen, Some(&undone.snapshot)).unwrap();
        assert_eq!(outcome, Outcome::Nothing);
        assert!(after.changed.is_empty());
        assert!(in_use(&world, &after.snapshot).is_empty());
    }
    assert!(!undone.snapshot.wants(&seen, world.now()));

    // Deleted, it is forgotten, and learned again from two new runs.
    project
        .update(&[change("k_1", EntryAction::Delete)], &[], None)
        .unwrap();
    assert_eq!(project.learn(&seen, None).unwrap().0, Outcome::Proposed);
    let (outcome, again) = project.learn(&seen, None).unwrap();
    assert_eq!(outcome, Outcome::Activated);
    assert_eq!(
        again.changed[0].id.as_str(),
        "k_2",
        "a new entry, under a new id"
    );
}

/// The done criterion: a test seen to flip becomes a flaky entry.
#[test]
fn a_test_seen_to_flip_twice_becomes_an_active_flaky_entry() {
    let world = World::new(scratch!("learn_flaky"));
    let project = world.project();
    let mut seen = flip("cargo test -p net");
    seen.cwd = Some("crates/net".to_owned());
    assert_eq!(project.learn(&seen, None).unwrap().0, Outcome::Proposed);
    let (outcome, updated) = project.learn(&seen, None).unwrap();
    assert_eq!(outcome, Outcome::Activated);
    let entry = &updated.changed[0];
    assert_eq!(entry.kind, Kind::Flaky);
    assert_eq!(
        entry.text,
        "The test command `cargo test -p net` (in `crates/net`) has failed and then passed with \
         nothing changed in between."
    );
    assert_eq!(entry.expires, world.now().checked_add(30 * DAY));
    assert_eq!(entry.check, None);
    let section = render(&updated.snapshot, world.now(), 1_500).text;
    assert!(
        section.contains("\n\nFlaky tests:\n- The test command `cargo test -p net`"),
        "{section}"
    );
    // A flaky entry is never a command to run without asking.
    project
        .update(&[change("k_1", EntryAction::Accept)], &[], None)
        .unwrap();
    assert!(unasked(&world, &world.load()).is_empty());
    // The command's own success is another entry.
    let own = Learnt {
        kind: Kind::Command,
        ..seen.clone()
    };
    assert_eq!(project.learn(&own, None).unwrap().0, Outcome::Proposed);
    assert_eq!(world.load().entries().len(), 2);
}

/// A command is the same command only with the same line, in the same
/// directory, as the same kind of entry.
#[test]
fn two_observations_count_only_for_the_same_line_in_the_same_directory() {
    let world = World::new(scratch!("learn_same"));
    let project = world.project();
    let seen = command("cargo test", Purpose::Test);
    project.learn(&seen, None).unwrap();
    let mut elsewhere = seen.clone();
    elsewhere.cwd = Some("crates/app".to_owned());
    for other in [
        elsewhere,
        command("cargo test --workspace", Purpose::Test),
        command("cargo Test", Purpose::Test),
        flip("cargo test"),
    ] {
        assert_eq!(
            project.learn(&other, None).unwrap().0,
            Outcome::Proposed,
            "{other:?}"
        );
    }
    assert!(in_use(&world, &world.load()).is_empty());
    assert_eq!(world.load().entries().len(), 5);
}

/// A later success confirms an active entry at most once a day: its expiry
/// moves out and its usual duration follows, and the section does not
/// change.
#[test]
fn an_active_entry_is_confirmed_at_most_once_a_day() {
    let world = World::new(scratch!("learn_confirm"));
    let project = world.project();
    let seen = command("cargo build", Purpose::Build);
    project.learn(&seen, None).unwrap();
    let (_, active) = project.learn(&seen, None).unwrap();
    let section = render(&active.snapshot, world.now(), 1_500);
    let written = world.store_text();

    // The same day: nothing is written, and the snapshot says so first.
    world.clock.advance(DAY / 2);
    assert!(!active.snapshot.wants(&seen, world.now()));
    let (outcome, same) = project.learn(&seen, Some(&active.snapshot)).unwrap();
    assert_eq!(outcome, Outcome::Nothing);
    assert_eq!(world.store_text(), written);

    // A day on: confirmed.
    world.clock.advance(DAY);
    assert!(same.snapshot.wants(&seen, world.now()));
    let mut slower = seen.clone();
    slower.duration_ms = 8_000;
    let (outcome, confirmed) = project.learn(&slower, Some(&same.snapshot)).unwrap();
    assert_eq!(outcome, Outcome::Confirmed);
    let entry = &confirmed.changed[0];
    assert_eq!(entry.confirmed, Some(world.now()));
    assert_eq!(entry.expires, world.now().checked_add(90 * DAY));
    assert_eq!(entry.seen, 3);
    // Three parts what it was, one part what it took now.
    assert_eq!(entry.command.as_ref().unwrap().usual_ms, Some(5_000));
    assert_eq!(render(&confirmed.snapshot, world.now(), 1_500), section);
}

/// An entry that lapsed is brought back by being seen to hold again, and
/// its acceptance does not come back with it: whether the clock's pass
/// had marked it or not.
#[test]
fn an_entry_that_lapsed_comes_back_unaccepted_when_it_is_seen_again() {
    for marked in [true, false] {
        let world = World::new(scratch!(if marked {
            "learn_lapsed_marked"
        } else {
            "learn_lapsed_unmarked"
        }));
        world.write_store(MIXED);
        let project = world.project();
        assert_eq!(unasked(&world, &project.load(None)), ["cargo test"]);
        world.clock.advance(91 * DAY);
        if marked {
            assert_eq!(project.lapse(None).unwrap().changed.len(), 2);
        }
        let seen = command("cargo test", Purpose::Test);
        let before = project.load(None);
        assert!(before.wants(&seen, world.now()));
        let (outcome, back) = project.learn(&seen, Some(&before)).unwrap();
        assert_eq!(outcome, Outcome::Revived, "marked: {marked}");
        let entry = &back.changed[0];
        assert_eq!(
            (entry.status, entry.lapsed, entry.accepted),
            (Status::Active, None, None)
        );
        assert_eq!(entry.expires, world.now().checked_add(90 * DAY));
        assert!(in_use(&world, &back.snapshot).contains(&"k_1".to_owned()));
        assert!(
            unasked(&world, &back.snapshot).is_empty(),
            "marked: {marked}"
        );
    }
}

/// What somebody wrote, and what the user accepted from the shared file,
/// is known already: the observer adds nothing beside it.
#[test]
fn a_command_somebody_wrote_is_not_learned_beside_it() {
    let world = World::new(scratch!("learn_known"));
    world.write_store(
        "version = 1\n[[entry]]\nid = \"k_1\"\nkind = \"command\"\ntext = \"Lints.\"\nstatus = \"active\"\ncommand = { purpose = \"check\", run = \"cargo clippy\" }\n",
    );
    world.write_shared(
        "version = 1\n[[entry]]\nkind = \"command\"\ntext = \"Builds.\"\ncommand = { purpose = \"build\", run = \"make\" }\n",
    );
    let project = world.project();
    let snapshot = project.load(None);
    let own = command("cargo clippy", Purpose::Check);
    assert!(!snapshot.wants(&own, world.now()));
    assert_eq!(
        project.learn(&own, Some(&snapshot)).unwrap().0,
        Outcome::Nothing
    );
    // The shared command is a proposal still: what the harness sees itself
    // is its own to learn.
    let shared = command("make", Purpose::Build);
    assert!(snapshot.wants(&shared, world.now()));
    // Accepted, it is known.
    let accepted = project
        .update(
            &[change(
                snapshot.entries()[1].id.as_str(),
                EntryAction::Accept,
            )],
            &[],
            None,
        )
        .unwrap();
    assert!(!accepted.snapshot.wants(&shared, world.now()));
    assert_eq!(
        project.learn(&shared, Some(&accepted.snapshot)).unwrap().0,
        Outcome::Nothing
    );
    assert_eq!(world.load().entries().len(), 2);
}

/// Entries seen once do not fill the store: the one seen longest ago goes
/// when there is no room for another, and one not seen again in a week is
/// dropped.
#[test]
fn entries_seen_once_are_bounded_and_do_not_wait_for_ever() {
    let world = World::new(scratch!("learn_seen_once"));
    world.write_store(MIXED);
    let project = world.project();
    for index in 0..MAX_SEEN_ONCE {
        world.clock.advance(std::time::Duration::from_secs(60));
        let (outcome, _) = project
            .learn(
                &command(&format!("cargo test -p part-{index}"), Purpose::Test),
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Proposed);
    }
    assert_eq!(project.load(None).entries().len(), 5 + MAX_SEEN_ONCE);
    // One more: the first of them goes, and nothing else does.
    let (_, updated) = project
        .learn(&command("cargo test -p one-more", Purpose::Test), None)
        .unwrap();
    assert_eq!(updated.removed, [id("k_6")]);
    assert_eq!(updated.snapshot.entries().len(), 5 + MAX_SEEN_ONCE);
    assert_eq!(in_use(&world, &updated.snapshot), ["k_1", "k_2", "k_4"]);
    // A week on, the next thing learned drops every one that was not seen
    // again; the model's proposal, which waits for the user, stays.
    world
        .clock
        .advance(u32::try_from(SEEN_ONCE_DAYS).unwrap() * DAY);
    let (_, later) = project
        .learn(&command("cargo build", Purpose::Build), None)
        .unwrap();
    assert_eq!(later.removed.len(), MAX_SEEN_ONCE);
    assert_eq!(later.snapshot.entries().len(), 6);
    assert_eq!(
        later.snapshot.find(&id("k_3")).unwrap().status,
        Status::Proposed
    );
}

/// The check the harness writes by itself: that a program which is a path
/// inside the workspace still exists.
#[test]
fn a_program_in_the_workspace_gets_a_check_that_it_exists() {
    let world = World::new(scratch!("learn_check"));
    let project = world.project();
    let mut script = command("./scripts/test.sh fast", Purpose::Test);
    script.cwd = Some("tools".to_owned());
    let (_, updated) = project.learn(&script, None).unwrap();
    assert_eq!(
        updated.changed[0].check,
        Some(Check::Exists {
            path: "tools/scripts/test.sh".to_owned()
        })
    );
    for (line, check) in [
        ("cargo test", None),
        ("/usr/bin/make test", None),
        ("../outside/run.sh test", None),
        // A program on a drive: the store refuses a check with such a
        // path, and the entry would never be learned (found by the
        // independent review).
        ("C:/tools/jest.cmd", None),
        ("CI=1 scripts/check.sh", Some("scripts/check.sh")),
    ] {
        let (_, updated) = project.learn(&command(line, Purpose::Test), None).unwrap();
        assert_eq!(
            updated.changed[0].check,
            check.map(|path| Check::Exists {
                path: path.to_owned()
            }),
            "{line}"
        );
    }
}

/// An observation is dropped, with the reason, when the store cannot take
/// it: it is invalid, or has no room for a new entry.
#[test]
fn an_observation_the_store_cannot_take_is_refused() {
    let world = World::new(scratch!("learn_refused"));
    world.write_store("version = 1\n[[entry]\n");
    let seen = command("cargo test", Purpose::Test);
    let snapshot = world.load();
    assert!(
        !snapshot.wants(&seen, world.now()),
        "a store that cannot be read wants nothing"
    );
    let refused = world.project().learn(&seen, None).unwrap_err();
    assert!(matches!(refused, StoreError::Invalid(_)));
    assert_eq!(world.store_text(), "version = 1\n[[entry]\n");

    let reserve = usize::try_from(cortex_knowledge::project::RESERVED_BYTES).unwrap();
    world.write_store(&crate::store_of(256 * 1024 - reserve + 2_000));
    let refused = world.project().learn(&seen, None).unwrap_err();
    assert_eq!(error_code(&refused), "project_store_full");
    // Only a command and a flaky test are observed.
    let mut fact = seen.clone();
    fact.kind = Kind::Fact;
    assert_eq!(
        error_code(&world.project().learn(&fact, None).unwrap_err()),
        "invalid_entry"
    );
}
