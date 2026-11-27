//! The user's own pins (`pins.toml`, since step 28) on a real file system:
//! one pin for each kind and name, signed whole by the person's key (since
//! the signed approvals step), read strictly, written owner-only and
//! atomically under a lock, and closed to every tool.

use std::sync::Arc;
use std::time::Duration;

use cineris_base::{AtomicWriteError, FakeClock, SharedClock, private};
use cineris_policy::test_support::Person;
use cineris_policy::trust::BEFORE_THE_PINS_ARE_WRITTEN;
use cineris_policy::{Checked, Definition, PinChange, PinError, PinStore, Standing, UserPins};
use cineris_protocol::lease::{Effect, Level, RuleAction};
use cineris_protocol::methods::TrustResourceKind;

use crate::Said::Deny;
use crate::{Scratch, clock, engine_with, said, shell, user};

fn store(scratch: &Scratch) -> PinStore {
    PinStore::new(scratch.join("state").join("pins.toml"), clock())
}

fn extension(name: &str, content: &str) -> Definition {
    Definition::of(TrustResourceKind::Extension, name, [content])
}

const EXTENSION: TrustResourceKind = TrustResourceKind::Extension;

/// The pins as a session the person opened reads them.
fn read(store: &PinStore, person: &Person) -> UserPins {
    store.pins(Some(&person.verifier())).unwrap()
}

/// A pin of `definition`, as the person's yes makes it: what a store that
/// refuses gives back.
fn pin_change(definition: &Definition) -> [PinChange; 1] {
    [PinChange::Pin(definition.clone().into())]
}

fn unpin_change(name: &str) -> [PinChange; 1] {
    [PinChange::Unpin {
        kind: EXTENSION,
        name: name.to_owned(),
    }]
}

#[test]
fn a_pin_is_written_and_read_back() {
    let scratch = cineris_testkit::scratch!("pins_round_trip");
    let store = store(&scratch);
    let person = Person::new();
    let tracker = extension("tracker", "the manifest and the script");
    // A name the file must escape comes back as it went in.
    let odd = extension("say \"hi\"\n\u{202e}é", "x");
    person.pin(&store, &tracker);
    let pinned = person.pin(&store, &odd);
    assert_eq!(pinned.standing(&tracker), Standing::Approved);
    assert_eq!(pinned.standing(&odd), Standing::Approved);

    // Read again by a store made afresh: a restart.
    let again = read(&PinStore::new(store.path(), clock()), &person);
    assert_eq!(again, pinned);
    let pin = again.get(&EXTENSION, "tracker").unwrap();
    assert_eq!(pin.hash, tracker.hash);
    assert_eq!(pin.definition(), tracker);
    assert_eq!(pin.at.as_str(), "2026-11-13T10:00:00Z");
    assert_eq!(
        again
            .list()
            .iter()
            .map(|pin| pin.name.as_str())
            .collect::<Vec<_>>(),
        ["tracker", odd.name.as_str()]
    );

    // The file, in the documented form: signed whole, at its second
    // revision.
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.starts_with("version = 2\n"), "{text}");
    for expected in [
        "revision = 2".to_owned(),
        "signature = \"".to_owned(),
        "[[pin]]".to_owned(),
        "kind = \"extension\"".to_owned(),
        "name = \"tracker\"".to_owned(),
        format!("hash = \"{}\"", tracker.hash),
        "at = \"2026-11-13T10:00:00Z\"".to_owned(),
    ] {
        assert!(text.contains(&expected), "no `{expected}` in:\n{text}");
    }
    assert_eq!(text.matches("[[pin]]").count(), 2, "{text}");
}

/// A definition of the user's own stands by its pinned hash, as a trusted
/// project's does by its pin in `trust.toml`.
#[test]
fn a_definition_of_the_users_own_stands_by_its_pinned_hash() {
    let scratch = cineris_testkit::scratch!("pins_standing");
    let store = store(&scratch);
    let person = Person::new();
    let tracker = extension("tracker", "manifest v1");
    let pins = person.pin(&store, &tracker);
    assert_eq!(pins.standing(&tracker), Standing::Approved);
    assert!(pins.standing(&tracker).is_approved());
    // Changed since it was pinned, by one character: not used.
    let edited = extension("tracker", "manifest v2");
    assert_eq!(pins.standing(&edited), Standing::Changed);
    assert!(!pins.standing(&edited).is_approved());
    // Never pinned.
    let notes = extension("notes", "manifest v1");
    assert_eq!(pins.standing(&notes), Standing::New);
    assert!(!pins.standing(&notes).is_approved());
    // The same name and hash of another kind is another thing.
    let server = Definition {
        kind: TrustResourceKind::McpServer,
        ..tracker.clone()
    };
    assert_eq!(pins.standing(&server), Standing::New);
    // Never untrusted: nothing pinned at all is new.
    assert_eq!(UserPins::default().standing(&tracker), Standing::New);
    // As read from the file.
    let read = read(&store, &person);
    assert_eq!(read.standing(&tracker), Standing::Approved);
    assert_eq!(read.standing(&edited), Standing::Changed);
    assert_eq!(read.standing(&notes), Standing::New);
}

