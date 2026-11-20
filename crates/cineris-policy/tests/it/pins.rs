//! The user's own pins (`pins.toml`, since step 28) on a real file system:
//! one pin for each kind and name, read strictly, written owner-only and
//! atomically under a lock, and closed to every tool.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use cineris_base::{AtomicWriteError, FakeClock, SharedClock, private};
use cineris_policy::{Definition, PinError, PinStore, Standing, UserPins};
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

#[test]
fn a_pin_is_written_and_read_back() {
    let scratch = cineris_testkit::scratch!("pins_round_trip");
    let store = store(&scratch);
    let tracker = extension("tracker", "the manifest and the script");
    // A name the file must escape comes back as it went in.
    let odd = extension("say \"hi\"\n\u{202e}é", "x");
    store.pin(&tracker).unwrap();
    let pinned = store.pin(&odd).unwrap();
    assert_eq!(pinned.standing(&tracker), Standing::Approved);
    assert_eq!(pinned.standing(&odd), Standing::Approved);

    // Read again by a store made afresh: a restart.
    let again = PinStore::new(store.path(), clock()).pins().unwrap();
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

    // The file, in the documented form.
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.starts_with("version = 1\n"), "{text}");
    for expected in [
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
    let tracker = extension("tracker", "manifest v1");
    let pins = store.pin(&tracker).unwrap();
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
    let read = store.pins().unwrap();
    assert_eq!(read.standing(&tracker), Standing::Approved);
    assert_eq!(read.standing(&edited), Standing::Changed);
    assert_eq!(read.standing(&notes), Standing::New);
}

/// Pinning again replaces the pin of the same kind and name, in its place:
/// the file never holds two.
#[test]
fn pinning_again_replaces_the_pin_of_the_same_kind_and_name() {
    let scratch = cineris_testkit::scratch!("pins_replace");
    let fake = Arc::new(FakeClock::starting_at("2026-11-20T10:00:00Z").unwrap());
    let shared: SharedClock = fake.clone();
    let store = PinStore::new(scratch.join("state").join("pins.toml"), shared);
    let first = extension("tracker", "v1");
    let notes = extension("notes", "v1");
    store.pin(&first).unwrap();
    store.pin(&notes).unwrap();

    fake.advance(Duration::from_secs(60));
    let second = extension("tracker", "v2");
    let pins = store.pin(&second).unwrap();
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
    assert_eq!(store.pins().unwrap(), pins);

    // The same name of another kind is another pin, and replaces nothing.
    let server = Definition::of(TrustResourceKind::McpServer, "tracker", ["v1"]);
    let pins = store.pin(&server).unwrap();
    assert_eq!(pins.list().len(), 3);
    assert_eq!(pins.standing(&second), Standing::Approved);
    assert_eq!(pins.standing(&server), Standing::Approved);

    // Pinned again as it is: still one pin.
    let pins = store.pin(&second).unwrap();
    assert_eq!(pins.list().len(), 3);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(text.matches("name = \"tracker\"").count(), 2, "{text}");
    assert_eq!(store.pins().unwrap(), pins);
}

#[test]
fn unpinning_says_whether_there_was_a_pin() {
    let scratch = cineris_testkit::scratch!("pins_unpin");
    let store = store(&scratch);
    let tracker = extension("tracker", "v1");
    let notes = extension("notes", "v1");
    store.pin(&tracker).unwrap();
    store.pin(&notes).unwrap();

    assert!(store.unpin(&EXTENSION, "tracker").unwrap());
    assert!(!store.unpin(&EXTENSION, "tracker").unwrap(), "only once");
    // Another kind of the same name is not unpinned.
    assert!(!store.unpin(&TrustResourceKind::McpServer, "notes").unwrap());
    let pins = store.pins().unwrap();
    assert_eq!(pins.standing(&tracker), Standing::New);
    assert_eq!(pins.standing(&notes), Standing::Approved);

    // The last one taken away leaves a file that pins nothing.
    assert!(store.unpin(&EXTENSION, "notes").unwrap());
    assert!(store.pins().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(store.path()).unwrap(),
        "version = 1\n"
    );

    // Unpinning what was never pinned writes nothing.
    let other = PinStore::new(scratch.join("other").join("pins.toml"), clock());
    assert!(!other.unpin(&EXTENSION, "tracker").unwrap());
    assert!(!other.path().exists());
}

#[test]
fn a_missing_file_pins_nothing_and_the_first_pin_makes_it() {
    let scratch = cineris_testkit::scratch!("pins_missing");
    let store = store(&scratch);
    let tracker = extension("tracker", "v1");
    let pins = store.pins().unwrap();
    assert!(pins.is_empty());
    assert!(pins.list().is_empty());
    assert!(pins.get(&EXTENSION, "tracker").is_none());
    assert_eq!(pins.standing(&tracker), Standing::New);
    assert!(!store.path().exists(), "reading creates nothing");
    assert!(!scratch.join("state").exists(), "not even its folder");

    store.pin(&tracker).unwrap();
    assert!(store.path().exists());
    assert!(
        private::is_private(&scratch.join("state")).unwrap(),
        "the folder it made is owner-only"
    );
    assert_eq!(store.pins().unwrap().standing(&tracker), Standing::Approved);
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
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "a newer version",
            format!("version = 2\n{good}"),
            "version is 2",
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
            format!("version = 1\n{good}{good}"),
            "twice",
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
        let error = store.pins().unwrap_err();
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
        assert!(store.pin(&extension("tracker", "v1")).is_err(), "{name}");
        assert!(store.unpin(&EXTENSION, "tracker").is_err(), "{name}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), contents, "{name}");
    }

    // The same file without a fault: with a byte-order mark and Windows'
    // line ends, as an editor may leave it.
    let scratch = cineris_testkit::scratch!("pins_strict_ok");
    scratch.file(
        "state/pins.toml",
        format!("\u{feff}version = 1\n{good}").replace('\n', "\r\n"),
    );
    let pins = store(&scratch).pins().unwrap();
    assert_eq!(pins.list().len(), 1);
    assert_eq!(pins.list()[0].hash.to_hex(), hash);
}

