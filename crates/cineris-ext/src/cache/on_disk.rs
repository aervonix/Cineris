//! The registration cache keeps each approved extension's registration by
//! its hash, is read strictly and whole or not at all, sets aside what it
//! cannot read, and is written owner-only and atomically.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "a test prepares real files, fails by panicking, and says when it skips"
)]

use std::path::{Path, PathBuf};

use crate::cache::{
    BEFORE_IT_IS_SET_ASIDE, BEFORE_IT_IS_WRITTEN, CacheError, Entry, Loaded, MAX_BYTES,
    MAX_ENTRIES, RegistrationCache, VERSION,
};
use crate::manifest::Manifest;
use crate::registration::{CommandDef, HookDef, Registration, ToolDef};
use cineris_base::{ContentHash, private};
use cineris_testkit::scratch::Scratch;
use serde_json::{Value as Json, json};

/// Makes a symbolic link to a file or, with `directory`, to a folder. On
/// Windows that needs a privilege an ordinary account lacks unless
/// Developer Mode is on; the CI runner has it, and there the link must be
/// made. Elsewhere `false` when it could not be, and the test says so and
/// stops.
fn make_link(target: &Path, link: &Path, directory: bool) -> bool {
    #[cfg(unix)]
    let made = {
        let _ = directory;
        std::os::unix::fs::symlink(target, link)
    };
    #[cfg(windows)]
    let made = if directory {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            println!("skipped: cannot make a link here ({error})");
            false
        }
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

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

/// The text of an entry of the extension `t` with these tools, commands and
/// hooks, each list's JSON without its brackets.
fn entry_of(tools: &str, commands: &str, hooks: &str) -> String {
    format!(
        r#"{{"name": "t", "api": 1, "tools": [{tools}], "commands": [{commands}], "hooks": [{hooks}]}}"#
    )
}

/// A tool's JSON.
fn tool_json(name: &str, description: &str, input: &str) -> String {
    format!(r#"{{"name": "{name}", "description": "{description}", "input": {input}}}"#)
}

/// A command's JSON.
fn command_json(name: &str, arguments: &str) -> String {
    format!(r#"{{"name": "{name}", "description": "", "arguments": [{arguments}]}}"#)
}

/// A hook's JSON.
fn hook_json(event: &str, tools: &str, timeout: &str) -> String {
    format!(r#"{{"event": "{event}", "tools": [{tools}], "timeout_ms": {timeout}}}"#)
}

/// `count` texts `"<stem><n>"`, as a JSON list's items.
fn texts(stem: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!(r#""{stem}{index}""#))
        .collect::<Vec<_>>()
        .join(", ")
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "an_entry_put_is_read_back_and_the_file_is_the_designs",
    );
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_missing_file_is_an_empty_cache_and_reading_it_makes_nothing",
    );
    let loaded = cache(&scratch).load();
    assert_eq!(loaded, Loaded::default());
    assert!(loaded.cache.is_empty());
    assert!(!scratch.join("state").exists(), "nothing was made");
}

#[test]
fn a_put_keeps_the_other_entries_and_replaces_its_own() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_put_keeps_the_other_entries_and_replaces_its_own",
    );
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_file_not_understood_whole_is_set_aside_and_read_as_empty",
    );
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
            "is not one a live runtime could have registered: it has 65 tools, and an extension \
             registers at most 64",
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
    // Each check a live registration meets, met again by an entry read.
    let one = |entry: String| file_of(&[(key.clone(), entry)]);
    let fine_tool = tool_json("find", "", "{}");
    let fine_command = command_json("go", "");
    let fine_hook = hook_json("tool.before", "", "null");
    let long = "d".repeat(2049);
    let checks: Vec<(&str, String, &str)> = vec![
        (
            "a tool's description over 2 KiB",
            one(entry_of(&tool_json("find", &long, "{}"), "", "")),
            "its tool `find`: a description is at most 2 KiB",
        ),
        (
            "a tool's description with an escape character",
            one(entry_of(&tool_json("find", "a\\u001bb", "{}"), "", "")),
            "its tool `find`: a description holds a character that acts unseen",
        ),
        (
            "a tool's schema that is not an object",
            one(entry_of(&tool_json("find", "", "[]"), "", "")),
            "its tool `find`: `input` must be a JSON Schema object of at most 16 KiB",
        ),
        (
            "a tool's schema over 16 KiB",
            one(entry_of(
                &tool_json(
                    "find",
                    "",
                    &format!(r#"{{"description": "{}"}}"#, "s".repeat(16 * 1024)),
                ),
                "",
                "",
            )),
            "its tool `find`: `input` must be a JSON Schema object of at most 16 KiB",
        ),
        (
            "a tool's schema with a key that acts unseen",
            one(entry_of(
                &tool_json("find", "", r#"{"properties": {"a\u202eb": {}}}"#),
                "",
                "",
            )),
            "its tool `find`: `input` holds a character that acts unseen",
        ),
        (
            "a tool's name that is not a name",
            one(entry_of(&tool_json("find issue", "", "{}"), "", "")),
            "`find issue` cannot be a name",
        ),
        (
            "a tool's name twice",
            one(entry_of(&format!("{fine_tool}, {fine_tool}"), "", "")),
            "a tool `find` is registered twice",
        ),
        (
            "a command's name that is not a name",
            one(entry_of("", &command_json("go/now", ""), "")),
            "its command `go/now`: `go/now` cannot be a name",
        ),
        (
            "a command's description over 2 KiB",
            one(entry_of(
                "",
                &format!(r#"{{"name": "go", "description": "{long}", "arguments": []}}"#),
                "",
            )),
            "its command `go`: a description is at most 2 KiB",
        ),
        (
            "a command of 17 arguments",
            one(entry_of("", &command_json("go", &texts("a", 17)), "")),
            "its command `go`: `arguments` lists at most 16, and this lists 17",
        ),
        (
            "a command's argument that is not a name",
            one(entry_of("", &command_json("go", r#""a b""#), "")),
            "its command `go`: `a b` cannot be a name",
        ),
        (
            "a command's name twice",
            one(entry_of("", &format!("{fine_command}, {fine_command}"), "")),
            "a command `go` is registered twice",
        ),
        (
            "a hook on what is not an event",
            one(entry_of("", "", &hook_json("tool.during", "", "null"))),
            "its hook on `tool.during`: `tool.during` is not an event",
        ),
        (
            "a hook whose filter lists 65 tools",
            one(entry_of(
                "",
                "",
                &hook_json("tool.before", &texts("t", 65), "null"),
            )),
            "its hook on `tool.before`: `tools` lists at most 64, and this lists 65",
        ),
        (
            "a hook's filter naming what is not a tool",
            one(entry_of(
                "",
                "",
                &hook_json("tool.before", r#""a b""#, "null"),
            )),
            "its hook on `tool.before`: `a b` cannot be a tool in a hook's filter",
        ),
        (
            "a hook's filter entry over 129 bytes",
            one(entry_of(
                "",
                "",
                &hook_json("tool.before", &format!(r#""{}""#, "t".repeat(130)), "null"),
            )),
            "cannot be a tool in a hook's filter: 1 to 129",
        ),
        (
            "a hook's time of nothing",
            one(entry_of("", "", &hook_json("tool.before", "", "0"))),
            "its hook on `tool.before`: a hook's `timeout` is from 1 to 600000 (milliseconds)",
        ),
        (
            "a hook's time over ten minutes",
            one(entry_of("", "", &hook_json("tool.before", "", "600001"))),
            "its hook on `tool.before`: a hook's `timeout` is from 1 to 600000 (milliseconds)",
        ),
        (
            "an entry with 33 hooks",
            one(entry_of("", "", &[fine_hook.as_str(); 33].join(", "))),
            "it has 33 hooks, and an extension registers at most 32",
        ),
        (
            "an entry whose name is not an extension's",
            file_of(&[(
                key.clone(),
                r#"{"name": ".hidden", "api": 1, "tools": [], "commands": [], "hooks": []}"#
                    .to_owned(),
            )]),
            "`.hidden` is not an extension's name",
        ),
    ];
    // Each of them alone is fine.
    for fine in [
        one(entry_of(&fine_tool, &fine_command, &fine_hook)),
        one(entry_of(
            &tool_json("find", &"d".repeat(2048), "{}"),
            &command_json("go", &texts("a", 16)),
            &hook_json("tool.before", &texts("t", 64), "600000"),
        )),
    ] {
        scratch.file(file(&scratch), &fine);
        let loaded = cache.load();
        assert_eq!((loaded.cache.len(), loaded.set_aside), (1, None), "{fine}");
    }
    let cases = cases.into_iter().chain(checks);
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_file_larger_than_16_mib_is_set_aside_unread",
    );
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_put_over_a_file_not_understood_sets_it_aside_and_begins_a_new_one",
    );
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_prune_takes_out_the_hashes_no_pin_names_and_writes_only_when_it_did",
    );
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
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "every_write_is_owner_only_and_leaves_no_temporary_file",
    );
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

/// A link at the file's name, or a folder, is moved aside as it is (the
/// link, never what it leads to), so that the cache can be written again:
/// left there, no write could replace it, and every session would register
/// every approved extension again.
#[test]
fn a_link_or_a_folder_at_the_files_name_is_set_aside_and_never_followed() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_link_or_a_folder_at_the_files_name_is_set_aside_and_never_followed",
    );
    let cache = cache(&scratch);
    let aside = cache.set_aside_path();
    let target = scratch.file("elsewhere.json", "kept");
    scratch.dir("state/extensions");
    if !make_link(&target, &file(&scratch), false) {
        return;
    }
    let reason = cache.load().set_aside.unwrap();
    assert!(
        reason.contains("it could not be read: it is a link"),
        "{reason}"
    );
    assert!(
        reason.ends_with(&format!(
            " It was moved to `{}` as it is (a link is moved, and what it leads to is left \
             alone), so that the next write makes the file anew.",
            aside.display()
        )),
        "{reason}"
    );
    assert!(
        std::fs::symlink_metadata(&aside)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!file(&scratch).exists());
    assert_eq!(scratch.read_text("elsewhere.json"), "kept");
    // The next write makes the file anew, owner-only, and reads back.
    cache.put(&hash("one"), entry("one")).unwrap();
    assert!(std::fs::symlink_metadata(file(&scratch)).unwrap().is_file());
    assert!(private::is_private(&file(&scratch)).unwrap());
    assert_eq!(cache.load().cache.get(&hash("one")), Some(&entry("one")));
    assert_eq!(scratch.read_text("elsewhere.json"), "kept");

    // A folder at the name, met by a put: moved aside, and the put goes on.
    std::fs::remove_file(file(&scratch)).unwrap();
    scratch.file("state/extensions/registered.json/inside", "the user's");
    cache.put(&hash("two"), entry("two")).unwrap();
    assert_eq!(scratch.read_text(aside.join("inside")), "the user's");
    assert_eq!(cache.load().cache.get(&hash("two")), Some(&entry("two")));
}

/// A cache another session writes between the look that says the file is
/// still the one read and its move aside is moved, seen to be another file,
/// and put back.
#[test]
fn a_cache_written_while_one_is_set_aside_is_put_back() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_cache_written_while_one_is_set_aside_is_put_back",
    );
    let cache = cache(&scratch);
    // What another session's put writes, made beside it.
    let other = RegistrationCache::new(scratch.join("other/registered.json"));
    other.put(&hash("fresh"), entry("fresh")).unwrap();
    let fresh = scratch.read(scratch.join("other/registered.json"));
    scratch.file(file(&scratch), r#"{"version": 1, "entries": {"#);

    let hold = cineris_testkit::Hold::new(BEFORE_IT_IS_SET_ASIDE);
    let reader = {
        let cache = cache.clone();
        std::thread::spawn(move || cache.load())
    };
    hold.reach();
    // Another session's write: a new file renamed over the name.
    let written = scratch.file("state/extensions/next.tmp", &fresh);
    std::fs::rename(&written, file(&scratch)).unwrap();
    hold.let_one_go();
    let loaded = reader.join().unwrap();
    let reason = loaded.set_aside.unwrap();
    assert!(
        reason.ends_with(" It was left as it is: another session had just written it again."),
        "{reason}"
    );
    assert_eq!(
        cache.load().cache.get(&hash("fresh")),
        Some(&entry("fresh"))
    );
    assert!(!cache.set_aside_path().exists());
}

/// A link made at the name after the file was read and before it is
/// written is not written through: the write refuses it.
#[test]
fn a_link_made_just_before_the_write_is_never_written_through() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_link_made_just_before_the_write_is_never_written_through",
    );
    let cache = cache(&scratch);
    let target = scratch.file("elsewhere.json", "kept");
    scratch.dir("state/extensions");
    let hold = cineris_testkit::Hold::new(BEFORE_IT_IS_WRITTEN);
    let writer = {
        let cache = cache.clone();
        std::thread::spawn(move || cache.put(&hash("one"), entry("one")))
    };
    hold.reach();
    let made = make_link(&target, &file(&scratch), false);
    hold.let_one_go();
    let written = writer.join().unwrap();
    if !made {
        return;
    }
    let error = written.unwrap_err();
    assert!(
        matches!(&error, CacheError::Write { .. }) && error.to_string().contains("is a link"),
        "{error}"
    );
    assert_eq!(scratch.read_text("elsewhere.json"), "kept");
}

/// The engine's own checks are the cache's: an entry is used only for the
/// extension it fits, its hooks among those the manifest allows.
#[test]
fn an_entry_fits_only_the_extension_whose_manifest_and_prompt_it_has() {
    let snapshot = snapshot(&[
        (
            "extension.toml",
            "api = 1\nname = \"tracker\"\nversion = \"0.1.0\"\nmain = \"main.js\"\nprompt = \
             \"prompt.md\"\n\n[capabilities]\nhooks = [\"tool.before\"]\n",
        ),
        ("main.js", "1"),
        ("prompt.md", "Use tracker's tools for issues."),
    ]);
    let manifest = Manifest::of(&snapshot).unwrap();
    let fits = |entry: &Entry| entry.fits(&manifest, &snapshot);
    let tracker = entry("tracker");
    assert_eq!(fits(&tracker), Ok(()));

    let mut other = tracker.clone();
    other.name = "trackr".to_owned();
    assert_eq!(
        fits(&other),
        Err("the entry is of `trackr`, and the extension is `tracker`".to_owned())
    );
    let mut newer = tracker.clone();
    newer.api = 2;
    assert_eq!(
        fits(&newer),
        Err("the entry is of `api = 2`, and this build knows 1".to_owned())
    );
    let mut sees_more = tracker.clone();
    sees_more.registration.hooks[0].event = "prompt.submit".to_owned();
    let refused = fits(&sees_more).unwrap_err();
    assert!(
        refused.starts_with(
            "the extension's manifest does not allow a hook on `prompt.submit`, which lets it see"
        ),
        "{refused}"
    );
    for prompt in [None, Some("Another text.".to_owned())] {
        let mut changed = tracker.clone();
        changed.prompt = prompt;
        assert_eq!(
            fits(&changed),
            Err("the entry's prompt is not the text of the extension's prompt file".to_owned())
        );
    }
}

#[test]
fn a_put_that_would_make_a_file_set_aside_is_refused_and_writes_nothing() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_put_that_would_make_a_file_set_aside_is_refused_and_writes_nothing",
    );
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

    // An entry within every cap cannot make the file larger than 16 MiB
    // alone: the write's cap is held by `cache::tests`, at a smaller cap.
}

/// A snapshot of an extension made of these files.
fn snapshot(files: &[(&str, &str)]) -> std::sync::Arc<crate::folder::Snapshot> {
    let files: std::collections::BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.as_bytes().to_vec()))
        .collect();
    std::sync::Arc::new(crate::folder::Snapshot::from_files(files))
}
