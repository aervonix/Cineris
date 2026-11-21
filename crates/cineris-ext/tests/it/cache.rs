//! The registration cache keeps each approved extension's registration by
//! its hash, is read strictly and whole or not at all, sets aside what it
//! cannot read, and is written owner-only and atomically.

use std::path::PathBuf;

use cineris_base::{ContentHash, private};
use cineris_ext::cache::{
    CacheError, Entry, Loaded, MAX_BYTES, MAX_ENTRIES, RegistrationCache, VERSION,
};
use cineris_ext::registration::{CommandDef, HookDef, Registration, ToolDef};
use cineris_testkit::scratch;
use cineris_testkit::scratch::Scratch;
use serde_json::{Value as Json, json};

/// The cache in a state directory of the test's own, which is not made.
fn cache(scratch: &Scratch) -> RegistrationCache {
    RegistrationCache::new(file(scratch))
}

fn file(scratch: &Scratch) -> PathBuf {
    scratch.join("state/extensions/registered.json")
}

fn hash(of: &str) -> ContentHash {
    ContentHash::of(of.as_bytes())
}

/// An entry with one of everything.
fn entry(name: &str) -> Entry {
    Entry {
        name: name.to_owned(),
        api: 1,
        registration: Registration {
            tools: vec![ToolDef {
                name: "find_issue".to_owned(),
                description: "Search issues by text".to_owned(),
                input: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
                read_only: true,
            }],
            commands: vec![CommandDef {
                name: "triage".to_owned(),
                description: "Sort the new issues".to_owned(),
                arguments: vec!["label".to_owned()],
            }],
            hooks: vec![HookDef {
                event: "tool.before".to_owned(),
                tools: vec!["edit".to_owned(), "write".to_owned()],
                observe: false,
                timeout_ms: Some(500),
            }],
        },
        prompt: Some(format!("Use {name}'s tools for issues.")),
    }
}