/// Pins count only with the key of the session that reads them: with none
/// they approve nothing and still restrict, and another key's session reads
/// them as not theirs.
#[test]
fn pins_count_only_with_the_key_they_are_signed_by() {
    let scratch = cineris_testkit::scratch!("pins_key");
    let store = store(&scratch);
    let person = Person::new();
    let model = Person::with_password("the model's own password");
    let tracker = extension("tracker", "v1");
    person.pin(&store, &tracker);
    let keyless = store.pins(None).unwrap();
    assert_eq!(keyless.checked(), Checked::NoKey);
    assert!(!keyless.counts() && keyless.list().is_empty());
    assert_eq!(keyless.standing(&tracker), Standing::New);
    assert_eq!(keyless.held().len(), 1, "it still restricts");
    let theirs = store.pins(Some(&model.verifier())).unwrap();
    assert_eq!(theirs.checked(), Checked::NotTheirs);
    assert_eq!(theirs.standing(&tracker), Standing::New);
    // The model's own set, signed with its own key, counts for nothing in
    // the person's session.
    model.pin(&store, &extension("evil", "v1"));
    let mine = read(&store, &person);
    assert_eq!(mine.checked(), Checked::NotTheirs);
    assert!(mine.is_empty());
}

/// Pinning again replaces the pin of the same kind and name, in its place:
/// the file never holds two.
#[test]
fn pinning_again_replaces_the_pin_of_the_same_kind_and_name() {
    let scratch = cineris_testkit::scratch!("pins_replace");
    let fake = Arc::new(FakeClock::starting_at("2026-11-20T10:00:00Z").unwrap());
    let shared: SharedClock = fake.clone();
    let store = PinStore::new(scratch.join("state").join("pins.toml"), shared);
    let person = Person::new();
    let first = extension("tracker", "v1");
    let notes = extension("notes", "v1");
    person.pin(&store, &first);
    person.pin(&store, &notes);

    fake.advance(Duration::from_secs(60));
    let second = extension("tracker", "v2");
    let pins = person.pin(&store, &second);
    assert_eq!(pins.list().len(), 2, "{pins:?}");
    assert_eq!(pins.standing(&second), Standing::Approved);
    assert_eq!(
        pins.standing(&first),
        Standing::Changed,
        "the old one is gone"
    );
    assert_eq!(pins.standing(&notes), Standing::Approved);
    let pin = pins.get(&EXTENSION, "tracker").unwrap();
    assert_eq!(pin.at.as_str(), "2026-11-20T10:01:00Z");
    assert_eq!(pins.list()[0].name, "tracker", "kept in its place");
    assert_eq!(read(&store, &person), pins);

    // The same name of another kind is another pin, and replaces nothing.
    let server = Definition::of(TrustResourceKind::McpServer, "tracker", ["v1"]);
    let pins = person.pin(&store, &server);
    assert_eq!(pins.list().len(), 3);
    assert_eq!(pins.standing(&second), Standing::Approved);
    assert_eq!(pins.standing(&server), Standing::Approved);

    // Pinned again as it is: still one pin.
    let pins = person.pin(&store, &second);
    assert_eq!(pins.list().len(), 3);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(text.matches("name = \"tracker\"").count(), 2, "{text}");
    assert_eq!(read(&store, &person), pins);
}

