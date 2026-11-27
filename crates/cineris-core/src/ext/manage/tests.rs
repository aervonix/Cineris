//! The management of extensions: how each stands, the names' rules, and
//! the folder's write and delete, which follow no link.

use std::collections::BTreeMap;
use std::sync::Arc;

use cineris_base::{ContentHash, FakeClock, SharedClock};
use cineris_ext::cache::{Cache, Entry};
use cineris_ext::discover::{Discovered, Found, LeftOut};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::{HookDef, Registration, ToolDef};
use cineris_ext::source::Source;
use cineris_policy::{Definition, PinStore, ProjectTrust, TrustStore, UserPins};
use cineris_protocol::ProjectKey;
use cineris_protocol::methods::TrustResourceKind;

use super::*;

fn found(source: Source, name: &str, body: &str) -> Found {
    let mut files = BTreeMap::new();
    files.insert(
        "extension.toml".to_owned(),
        format!("api = 1\nname = \"{name}\"\nversion = \"1\"\nmain = \"main.js\"\n").into_bytes(),
    );
    files.insert("main.js".to_owned(), body.as_bytes().to_vec());
    let snapshot = Snapshot::from_files(files);
    let manifest = Manifest::of(&snapshot).unwrap();
    let root = match source {
        Source::User => "/config/extensions",
        Source::Project => "/work/.cineris/extensions",
    };
    Found {
        source,
        folder: PathBuf::from(root).join(name),
        shown: format!("{root}/{name}"),
        snapshot: Arc::new(snapshot),
        manifest,
    }
}

fn discovered(source: Source, found: Vec<Found>, left: &[&str]) -> Discovered {
    let root = match source {
        Source::User => "/config/extensions",
        Source::Project => "/work/.cineris/extensions",
    };
    Discovered {
        names: found
            .iter()
            .map(|one| (source, one.manifest.name.clone()))
            .chain(left.iter().map(|name| (source, (*name).to_owned())))
            .collect(),
        left_out: left
            .iter()
            .map(|name| LeftOut {
                source,
                folder: PathBuf::from(root).join(name),
                shown: format!("{root}/{name}"),
                reason: format!("the extension folder `{name}` was left out: it holds a link"),
            })
            .chain([LeftOut {
                source,
                folder: PathBuf::from(root),
                shown: root.to_owned(),
                reason: "a note of the root's own".to_owned(),
            }])
            .collect(),
        found,
        roots: vec![(source, PathBuf::from(root))],
        ..Discovered::default()
    }
}

fn standing_of<'a>(
    listed: &'a [ExtensionListed],
    source: Source,
    name: &str,
) -> &'a ExtensionListed {
    listed
        .iter()
        .find(|one| one.source == source && one.name == name)
        .unwrap_or_else(|| panic!("{name} is not listed: {listed:#?}"))
}