/// A file that cannot be read pins nothing, and the error says why.
#[test]
fn a_pin_store_that_cannot_be_read_pins_nothing_and_says_why() {
    let scratch = cineris_testkit::scratch!("pins_unreadable");
    // A folder where the file should be.
    scratch.dir("state/pins.toml");
    let store = store(&scratch);
    let error = store.pins().unwrap_err();
    assert!(matches!(error, PinError::Read { .. }), "{error}");
    let said = error.to_string();
    assert!(said.contains("could not be read"), "{said}");
    assert!(said.contains("nothing is taken as pinned"), "{said}");
    assert!(store.pin(&extension("tracker", "v1")).is_err());
}

/// A kind this build does not know (a later build's) is kept as it was
/// written, written back with the next change, and approves nothing here.
#[test]
fn a_kind_this_build_does_not_know_is_kept_and_written_back() {
    let scratch = cineris_testkit::scratch!("pins_unknown_kind");
    let hash = "1".repeat(64);
    scratch.file(
        "state/pins.toml",
        format!(
            "version = 1\n\n[[pin]]\nkind = \"mcp_tool\"\nname = \"docs/search\"\nhash = \
             \"{hash}\"\nat = \"2026-11-13T10:00:00Z\"\n"
        ),
    );
    let store = store(&scratch);
    let pins = store.pins().unwrap();
    assert_eq!(pins.list().len(), 1);
    assert_eq!(pins.list()[0].kind.as_str(), "mcp_tool");
    assert!(!pins.list()[0].kind.is_known());
    let tracker = extension("tracker", "v1");
    let after = store.pin(&tracker).unwrap();
    assert_eq!(after.list().len(), 2);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("kind = \"mcp_tool\""), "{text}");
    assert!(text.contains(&hash), "{text}");
    assert_eq!(store.pins().unwrap(), after);
}

/// Every write leaves a file only its owner can open, a file made by hand
/// that others could read included, and leaves no temporary file behind.
#[test]
fn every_write_is_owner_only_and_leaves_no_temporary_file() {
    let scratch = cineris_testkit::scratch!("pins_private");
    let file = scratch.file("state/pins.toml", "version = 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!private::is_private(&file).unwrap());
    }
    let store = store(&scratch);
    store.pin(&extension("tracker", "v1")).unwrap();
    assert!(
        private::is_private(&file).unwrap(),
        "the file is owner-only once written"
    );
    store.pin(&extension("notes", "v1")).unwrap();
    store.pin(&extension("tracker", "v2")).unwrap();
    assert!(store.unpin(&EXTENSION, "notes").unwrap());
    assert!(private::is_private(&file).unwrap());
    assert!(
        private::is_private(&private::lock_path(&file)).unwrap(),
        "and so is its lock"
    );
    assert_eq!(scratch.listing("state"), ["pins.toml", "pins.toml.lock"]);
}

/// Two sessions that pin at once both keep their pins: each change holds
/// the lock from its read to its write, so that neither writes back a file
/// it read before the other's change. Nothing waits on a clock: the writers
/// start together and take turns at the lock.
#[test]
fn two_writers_at_once_both_keep_their_pins() {
    const WRITERS: usize = 4;
    const EACH: usize = 6;
    let scratch = cineris_testkit::scratch!("pins_two_writers");
    let file = scratch.join("state").join("pins.toml");
    let start = Arc::new(Barrier::new(WRITERS));
    let writers: Vec<_> = (0..WRITERS)
        .map(|writer| {
            // A store of its own for each, as each session has.
            let store = PinStore::new(&file, clock());
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                for each in 0..EACH {
                    store
                        .pin(&extension(&format!("w{writer}-{each}"), "v1"))
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let pins = PinStore::new(&file, clock()).pins().unwrap();
    let mut names: Vec<&str> = pins.list().iter().map(|pin| pin.name.as_str()).collect();
    names.sort_unstable();
    let mut expected: Vec<String> = (0..WRITERS)
        .flat_map(|writer| (0..EACH).map(move |each| format!("w{writer}-{each}")))
        .collect();
    expected.sort_unstable();
    assert_eq!(names, expected, "a pin was lost");
}

/// A lock that cannot be taken stops every change, and no read: so each
/// change takes it. (A folder stands where the lock goes.)
#[test]
fn a_lock_that_cannot_be_taken_stops_every_change_and_no_read() {
    let scratch = cineris_testkit::scratch!("pins_lock_refused");
    let store = store(&scratch);
    let tracker = extension("tracker", "v1");
    store.pin(&tracker).unwrap();
    let before = std::fs::read_to_string(store.path()).unwrap();
    let lock = private::lock_path(store.path());
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    for result in [
        store.pin(&extension("notes", "v1")).map(drop),
        store.unpin(&EXTENSION, "tracker").map(drop),
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
    assert_eq!(store.pins().unwrap().standing(&tracker), Standing::Approved);
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