#[test]
fn unpinning_says_whether_there_was_a_pin() {
    let scratch = cineris_testkit::scratch!("pins_unpin");
    let store = store(&scratch);
    let person = Person::new();
    let tracker = extension("tracker", "v1");
    let notes = extension("notes", "v1");
    person.pin(&store, &tracker);
    person.pin(&store, &notes);

    assert!(person.unpin(&store, &EXTENSION, "tracker"));
    assert!(!person.unpin(&store, &EXTENSION, "tracker"), "only once");
    // Another kind of the same name is not unpinned.
    assert!(!person.unpin(&store, &TrustResourceKind::McpServer, "notes"));
    let pins = read(&store, &person);
    assert_eq!(pins.standing(&tracker), Standing::New);
    assert_eq!(pins.standing(&notes), Standing::Approved);

    // The last one taken away leaves a signed file that pins nothing.
    assert!(person.unpin(&store, &EXTENSION, "notes"));
    let pins = read(&store, &person);
    assert!(pins.is_empty() && pins.counts());
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.starts_with("version = 2\n"), "{text}");
    assert!(!text.contains("[[pin]]"), "{text}");

    // Unpinning what was never pinned writes nothing.
    let other = PinStore::new(scratch.join("other").join("pins.toml"), clock());
    assert!(!person.unpin(&other, &EXTENSION, "tracker"));
    assert!(!other.path().exists());
}

#[test]
fn a_missing_file_pins_nothing_and_the_first_pin_makes_it() {
    let scratch = cineris_testkit::scratch!("pins_missing");
    let store = store(&scratch);
    let person = Person::new();
    let tracker = extension("tracker", "v1");
    let pins = read(&store, &person);
    assert!(pins.is_empty());
    assert!(pins.list().is_empty());
    assert!(pins.get(&EXTENSION, "tracker").is_none());
    assert_eq!(pins.standing(&tracker), Standing::New);
    assert_eq!(pins.checked(), Checked::Empty);
    assert!(!store.path().exists(), "reading creates nothing");
    assert!(!scratch.join("state").exists(), "not even its folder");

    person.pin(&store, &tracker);
    assert!(store.path().exists());
    assert!(
        private::is_private(&scratch.join("state")).unwrap(),
        "the folder it made is owner-only"
    );
    assert_eq!(read(&store, &person).standing(&tracker), Standing::Approved);
}