/// Each row of the table of how a found extension stands is a listing's,
/// as a session judges it: the user's by `pins.toml` and their own word, a
/// project's by its trust, a project's of a name the user's folder takes not
/// used, a store that cannot be read unjudged, a pin whose folder is gone,
/// a folder left out; and what each registered from the cache, at the hash
/// found or else at its pin's.
#[test]
fn each_extension_stands_as_a_session_would_judge_it() {
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "manage_judged");
    let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-25T10:00:00Z").unwrap());
    let approved = found(Source::User, "approved", "a");
    let changed = found(Source::User, "changed", "b");
    let off = found(Source::User, "off", "c");
    let new = found(Source::User, "new", "d");
    let pins = PinStore::new(scratch.join("pins.toml"), Arc::clone(&clock));
    pins.pin(&definition(&approved)).unwrap();
    pins.pin(&Definition {
        kind: TrustResourceKind::Extension,
        name: "changed".to_owned(),
        hash: ContentHash::of(b"another"),
    })
    .unwrap();
    let gone_hash = ContentHash::of(b"gone");
    pins.pin(&Definition {
        kind: TrustResourceKind::Extension,
        name: "gone".to_owned(),
        hash: gone_hash,
    })
    .unwrap();
    let user_pins = pins.pins().unwrap();
    let theirs = found(Source::Project, "theirs", "e");
    let taken = found(Source::Project, "APPROVED", "f");
    let unpinned = found(Source::Project, "unpinned", "g");
    let key = ProjectKey::new("p_1");
    let trust_store = TrustStore::new(scratch.join("trust.toml"), Arc::clone(&clock));
    trust_store
        .trust(&key, "~/work", &[definition(&theirs)], false)
        .unwrap();
    let trust = trust_store.project(&key).unwrap();
    let user = discovered(
        Source::User,
        vec![approved.clone(), changed, off, new],
        &["linked"],
    );
    let project = discovered(Source::Project, vec![theirs, taken, unpinned], &[]);
    let file = cineris_ext::cache::RegistrationCache::new(scratch.join("registered.json"));
    let registration = Registration {
        tools: vec![ToolDef {
            name: "find".to_owned(),
            description: String::new(),
            input: serde_json::json!({"type": "object"}),
            read_only: true,
        }],
        commands: Vec::new(),
        hooks: vec![HookDef {
            event: "tool.before".to_owned(),
            tools: vec!["edit".to_owned()],
            observe: false,
            timeout_ms: None,
        }],
    };
    file.put(
        &approved.snapshot.hash(),
        Entry::new(&approved.manifest, registration.clone(), None),
    )
    .unwrap();
    let mut gone_manifest = approved.manifest.clone();
    gone_manifest.name = "gone".to_owned();
    file.put(&gone_hash, Entry::new(&gone_manifest, registration, None))
        .unwrap();
    let cache: Cache = file.load().cache;
    let judge =
        |pins: Result<&UserPins, String>, trust: Result<&ProjectTrust, String>, trusted: bool| {
            judged(&Judging {
                user: Some(&user),
                project: Some(&project),
                project_trusted: trusted,
                pins,
                trust,
                for_the_run: &[],
                disabled: &["off".to_owned()],
                cache: Some(&cache),
                confined: false,
            })
        };
    let listed = judge(Ok(&user_pins), Ok(&trust), true);
    let at = |source, name| standing_of(&listed, source, name).standing.clone();
    assert_eq!(at(Source::User, "approved"), ExtensionStanding::Approved);
    assert_eq!(at(Source::User, "changed"), ExtensionStanding::Changed);
    assert_eq!(at(Source::User, "off"), ExtensionStanding::TurnedOff);
    assert_eq!(at(Source::User, "new"), ExtensionStanding::NotApproved);
    assert_eq!(at(Source::User, "gone"), ExtensionStanding::Gone);
    assert!(
        matches!(at(Source::User, "linked"), ExtensionStanding::LeftOut(why) if why.contains("holds a link"))
    );
    assert_eq!(at(Source::Project, "theirs"), ExtensionStanding::Approved);
    assert!(matches!(
        at(Source::Project, "APPROVED"),
        ExtensionStanding::NotUsed(_)
    ));
    assert_eq!(
        at(Source::Project, "unpinned"),
        ExtensionStanding::NotApproved
    );
    assert!(
        !listed.iter().any(|one| one.name == "extensions"),
        "a root's own note is no extension"
    );
    let shown = standing_of(&listed, Source::User, "approved");
    assert_eq!(
        shown.registered.as_ref().unwrap().hooks,
        ["tool.before on edit"]
    );
    assert!(shown.pin.is_some());
    assert!(
        shown
            .words
            .as_deref()
            .unwrap()
            .contains("It runs no program.")
    );
    let gone = standing_of(&listed, Source::User, "gone");
    assert_eq!(gone.folder, None);
    assert_eq!(
        gone.registered.as_ref().unwrap().tools,
        ["find"],
        "from the pin's hash"
    );
    // The project not trusted: nothing of it is used, and no pin of it is
    // gone.
    let untrusted = judge(Ok(&user_pins), Ok(&ProjectTrust::untrusted()), false);
    assert_eq!(
        standing_of(&untrusted, Source::Project, "theirs").standing,
        ExtensionStanding::ProjectNotTrusted
    );
    // A store that cannot be read: what it holds cannot be judged.
    let unread = judge(
        Err("the pin store is broken".to_owned()),
        Err("the trust store is broken".to_owned()),
        true,
    );
    assert!(matches!(
        &standing_of(&unread, Source::User, "approved").standing,
        ExtensionStanding::Unjudged(why) if why.contains("pin store")
    ));
    assert_eq!(
        standing_of(&unread, Source::User, "off").standing,
        ExtensionStanding::TurnedOff,
        "the user's word stands"
    );
    assert!(matches!(
        &standing_of(&unread, Source::Project, "theirs").standing,
        ExtensionStanding::Unjudged(why) if why.contains("trust store")
    ));
    assert!(!unread.iter().any(|one| one.name == "gone"));
}

/// A name is a folder of the user's only when a session would find it, the
/// same on every system.
#[test]
fn a_name_must_be_one_a_session_finds_on_every_system() {
    for fine in ["tracker", "acme.lint", "a-b_c", "CONSOLE", "com10", "lpt"] {
        assert_eq!(folder_name_refused(fine), None, "{fine}");
    }
    for refused in [
        "",
        ".",
        "..",
        ".hidden",
        "trailing.",
        "con",
        "NUL",
        "aux.x",
        "com1",
        "LPT9.txt",
        "a/b",
        "a\\b",
        "with space",
    ] {
        assert!(folder_name_refused(refused).is_some(), "{refused:?}");
    }
}