/// The text of an entry as the file holds it.
fn entry_json(name: &str) -> String {
    format!(r#"{{"name": "{name}", "api": 1, "tools": [], "commands": [], "hooks": []}}"#)
}

/// A file of the given entries, each `(key, entry's JSON)`.
fn file_of(entries: &[(String, String)]) -> String {
    let entries: Vec<String> = entries
        .iter()
        .map(|(key, entry)| format!(r#""{key}": {entry}"#))
        .collect();
    format!(r#"{{"version": 1, "entries": {{{}}}}}"#, entries.join(", "))
}

#[test]
fn an_entry_put_is_read_back_and_the_file_is_the_designs() {
    let scratch = scratch!("an_entry_put_is_read_back_and_the_file_is_the_designs");
    let cache = cache(&scratch);
    let tracker = entry("tracker");
    let mut plain = entry("plain");
    plain.prompt = None;
    plain.registration = Registration::default();
    cache.put(&hash("tracker"), tracker.clone()).unwrap();
    cache.put(&hash("plain"), plain.clone()).unwrap();

    let loaded = cache.load();
    assert_eq!(loaded.set_aside, None);
    assert_eq!(loaded.cache.len(), 2);
    assert_eq!(loaded.cache.get(&hash("tracker")), Some(&tracker));
    assert_eq!(loaded.cache.get(&hash("plain")), Some(&plain));
    assert_eq!(loaded.cache.get(&hash("other")), None);

    // `{"version": 1, "entries": {"<hash>": {"name", "api", "tools",
    // "commands", "hooks", "prompt"}}}`, with no prompt where there is none.
    let written: Json = serde_json::from_slice(&scratch.read(file(&scratch))).unwrap();
    assert_eq!(written["version"], json!(VERSION));
    let entries = written["entries"].as_object().unwrap();
    let keys: Vec<&String> = entries.keys().collect();
    let mut hashes = [hash("tracker").to_hex(), hash("plain").to_hex()];
    hashes.sort();
    assert_eq!(keys, hashes.iter().collect::<Vec<_>>());
    let fields = |key: &str| {
        let mut fields: Vec<String> = entries[key].as_object().unwrap().keys().cloned().collect();
        fields.sort();
        fields
    };
    assert_eq!(
        fields(&hash("tracker").to_hex()),
        ["api", "commands", "hooks", "name", "prompt", "tools"]
    );
    assert_eq!(
        fields(&hash("plain").to_hex()),
        ["api", "commands", "hooks", "name", "tools"]
    );
    assert_eq!(
        entries[&hash("tracker").to_hex()]["tools"][0]["name"],
        json!("find_issue")
    );
    assert_eq!(
        entries[&hash("tracker").to_hex()]["prompt"],
        json!("Use tracker's tools for issues.")
    );
}

#[test]
fn a_missing_file_is_an_empty_cache_and_reading_it_makes_nothing() {
    let scratch = scratch!("a_missing_file_is_an_empty_cache_and_reading_it_makes_nothing");
    let loaded = cache(&scratch).load();
    assert_eq!(loaded, Loaded::default());
    assert!(loaded.cache.is_empty());
    assert!(!scratch.join("state").exists(), "nothing was made");
}

#[test]
fn a_put_keeps_the_other_entries_and_replaces_its_own() {
    let scratch = scratch!("a_put_keeps_the_other_entries_and_replaces_its_own");
    let cache = cache(&scratch);
    cache.put(&hash("a"), entry("first")).unwrap();
    cache.put(&hash("b"), entry("second")).unwrap();
    let mut again = entry("first");
    again.prompt = Some("Changed.".to_owned());
    cache.put(&hash("a"), again.clone()).unwrap();
    let loaded = cache.load();
    assert_eq!(loaded.set_aside, None);
    let names: Vec<(&ContentHash, &str)> = loaded
        .cache
        .iter()
        .map(|(hash, entry)| (hash, entry.name.as_str()))
        .collect();
    let mut expected = [(hash("a"), "first"), (hash("b"), "second")];
    expected.sort();
    assert_eq!(
        names,
        expected
            .iter()
            .map(|(hash, name)| (hash, *name))
            .collect::<Vec<_>>()
    );
    assert_eq!(loaded.cache.get(&hash("a")), Some(&again));
}

#[test]
fn a_file_not_understood_whole_is_set_aside_and_read_as_empty() {
    let scratch = scratch!("a_file_not_understood_whole_is_set_aside_and_read_as_empty");
    let cache = cache(&scratch);
    let key = hash("tracker").to_hex();
    let good = entry_json("tracker");
    let many: Vec<(String, String)> = (0..=MAX_ENTRIES)
        .map(|index| (hash(&index.to_string()).to_hex(), entry_json("e")))
        .collect();
    let tools: Vec<String> = (0..65)
        .map(|index| format!(r#"{{"name": "t{index}", "description": "", "input": {{}}}}"#))
        .collect();
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "an unknown field at the top",
            r#"{"version": 1, "entries": {}, "extra": 1}"#.to_owned(),
            "unknown field `extra`",
        ),
        (
            "an unknown field in an entry",
            file_of(&[(
                key.clone(),
                r#"{"name": "t", "api": 1, "tools": [], "commands": [], "hooks": [], "color": "red"}"#
                    .to_owned(),
            )]),
            "unknown field `color`",
        ),
        (
            "an unknown field in a tool",
            file_of(&[(
                key.clone(),
                r#"{"name": "t", "api": 1, "tools": [{"name": "a", "description": "", "input": {}, "danger": true}], "commands": [], "hooks": []}"#
                    .to_owned(),
            )]),
            "unknown field `danger`",
        ),
        (
            "a value of the wrong type",
            file_of(&[(
                key.clone(),
                r#"{"name": "t", "api": "1", "tools": [], "commands": [], "hooks": []}"#.to_owned(),
            )]),
            "invalid type",
        ),
        (
            "a field missing",
            file_of(&[(
                key.clone(),
                r#"{"name": "t", "api": 1, "tools": [], "commands": []}"#.to_owned(),
            )]),
            "missing field `hooks`",
        ),
        (
            "a newer version",
            r#"{"version": 2, "entries": {}}"#.to_owned(),
            "it is of version 2, written by a newer build, and this build reads version 1",
        ),
        (
            "a newer version, which adds a field",
            r#"{"version": 2, "entries": {}, "signed": true}"#.to_owned(),
            "it is of version 2, written by a newer build, and this build reads version 1",
        ),
        (
            "a version no build writes",
            r#"{"version": 0, "entries": {}}"#.to_owned(),
            "it is of version 0, and this build reads version 1",
        ),
        (
            "text that is not JSON",
            r#"{"version": 1, "entries": {"#.to_owned(),
            "it is not a registration cache this build reads in full",
        ),
        (
            "an empty file",
            String::new(),
            "it is not a registration cache this build reads in full",
        ),
        (
            "a key that is not a hash",
            file_of(&[("abc".to_owned(), good.clone())]),
            "a content hash is 64 lowercase hexadecimal digits",
        ),
        (
            "a hash in upper case",
            file_of(&[(key.to_uppercase(), good.clone())]),
            "a content hash is 64 lowercase hexadecimal digits",
        ),
        (
            "a key given twice",
            file_of(&[(key.clone(), good.clone()), (key.clone(), good.clone())]),
            "twice",
        ),
        (
            "more than 1024 entries",
            file_of(&many),
            "it holds more than 1024 entries",
        ),
        (
            "an entry with 65 tools",
            file_of(&[(
                key.clone(),
                format!(
                    r#"{{"name": "t", "api": 1, "tools": [{}], "commands": [], "hooks": []}}"#,
                    tools.join(", ")
                ),
            )]),
            "is past a registration's caps: it has 65 tools, and an extension registers at most 64",
        ),
        (
            "an entry with a prompt over 4 KiB",
            file_of(&[(
                key.clone(),
                format!(
                    r#"{{"name": "t", "api": 1, "tools": [], "commands": [], "hooks": [], "prompt": "{}"}}"#,
                    "p".repeat(4097)
                ),
            )]),
            "its prompt is 4097 bytes, more than the 4096 a prompt file may hold",
        ),
    ];
    let aside = cache.set_aside_path();
    for (label, text, says) in cases {
        scratch.file(file(&scratch), &text);
        let loaded = cache.load();
        assert!(loaded.cache.is_empty(), "{label}: nothing of it is read");
        let reason = loaded
            .set_aside
            .unwrap_or_else(|| panic!("{label}: it was not set aside"));
        assert!(reason.contains(says), "{label}: {reason}");
        assert!(
            reason.starts_with(&format!(
                "the registration cache `{}` was set aside",
                file(&scratch).display()
            )),
            "{label}: {reason}"
        );
        assert!(
            reason.ends_with(&format!(" It was moved to `{}`.", aside.display())),
            "{label}: {reason}"
        );
        // Moved aside as it was, and the next read starts from nothing.
        assert!(!file(&scratch).exists(), "{label}");
        assert_eq!(scratch.read_text(&aside), text, "{label}");
        assert_eq!(cache.load(), Loaded::default(), "{label}");
    }
}

#[test]
fn a_file_larger_than_16_mib_is_set_aside_unread() {
    let scratch = scratch!("a_file_larger_than_16_mib_is_set_aside_unread");
    let cache = cache(&scratch);
    // A file with no blocks of its own: its size is all that is asked.
    let large =
        std::fs::File::create(scratch.dir("state/extensions").join("registered.json")).unwrap();
    large.set_len(MAX_BYTES + 1).unwrap();
    drop(large);
    let loaded = cache.load();
    assert!(loaded.cache.is_empty());
    let reason = loaded.set_aside.unwrap();
    assert!(
        reason.contains("it is 16777217 bytes, more than the 16777216 a registration cache may be"),
        "{reason}"
    );
    assert!(cache.set_aside_path().exists());
}

#[test]
fn a_put_over_a_file_not_understood_sets_it_aside_and_begins_a_new_one() {
    let scratch = scratch!("a_put_over_a_file_not_understood_sets_it_aside_and_begins_a_new_one");
    let cache = cache(&scratch);
    scratch.file(file(&scratch), r#"{"version": 9, "entries": {}}"#);
    cache.put(&hash("tracker"), entry("tracker")).unwrap();
    assert_eq!(
        scratch.read_text(cache.set_aside_path()),
        r#"{"version": 9, "entries": {}}"#
    );
    let loaded = cache.load();
    assert_eq!(loaded.set_aside, None);
    assert_eq!(loaded.cache.len(), 1);
    assert_eq!(loaded.cache.get(&hash("tracker")), Some(&entry("tracker")));
}

#[test]
fn a_prune_takes_out_the_hashes_no_pin_names_and_writes_only_when_it_did() {
    let scratch = scratch!("a_prune_takes_out_the_hashes_no_pin_names_and_writes_only_when_it_did");
    let cache = cache(&scratch);
    for name in ["kept", "gone", "also-gone"] {
        cache.put(&hash(name), entry(name)).unwrap();
    }
    let pinned = [hash("kept")];
    assert_eq!(cache.prune(|hash| pinned.contains(hash)).unwrap(), 2);
    let loaded = cache.load();
    assert_eq!(loaded.cache.len(), 1);
    assert_eq!(loaded.cache.get(&hash("kept")), Some(&entry("kept")));
    // Nothing to take out: the file is not written again, so a file laid
    // out otherwise than this build writes it stays as it is.
    let written: Json = serde_json::from_slice(&scratch.read(file(&scratch))).unwrap();
    let before = serde_json::to_string_pretty(&written).unwrap();
    scratch.file(file(&scratch), &before);
    assert_eq!(cache.prune(|_| true).unwrap(), 0);
    assert_eq!(scratch.read_text(file(&scratch)), before);
    assert_eq!(cache.load().cache.len(), 1);
}

#[test]
fn every_write_is_owner_only_and_leaves_no_temporary_file() {
    let scratch = scratch!("every_write_is_owner_only_and_leaves_no_temporary_file");
    let cache = cache(&scratch);
    cache.put(&hash("one"), entry("one")).unwrap();
    // Its folder was made owner-only too.
    assert!(private::is_private(&scratch.join("state/extensions")).unwrap());
    assert!(private::is_private(&file(&scratch)).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file(&scratch), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!private::is_private(&file(&scratch)).unwrap());
    }
    cache.put(&hash("two"), entry("two")).unwrap();
    assert!(
        private::is_private(&file(&scratch)).unwrap(),
        "a write leaves it owner-only, whatever it was"
    );
    assert_eq!(scratch.listing("state/extensions"), ["registered.json"]);
}

#[test]
fn a_link_at_the_files_name_is_never_written_through() {
    let scratch = scratch!("a_link_at_the_files_name_is_never_written_through");
    let cache = cache(&scratch);
    let target = scratch.file("elsewhere.json", "kept");
    scratch.dir("state/extensions");
    if !crate::make_link(&target, &file(&scratch), false) {
        return;
    }
    let error = cache.put(&hash("one"), entry("one")).unwrap_err();
    assert!(
        matches!(&error, CacheError::Write { .. }) && error.to_string().contains("is a link"),
        "{error}"
    );
    assert_eq!(scratch.read_text("elsewhere.json"), "kept");
    // Read, it is not followed either: set aside, and left where it is.
    let reason = cache.load().set_aside.unwrap();
    assert!(reason.contains("it is a link"), "{reason}");
    assert!(reason.ends_with(" It was left as it is."), "{reason}");
    assert_eq!(scratch.read_text("elsewhere.json"), "kept");
}

#[test]
fn a_put_that_would_make_a_file_set_aside_is_refused_and_writes_nothing() {
    let scratch = scratch!("a_put_that_would_make_a_file_set_aside_is_refused_and_writes_nothing");
    let cache = cache(&scratch);
    // An entry past a registration's caps.
    let mut long = entry("long");
    long.prompt = Some("p".repeat(4097));
    let error = cache.put(&hash("long"), long).unwrap_err();
    assert!(matches!(error, CacheError::Entry { .. }), "{error}");
    assert_eq!(
        error.to_string(),
        "the registration of `long` is not one the registration cache keeps: its prompt is 4097 \
         bytes, more than the 4096 a prompt file may hold"
    );
    let mut many = entry("many");
    many.registration.hooks = vec![many.registration.hooks[0].clone(); 33];
    assert!(matches!(
        cache.put(&hash("many"), many),
        Err(CacheError::Entry { .. })
    ));
    assert!(!file(&scratch).exists(), "nothing was written");

    // A file of 1024 entries takes no new one, and still takes one of its
    // own hashes.
    let full: Vec<(String, String)> = (0..MAX_ENTRIES)
        .map(|index| (hash(&index.to_string()).to_hex(), entry_json("e")))
        .collect();
    scratch.file(file(&scratch), file_of(&full));
    assert_eq!(cache.load().cache.len(), MAX_ENTRIES);
    let before = scratch.read(file(&scratch));
    let error = cache.put(&hash("one more"), entry("one")).unwrap_err();
    assert!(matches!(error, CacheError::Full { .. }), "{error}");
    assert_eq!(scratch.read(file(&scratch)), before);
    cache.put(&hash("0"), entry("again")).unwrap();
    assert_eq!(
        cache
            .load()
            .cache
            .get(&hash("0"))
            .map(|entry| entry.name.as_str()),
        Some("again")
    );

    // An entry whose text would make the file larger than 16 MiB.
    std::fs::remove_file(file(&scratch)).unwrap();
    let mut huge = entry("huge");
    huge.registration.tools[0].description = "d".repeat(usize::try_from(MAX_BYTES).unwrap());
    let error = cache.put(&hash("huge"), huge).unwrap_err();
    assert!(
        matches!(error, CacheError::TooLarge { bytes, .. } if bytes as u64 > MAX_BYTES),
        "{error}"
    );
    assert!(!file(&scratch).exists(), "nothing was written");
}