/// A pin store that is not understood in full pins nothing, says why, and
/// is not overwritten (decision 0028's rule).
#[test]
fn a_pin_store_that_is_not_understood_pins_nothing_and_is_left_as_it_is() {
    let hash = "0".repeat(64);
    let pin = |kind: &str, name: &str, hash: &str, extra: &str| {
        format!(
            "\n[[pin]]\nkind = {kind}\nname = {name}\nhash = {hash}\nat = \
             \"2026-11-13T10:00:00Z\"\n{extra}"
        )
    };
    let good = pin("\"extension\"", "\"tracker\"", &format!("\"{hash}\""), "");
    let person = Person::new();
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "a newer version",
            format!("version = 3\n{good}"),
            "version is 3",
        ),
        ("version 0", format!("version = 0\n{good}"), "version is 0"),
        ("no version", good.clone(), "version"),
        (
            "a version that is text",
            format!("version = \"1\"\n{good}"),
            "",
        ),
        (
            "an unknown key at the top",
            format!("version = 1\napprove_everything = true\n{good}"),
            "approve_everything",
        ),
        (
            "an unknown key in a pin",
            format!(
                "version = 1\n{}",
                pin(
                    "\"extension\"",
                    "\"tracker\"",
                    &format!("\"{hash}\""),
                    "scope = \"all\"\n"
                )
            ),
            "scope",
        ),
        (
            "a hash that is not one",
            format!(
                "version = 1\n{}",
                pin("\"extension\"", "\"tracker\"", "\"abc\"", "")
            ),
            "hexadecimal",
        ),
        (
            "a hash in capitals",
            format!(
                "version = 1\n{}",
                pin(
                    "\"extension\"",
                    "\"tracker\"",
                    &format!("\"{}\"", "A".repeat(64)),
                    ""
                )
            ),
            "hexadecimal",
        ),
        (
            "a hash that is a number",
            format!(
                "version = 1\n{}",
                pin("\"extension\"", "\"tracker\"", "5", "")
            ),
            "",
        ),
        (
            "a kind that is a number",
            format!(
                "version = 1\n{}",
                pin("3", "\"tracker\"", &format!("\"{hash}\""), "")
            ),
            "",
        ),
        (
            "a name that is a list",
            format!(
                "version = 1\n{}",
                pin("\"extension\"", "[\"tracker\"]", &format!("\"{hash}\""), "")
            ),
            "",
        ),
        (
            "a time that is not text",
            format!(
                "version = 1\n\n[[pin]]\nkind = \"extension\"\nname = \"tracker\"\nhash = \
                 \"{hash}\"\nat = 2026-11-13T10:00:00Z\n"
            ),
            "",
        ),
        (
            "a pin without its hash",
            "version = 1\n\n[[pin]]\nkind = \"extension\"\nname = \"tracker\"\nat = \
             \"2026-11-13T10:00:00Z\"\n"
                .to_owned(),
            "hash",
        ),
        (
            "two pins of one kind and name",
            format!("version = 2\n{good}{good}"),
            "twice",
        ),
        // Version 1 signed nothing: one that says it did is not one a build
        // wrote.
        (
            "a version 1 file with a revision",
            format!("version = 1\nrevision = 3\n{good}"),
            "version 1",
        ),
        (
            "a version 1 file with a signature",
            format!("version = 1\nsignature = \"{}\"\n{good}", "0".repeat(128)),
            "version 1",
        ),
        (
            "a signature that is not one",
            format!("version = 2\nrevision = 1\nsignature = \"abc\"\n{good}"),
            "signature",
        ),
        ("not TOML", "version = = 1".to_owned(), ""),
        (
            "too large",
            format!("version = 1\n{good}#{}\n", " ".repeat(4 * 1024 * 1024)),
            "bytes",
        ),
    ];
    for (name, contents, mentions) in cases {
        let scratch = cineris_testkit::scratch!("pins_strict");
        let file = scratch.file("state/pins.toml", &contents);
        let store = store(&scratch);
        let error = store.pins(Some(&person.verifier())).unwrap_err();
        assert!(
            matches!(error, PinError::NotUnderstood { .. }),
            "{name}: {error}"
        );
        let said = error.to_string();
        assert!(said.contains(mentions), "{name}: {said}");
        assert!(
            said.contains("nothing is taken as pinned"),
            "{name}: {said}"
        );
        // A change through it fails, and leaves it as it was.
        assert!(
            person
                .change_pins(&store, &pin_change(&extension("tracker", "v1")))
                .is_err(),
            "{name}"
        );
        assert!(
            person
                .change_pins(&store, &unpin_change("tracker"))
                .is_err(),
            "{name}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), contents, "{name}");
    }

    // The same file without a fault: with a byte-order mark and Windows'
    // line ends, as an editor may leave it. It is step 28's version, which
    // signed nothing: its pin holds, and approves nothing.
    let scratch = cineris_testkit::scratch!("pins_strict_ok");
    scratch.file(
        "state/pins.toml",
        format!("\u{feff}version = 1\n{good}").replace('\n', "\r\n"),
    );
    let pins = store(&scratch).pins(Some(&person.verifier())).unwrap();
    assert_eq!(pins.checked(), Checked::Unsigned);
    assert!(pins.list().is_empty());
    assert_eq!(pins.held().len(), 1);
    assert_eq!(pins.held()[0].hash.to_hex(), hash);
}

/// A file that cannot be read pins nothing, and the error says why.
#[test]
fn a_pin_store_that_cannot_be_read_pins_nothing_and_says_why() {
    let scratch = cineris_testkit::scratch!("pins_unreadable");
    // A folder where the file should be.
    scratch.dir("state/pins.toml");
    let store = store(&scratch);
    let person = Person::new();
    let error = store.pins(Some(&person.verifier())).unwrap_err();
    assert!(matches!(error, PinError::Read { .. }), "{error}");
    let said = error.to_string();
    assert!(said.contains("could not be read"), "{said}");
    assert!(said.contains("nothing is taken as pinned"), "{said}");
    assert!(
        person
            .change_pins(&store, &pin_change(&extension("tracker", "v1")))
            .is_err()
    );
}

/// A kind this build does not know (a later build's) is kept as it was
/// signed, written back with the next change, and approves nothing here.
#[test]
fn a_kind_this_build_does_not_know_is_kept_and_written_back() {
    let scratch = cineris_testkit::scratch!("pins_unknown_kind");
    let store = store(&scratch);
    let person = Person::new();
    let later = Definition::of(
        TrustResourceKind::from_wire("mcp_tool"),
        "docs/search",
        ["x"],
    );
    person.pin(&store, &later);
    let pins = read(&store, &person);
    assert_eq!(pins.list().len(), 1);
    assert_eq!(pins.list()[0].kind.as_str(), "mcp_tool");
    assert!(!pins.list()[0].kind.is_known());
    let tracker = extension("tracker", "v1");
    let after = person.pin(&store, &tracker);
    assert_eq!(after.list().len(), 2);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("kind = \"mcp_tool\""), "{text}");
    assert!(text.contains(&later.hash.to_hex()), "{text}");
    assert_eq!(read(&store, &person), after);
}