/// A removal deletes only a name directly in the user's folder of
/// extensions, as the folder holds it: a name that is not one name (`..`,
/// a path through it) is refused before anything is looked at, through
/// the core as `cineris ext remove` calls it, so nothing outside the
/// folder is deleted. The name rules are the guard (the full review of
/// step 28, its finding 18: a check of the folder above the target,
/// resolved, compared the folder with itself, since the target is always
/// built from such a name, and was taken out).
#[test]
fn a_path_that_leads_out_of_the_folder_of_extensions_is_not_deleted() {
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "manage_within");
    let root = scratch.join("config/extensions");
    std::fs::create_dir_all(root.join("tracker")).unwrap();
    std::fs::create_dir_all(scratch.join("elsewhere/x")).unwrap();
    std::fs::create_dir_all(root.join("inner/x")).unwrap();
    // The root and the target as a removal builds them, from the
    // configuration directory of a core.
    let env = cineris_config::Env::empty(cineris_config::Platform::CURRENT).with(
        if cfg!(windows) { "USERPROFILE" } else { "HOME" },
        scratch.join("home").to_str().unwrap(),
    );
    let mut dirs = cineris_config::Dirs::from_env(&env);
    dirs.config = Some(scratch.join("config"));
    dirs.managed_config = Some(scratch.join("managed.toml"));
    let mut options = crate::CoreOptions::new(
        env,
        std::sync::Arc::new(cineris_testkit::timer::FakeTimer::manual(
            &cineris_base::FakeClock::starting_at("2026-11-26T10:00:00Z").unwrap(),
        )),
    );
    options.dirs = Some(dirs);
    let core = crate::Core::start(options).unwrap();
    std::fs::create_dir_all(scratch.join("work")).unwrap();
    let project = core.project(&scratch.join("work")).unwrap();
    let removal = core.extension_removal(&project, "Tracker").unwrap();
    assert_eq!(removal.name, "tracker");
    assert_eq!(removal.target, root.join("tracker"), "{removal:?}");
    let out = scratch.join("elsewhere").join("x");
    let mut names = vec![
        "..".to_owned(),
        "../../elsewhere/x".to_owned(),
        out.to_string_lossy().into_owned(),
        "inner/x".to_owned(),
    ];
    if cfg!(windows) {
        names.push("..\\..\\elsewhere\\x".to_owned());
    }
    for name in &names {
        let refused = core.extension_removal(&project, name).unwrap_err();
        assert!(
            refused.message.contains("cannot be removed"),
            "{name}: {refused:?}"
        );
    }
    assert!(out.is_dir() && root.join("inner/x").is_dir());
    // What is removed, with its yes, is the folder at the name alone.
    let removed = core.remove_extension(&removal, true).unwrap();
    assert!(removed.deleted);
    assert!(!root.join("tracker").exists() && root.join("inner/x").is_dir());
}

/// What is deleted is never walked through a link: a link at the name is
/// unlinked, what it leads to untouched; a folder goes whole.
#[cfg(unix)]
#[test]
fn a_link_at_the_name_is_unlinked_and_not_walked() {
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "manage_unlink");
    let outside = scratch.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep"), "k").unwrap();
    let root = scratch.join("extensions");
    std::fs::create_dir_all(root.join("real/inner")).unwrap();
    std::fs::write(root.join("real/inner/f"), "f").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("real/inner/out")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
    assert!(delete_unfollowed(&root.join("linked")).unwrap());
    assert!(std::fs::symlink_metadata(root.join("linked")).is_err());
    assert!(delete_unfollowed(&root.join("real")).unwrap());
    assert!(!root.join("real").exists());
    assert_eq!(std::fs::read_to_string(outside.join("keep")).unwrap(), "k");
    assert!(
        !delete_unfollowed(&root.join("real")).unwrap(),
        "already gone"
    );
}

/// The folder is written whole in place of what was there, a link at the
/// name replaced and never written through, and nothing of the write's own
/// left beside it.
#[test]
fn the_folder_is_written_whole_in_place_of_what_was_there() {
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "manage_write");
    let root = scratch.join("extensions");
    std::fs::create_dir_all(root.join("tracker/old")).unwrap();
    std::fs::write(root.join("tracker/old/stale.js"), "x").unwrap();
    let snapshot = found(Source::User, "tracker", "new").snapshot;
    write_folder(&root.join("tracker"), &snapshot).unwrap();
    assert_eq!(
        Snapshot::read_unfollowed(&root.join("tracker"))
            .0
            .unwrap()
            .hash(),
        snapshot.hash()
    );
    let names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["tracker"]);
    #[cfg(unix)]
    {
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::remove_dir_all(root.join("tracker")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("tracker")).unwrap();
        write_folder(&root.join("tracker"), &snapshot).unwrap();
        assert!(
            !std::fs::symlink_metadata(root.join("tracker"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing written through it"
        );
    }
}