/// Every write leaves a file only its owner can open, a file made by hand
/// that others could read included, and leaves no temporary file behind.
#[test]
fn every_write_is_owner_only_and_leaves_no_temporary_file() {
    let scratch = cineris_testkit::scratch!("pins_private");
    let file = scratch.file("state/pins.toml", "version = 2\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!private::is_private(&file).unwrap());
    }
    let store = store(&scratch);
    let person = Person::new();
    person.pin(&store, &extension("tracker", "v1"));
    assert!(
        private::is_private(&file).unwrap(),
        "the file is owner-only once written"
    );
    person.pin(&store, &extension("notes", "v1"));
    person.pin(&store, &extension("tracker", "v2"));
    assert!(person.unpin(&store, &EXTENSION, "notes"));
    assert!(private::is_private(&file).unwrap());
    assert!(
        private::is_private(&private::lock_path(&file)).unwrap(),
        "and so is its lock"
    );
    assert_eq!(scratch.listing("state"), ["pins.toml", "pins.toml.lock"]);
}

/// Two sessions that pin at once never lose each other's pin: each change
/// is committed under the lock, held from its read to its write, and only
/// over the file it was worked out against, so that neither writes back a
/// file it read before the other's change. The test chooses the order. The
/// first writer has read the file under the lock and is stopped before its
/// write (`BEFORE_THE_PINS_ARE_WRITTEN`), and the second comes then. The
/// lock is held, so the second waits for it, and reads the file only once
/// the first has written it: it either was worked out against the file the
/// first wrote, and keeps its pin, or against the one before, and is told
/// the file moved (asked again), writing nothing. Where nothing held the
/// lock, the second would read the file the first read: the test waits
/// until it too is stopped before its write, and whichever then writes last
/// writes back a file without the other's pin, every time. No wait is on a
/// clock, and none is for a thread in the system's lock, where a test cannot
/// see it.
#[test]
fn a_writer_between_anothers_read_and_write_waits_and_loses_no_pin() {
    let scratch = cineris_testkit::scratch!("pins_two_writers");
    // The folder is there, as after any earlier change.
    scratch.dir("state");
    let file = scratch.join("state").join("pins.toml");
    let hold = cineris_testkit::Hold::new(BEFORE_THE_PINS_ARE_WRITTEN);
    // A store of its own for each, as each session has.
    let writer = |name: &'static str| {
        let store = PinStore::new(&file, clock());
        std::thread::spawn(move || {
            Person::new()
                .change_pins(&store, &pin_change(&extension(name, "v1")))
                .map(drop)
        })
    };
    let first = writer("first");
    hold.reach();
    // The first has read the file, and has not written it.
    let kept_out = lock_is_held(&file);
    let second = writer("second");
    if !kept_out {
        // Nothing keeps the second out: it reads what the first read.
        hold.reach_times(2);
    }
    hold.release();
    first.join().unwrap().unwrap();
    let second = second.join().unwrap();
    assert_eq!(hold.timed_out(), 0, "a writer was let go by the bound");
    let person = Person::new();
    let pins = read(&PinStore::new(&file, clock()), &person);
    let mut names: Vec<&str> = pins.list().iter().map(|pin| pin.name.as_str()).collect();
    names.sort_unstable();
    match second {
        Ok(()) => assert_eq!(names, ["first", "second"], "a pin was lost"),
        Err(PinError::Moved { .. }) => assert_eq!(names, ["first"], "a pin was lost"),
        Err(other) => panic!("{other}"),
    }
}

/// Whether the lock of the store at `file` is held: a second open of its
/// lock file cannot take it, as a second session's could not. Both Linux
/// (`flock`) and Windows (`LockFileEx`) keep two opens of one file apart, in
/// one process too. No lock file is no lock.
fn lock_is_held(file: &std::path::Path) -> bool {
    let Ok(other) = std::fs::File::open(private::lock_path(file)) else {
        return false;
    };
    match other.try_lock() {
        Ok(()) => false,
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(error)) => panic!("{error}"),
    }
}

/// A lock that cannot be taken stops every change, and no read: so each
/// change takes it. (A folder stands where the lock goes.)
#[test]
fn a_lock_that_cannot_be_taken_stops_every_change_and_no_read() {
    let scratch = cineris_testkit::scratch!("pins_lock_refused");
    let store = store(&scratch);
    let person = Person::new();
    let tracker = extension("tracker", "v1");
    person.pin(&store, &tracker);
    let before = std::fs::read_to_string(store.path()).unwrap();
    let lock = private::lock_path(store.path());
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    for result in [
        person
            .change_pins(&store, &pin_change(&extension("notes", "v1")))
            .map(drop),
        person
            .change_pins(&store, &unpin_change("tracker"))
            .map(drop),
    ] {
        assert!(
            matches!(
                &result,
                Err(PinError::Write(AtomicWriteError::Io { step: "lock", .. }))
            ),
            "{result:?}"
        );
    }
    assert_eq!(std::fs::read_to_string(store.path()).unwrap(), before);
    assert_eq!(read(&store, &person).standing(&tracker), Standing::Approved);
}

/// Nothing a tool does can reach the user's pins, at any level and whatever
/// the rules allow: they are in the state directory, which the built-in
/// protection closes. A model that could write `pins.toml` could approve
/// its own change to an extension of the user's.
#[test]
fn no_tool_can_read_or_write_the_users_pins() {
    let engine = engine_with(|parts| {
        parts.level = Level::Unrestricted;
        parts.rules = vec![
            user(Effect::Read, "/**", RuleAction::Allow),
            user(Effect::Write, "/**", RuleAction::Allow),
            user(Effect::Exec, "*", RuleAction::Allow),
        ];
    });
    for file in [
        "/users/dev/.local/state/cineris/pins.toml",
        "/users/dev/.local/state/cineris/pins.toml.lock",
        "/users/dev/.local/state/cineris/approval-key.toml",
        // What a write that was killed before its rename leaves.
        "/users/dev/.local/state/cineris/.cineris-0123456789abcdef.tmp",
    ] {
        assert_eq!(said(&crate::read(&engine, file)), Deny, "read {file}");
        assert_eq!(said(&crate::write(&engine, file)), Deny, "write {file}");
        for command in [
            format!("cat {file}"),
            format!("cp {file} /tmp/x"),
            format!("cp /tmp/x {file}"),
            format!("echo x > {file}"),
        ] {
            assert_eq!(said(&shell(&engine, &command)), Deny, "{command:?}");
        }
    }
    // Approving its own change is writing that file.
    assert_eq!(
        said(&shell(
            &engine,
            "echo '[[pin]]' >> ~/.local/state/cineris/pins.toml"
        )),
        Deny
    );
}

/// Makes `link` a link to the file `target`; `false`, said, where links
/// cannot be made (a Windows account without the right), except on a CI
/// runner, which must make one.
fn file_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            println!("skipped: cannot make a link here ({error})");
            false
        }
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

/// A link at the store's name is not followed, as the registration
/// cache's and an extension's values' are not: what it names is not read
/// as the user's pins, so nothing is taken as pinned and the error says
/// why, and no change writes through it.
#[test]
fn a_link_at_the_stores_name_is_neither_read_nor_written_through() {
    let scratch = cineris_testkit::scratch!("pins_link");
    let hash = "1".repeat(64);
    let named = format!(
        "version = 1\n\n[[pin]]\nkind = \"extension\"\nname = \"tracker\"\nhash = \
         \"{hash}\"\nat = \"2026-11-13T10:00:00Z\"\n"
    );
    let elsewhere = scratch.file("elsewhere/pins.toml", &named);
    scratch.dir("state");
    let store = store(&scratch);
    let person = Person::new();
    if !file_link(&elsewhere, store.path()) {
        return;
    }
    let error = store.pins(Some(&person.verifier())).unwrap_err();
    assert!(matches!(error, PinError::Read { .. }), "{error}");
    let said = error.to_string();
    assert!(said.contains("link"), "{said}");
    assert!(said.contains("nothing is taken as pinned"), "{said}");
    assert!(
        person
            .change_pins(&store, &pin_change(&extension("notes", "v1")))
            .is_err()
    );
    assert!(
        person
            .change_pins(&store, &unpin_change("tracker"))
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), named);
    assert!(
        std::fs::symlink_metadata(store.path())
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is left as it is"
    );
}
