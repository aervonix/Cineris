//! What a build without the script engine holds of the extensions that
//! stand approved, and what `extensions/status` says of them: the decision
//! such a build puts in a session, taken here in the ordinary build.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use cineris_base::{ContentHash, FakeClock, SharedClock};
use cineris_ext::cache::{Cache, Entry, RegistrationCache};
use cineris_ext::discover::{Discovered, Found};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_ext::registration::{HookDef, Registration};
use cineris_policy::{Definition, PinStore, ProjectTrust, TrustStore};
use cineris_protocol::ProjectKey;
use cineris_protocol::methods::{ComponentState, TrustResourceKind};

use super::*;
use crate::ext::found::definition;
use crate::ext::manage::component_state;

/// An extension's folder found, its manifest letting it hook `hooks`.
fn found(source: Source, name: &str, hooks: &[&str]) -> Found {
    let listed: Vec<String> = hooks.iter().map(|hook| format!("\"{hook}\"")).collect();
    let mut files = BTreeMap::new();
    files.insert(
        "extension.toml".to_owned(),
        format!(
            "api = 1\nname = \"{name}\"\nversion = \"1\"\nmain = \"main.js\"\n\n[capabilities]\n\
             hooks = [{}]\n",
            listed.join(", ")
        )
        .into_bytes(),
    );
    files.insert("main.js".to_owned(), name.as_bytes().to_vec());
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

fn discovered(source: Source, found: Vec<Found>) -> Discovered {
    let root = match source {
        Source::User => "/config/extensions",
        Source::Project => "/work/.cineris/extensions",
    };
    Discovered {
        names: found
            .iter()
            .map(|one| (source, one.manifest.name.clone()))
            .collect(),
        found,
        roots: vec![(source, PathBuf::from(root))],
        ..Discovered::default()
    }
}

fn hook(event: &str, tools: &[&str], observe: bool) -> HookDef {
    HookDef {
        event: event.to_owned(),
        tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
        observe,
        timeout_ms: None,
    }
}

fn registration(hooks: Vec<HookDef>) -> Registration {
    Registration {
        tools: Vec::new(),
        commands: Vec::new(),
        hooks,
    }
}

/// Each held hook of `holding`: its name, its event and the tools it is on
/// (`None`: every call).
fn held(holding: &Holding) -> Vec<(&str, LoopEvent, Option<Vec<String>>)> {
    holding
        .held
        .iter()
        .map(|held| {
            (
                held.hook.as_str(),
                held.event,
                held.filter.as_ref().map(|filter| filter.entries().to_vec()),
            )
        })
        .collect()
}

fn of<'a>(holding: &'a Holding, extension: &str) -> Vec<&'a Held> {
    holding
        .held
        .iter()
        .filter(|held| held.extension == extension)
        .collect()
}

/// In a build without the script engine, an extension that stands approved
/// does not let its gates go: each gating hook it registered at its pin's
/// hash is held on its tools, failing with why and the way out, its
/// observers and the hooks that only add left out; one the cache does not
/// keep holds a gate on each gating event its approved manifest lists; one
/// changed, or gone with nothing kept of it, holds both. What does not stand
/// approved (turned off, never approved, a project's not trusted or of a
/// name of the user's) holds nothing, and the user is told once of each
/// that stands approved that it is not loaded.
#[test]
fn an_approved_extensions_gates_hold_without_the_script_engine() {
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "without_engine_held");
    let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-26T10:00:00Z").unwrap());
    let cached = found(Source::User, "cached", &["tool.before", "tool.after"]);
    let listed = found(Source::User, "listed", &["prompt.submit", "turn.end"]);
    let changed = found(Source::User, "changed", &["tool.after"]);
    let quiet = found(Source::User, "quiet", &["tool.after"]);
    let off = found(Source::User, "off", &["tool.before"]);
    let new = found(Source::User, "new", &["tool.before"]);
    let pins = PinStore::new(scratch.join("pins.toml"), Arc::clone(&clock));
    for one in [&cached, &listed, &quiet, &off] {
        cineris_policy::test_support::Person::new().pin(&pins, &definition(one));
    }
    let elsewhere = |name: &str| Definition {
        kind: TrustResourceKind::Extension,
        name: name.to_owned(),
        hash: ContentHash::of(name.as_bytes()),
    };
    cineris_policy::test_support::Person::new().pin(&pins, &elsewhere("changed"));
    cineris_policy::test_support::Person::new().pin(&pins, &elsewhere("gone"));
    cineris_policy::test_support::Person::new().pin(&pins, &elsewhere("vanished"));
    let user_pins = pins
        .pins(Some(
            &cineris_policy::test_support::Person::new().verifier(),
        ))
        .unwrap();
    let theirs = found(Source::Project, "theirs", &["prompt.submit"]);
    let taken = found(Source::Project, "Cached", &["tool.before"]);
    let key = ProjectKey::new("p_1");
    let trust_store = TrustStore::new(scratch.join("trust.toml"), Arc::clone(&clock));
    cineris_policy::test_support::Person::new().trust(
        &trust_store,
        &key,
        "~/work",
        &[definition(&theirs), definition(&taken)],
        false,
    );
    let trust = trust_store
        .project(
            &key,
            Some(&cineris_policy::test_support::Person::new().verifier()),
        )
        .unwrap();
    let file = RegistrationCache::new(scratch.join("registered.json"));
    file.put(
        &cached.snapshot.hash(),
        Entry::new(
            &cached.manifest,
            registration(vec![
                hook("tool.before", &["edit", "write"], false),
                hook("tool.before", &[], true),
                hook("tool.after", &[], false),
                hook("tool.before", &["tracker/*"], false),
            ]),
            None,
        ),
    )
    .unwrap();
    file.put(
        &quiet.snapshot.hash(),
        Entry::new(
            &quiet.manifest,
            registration(vec![hook("tool.after", &[], false)]),
            None,
        ),
    )
    .unwrap();
    let mut gone_manifest = cached.manifest.clone();
    gone_manifest.name = "gone".to_owned();
    file.put(
        &elsewhere("gone").hash,
        Entry::new(
            &gone_manifest,
            registration(vec![hook("prompt.submit", &[], false)]),
            None,
        ),
    )
    .unwrap();
    let cache: Cache = file.load().cache;
    let user = discovered(Source::User, vec![cached, listed, changed, quiet, off, new]);
    let project = discovered(Source::Project, vec![theirs, taken]);
    let judge = |trusted: bool, trust: &ProjectTrust| {
        holding(&Judging {
            user: Some(&user),
            project: Some(&project),
            project_trusted: trusted,
            pins: Ok(&user_pins),
            pins_held: &[],
            trust: Ok(trust),
            for_the_run: &[],
            disabled: &["off".to_owned()],
            cache: Some(&cache),
            confined: false,
        })
    };
    let holding = judge(true, &trust);
    let every = None;
    let on = |tools: &[&str]| Some(tools.iter().map(|tool| (*tool).to_owned()).collect());
    assert_eq!(
        held(&holding),
        [
            // From the cache, each on its tools: the observer and the hook
            // that only adds are not gates, and count in the numbering.
            (
                "cached/hook.tool.before",
                LoopEvent::ToolBefore,
                on(&["edit", "write"])
            ),
            (
                "cached/hook.tool.before.3",
                LoopEvent::ToolBefore,
                on(&["tracker/*"])
            ),
            // Not in the cache: its approved manifest's gating events.
            (
                "listed/hook.prompt.submit",
                LoopEvent::PromptSubmit,
                every.clone()
            ),
            // Changed, nothing kept of what was approved: both.
            (
                "changed/hook.prompt.submit",
                LoopEvent::PromptSubmit,
                every.clone()
            ),
            (
                "changed/hook.tool.before",
                LoopEvent::ToolBefore,
                every.clone()
            ),
            // Gone: what the cache keeps at its pin, its tools none, so
            // every one.
            ("gone/hook.prompt.submit", LoopEvent::PromptSubmit, on(&[])),
            // Gone, and nothing kept: both.
            (
                "vanished/hook.prompt.submit",
                LoopEvent::PromptSubmit,
                every.clone()
            ),
            (
                "vanished/hook.tool.before",
                LoopEvent::ToolBefore,
                every.clone()
            ),
            // A trusted project's, pinned.
            ("theirs/hook.prompt.submit", LoopEvent::PromptSubmit, every),
        ]
    );
    for held in &holding.held {
        assert!(held.why.contains(NO_SCRIPT_ENGINE), "{}", held.why);
    }
    let [mine, ..] = of(&holding, "cached")[..] else {
        panic!("{:?}", holding.held);
    };
    assert!(
        mine.why.contains("`cineris ext disable cached`")
            && mine.why.contains("`[extensions.cached] enabled = false`"),
        "{}",
        mine.why
    );
    let [project_s] = of(&holding, "theirs")[..] else {
        panic!("{:?}", holding.held);
    };
    assert!(
        project_s.why.contains("`cineris permissions untrust`"),
        "{}",
        project_s.why
    );
    // Told once of each that stands approved, those that hold nothing too.
    let told_of = |name: &str| -> Vec<&String> {
        holding
            .told
            .iter()
            .filter(|told| told.contains(&format!("`{name}`")))
            .collect()
    };
    assert_eq!(holding.told.len(), 7, "{:#?}", holding.told);
    let [quiet_told] = told_of("quiet")[..] else {
        panic!("{:#?}", holding.told);
    };
    assert!(quiet_told.ends_with(&format!("is not loaded: {NO_SCRIPT_ENGINE}.")));
    assert!(of(&holding, "quiet").is_empty(), "it registered no gate");
    assert!(
        told_of("cached")[0].contains("Its gate on `tool.before` holds"),
        "{:?}",
        told_of("cached")
    );
    assert!(told_of("changed")[0].contains("has changed since you approved it"));
    assert!(told_of("changed")[0].contains("Its gates on `prompt.submit` and `tool.before` hold"));
    for name in ["off", "new", "Cached"] {
        assert!(of(&holding, name).is_empty(), "{name} holds nothing");
        assert!(told_of(name).is_empty(), "{name} is not told of");
    }
    // A project not trusted holds nothing.
    let untrusted = judge(false, &ProjectTrust::untrusted());
    assert!(of(&untrusted, "theirs").is_empty());
}

/// While the user's pin store cannot be read, each of the user's found
/// holds what the cache keeps of its name and its manifest's gating events;
/// a project's, while the trust store cannot be read, holds nothing: no
/// repository gates a session by itself.
#[test]
fn an_unread_store_holds_the_users_gates_and_no_projects() {
    let mine = found(Source::User, "mine", &["prompt.submit"]);
    let theirs = found(Source::Project, "theirs", &["prompt.submit", "tool.before"]);
    let user = discovered(Source::User, vec![mine.clone()]);
    let project = discovered(Source::Project, vec![theirs]);
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "without_engine_unread");
    let file = RegistrationCache::new(scratch.join("registered.json"));
    file.put(
        &ContentHash::of(b"an older mine"),
        Entry::new(
            &mine.manifest,
            registration(vec![hook("tool.before", &["edit"], false)]),
            None,
        ),
    )
    .unwrap();
    let cache = file.load().cache;
    let holding = holding(&Judging {
        user: Some(&user),
        project: Some(&project),
        project_trusted: true,
        pins: Err("the pin store is broken".to_owned()),
        pins_held: &[],
        trust: Err("the trust store is broken".to_owned()),
        for_the_run: &[],
        disabled: &[],
        cache: Some(&cache),
        confined: false,
    });
    assert_eq!(
        held(&holding),
        [
            (
                "mine/hook.tool.before",
                LoopEvent::ToolBefore,
                Some(vec!["edit".to_owned()])
            ),
            ("mine/hook.prompt.submit", LoopEvent::PromptSubmit, None),
        ]
    );
    let [told] = &holding.told[..] else {
        panic!("{:#?}", holding.told);
    };
    assert!(
        told.contains("cannot be judged (the pin store is broken)"),
        "{told}"
    );
}

/// While the user's pins do not count (no key, unsigned, or not the
/// person's), what they pin restricts all the same: one gone holds both
/// gates, as it does while they count, and one left out holds what the
/// cache keeps at the hash pinned (the review of the permission path: one
/// gone held nothing). With no pin held, nothing is.
#[test]
fn a_pinned_extension_gone_holds_its_gates_while_the_pins_do_not_count() {
    let user = discovered(Source::User, Vec::new());
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "without_engine_gone");
    let file = RegistrationCache::new(scratch.join("registered.json"));
    let gone = found(Source::User, "gone", &["tool.before"]);
    file.put(
        &gone.snapshot.hash(),
        Entry::new(
            &gone.manifest,
            registration(vec![hook("tool.before", &["edit"], false)]),
            None,
        ),
    )
    .unwrap();
    let cache = file.load().cache;
    let pin = |name: &str, hash: ContentHash| cineris_policy::UserPin {
        kind: TrustResourceKind::Extension,
        name: name.to_owned(),
        hash,
        registration: None,
        at: cineris_protocol::ids::Timestamp::new("2026-11-27T10:00:00Z"),
    };
    let held_pins = [
        pin("gone", gone.snapshot.hash()),
        pin("vanished", ContentHash::of(b"not cached")),
    ];
    let judge = |pins_held: &[cineris_policy::UserPin]| {
        holding(&Judging {
            user: Some(&user),
            project: None,
            project_trusted: false,
            pins: Err("your approvals are not open in this session".to_owned()),
            pins_held,
            trust: Ok(&ProjectTrust::untrusted()),
            for_the_run: &[],
            disabled: &[],
            cache: Some(&cache),
            confined: false,
        })
    };
    let holding = judge(&held_pins);
    assert_eq!(
        held(&holding),
        [
            (
                "gone/hook.tool.before",
                LoopEvent::ToolBefore,
                Some(vec!["edit".to_owned()])
            ),
            ("vanished/hook.prompt.submit", LoopEvent::PromptSubmit, None),
            ("vanished/hook.tool.before", LoopEvent::ToolBefore, None),
        ]
    );
    assert!(
        holding
            .told
            .iter()
            .all(|told| told.contains("cannot be found where you approved it")),
        "{:#?}",
        holding.told
    );
    assert!(judge(&[]).held.is_empty());
}

/// A build without the script engine holds the gates of what a project's
/// entry that does not count pins (`uncounted_gates`): from what the cache
/// keeps at the hash pinned, on its tools, or both gates on every call where
/// it keeps nothing, each saying why. So under `--trust-project` (the
/// project looked in, and its extension found and not approved) as in a
/// project not trusted (the review of the permission path).
#[test]
fn a_session_without_the_engine_holds_what_an_uncounted_entry_pins() {
    let scratch =
        cineris_testkit::Scratch::outside("cineris-core-unit", "without_engine_uncounted");
    let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-27T10:00:00Z").unwrap());
    let workspace = scratch.dir("work");
    scratch.file(
        "work/.cineris/extensions/theirs/extension.toml",
        manifest_file("theirs", "\"tool.before\""),
    );
    scratch.file("work/.cineris/extensions/theirs/main.js", "theirs");
    let resolver = Arc::new(
        cineris_base::PathResolver::new(&workspace, cineris_base::path::PathOptions::new())
            .unwrap(),
    );
    let state = cineris_config::StateDir::new(scratch.dir("state"));
    let theirs = crate::ext::found::in_project(&resolver, &clock).0;
    let [theirs] = &theirs.found[..] else {
        panic!("{:?}", theirs.found);
    };
    RegistrationCache::new(state.registration_cache_file())
        .put(
            &theirs.snapshot.hash(),
            Entry::new(
                &theirs.manifest,
                registration(vec![hook("tool.before", &["list"], false)]),
                None,
            ),
        )
        .unwrap();
    let uncounted = crate::ext::found::Uncounted {
        checked: cineris_policy::Checked::NoKey,
        extensions: vec![
            ("theirs".to_owned(), theirs.snapshot.hash()),
            ("gone".to_owned(), ContentHash::of(b"not cached")),
        ],
    };
    for project in [None, Some(ProjectKey::new("p_1"))] {
        let view = SessionView {
            resolver: Arc::clone(&resolver),
            config_dir: None,
            state: Some(state.clone()),
            project: project.clone(),
            verifier: None,
            uncounted: Some(uncounted.clone()),
            for_the_run: Vec::new(),
            disabled: Vec::new(),
        };
        let holding = in_session(&view, &clock, None);
        assert_eq!(
            held(&holding),
            [
                (
                    "theirs/hook.tool.before",
                    LoopEvent::ToolBefore,
                    Some(vec!["list".to_owned()])
                ),
                ("gone/hook.prompt.submit", LoopEvent::PromptSubmit, None),
                ("gone/hook.tool.before", LoopEvent::ToolBefore, None),
            ],
            "{project:?}"
        );
        for gate in &holding.held {
            assert!(
                gate.why.contains(
                    "this project's trust does not count in this session (your approvals are \
                     not open"
                ),
                "{project:?}: {}",
                gate.why
            );
        }
        // With no entry that does not count, nothing is held.
        let view = SessionView {
            uncounted: None,
            ..view
        };
        assert!(
            in_session(&view, &clock, None).held.is_empty(),
            "{project:?}"
        );
    }
}

/// The manifest of an extension `name` that may hook `hooks`, as a file.
fn manifest_file(name: &str, hooks: &str) -> String {
    format!(
        "api = 1\nname = \"{name}\"\nversion = \"1\"\nmain = \"main.js\"\n\n[capabilities]\n\
         hooks = [{hooks}]\n"
    )
}

/// What a session without the script engine holds and tells, judged from
/// its view as it opened (`in_session`), the stores and folders read from
/// disk as the session reads them: the user's extension pinned, which holds
/// what the cache keeps it registered, on its tools; and a trusted
/// project's approved for this session by a yes the trust store could not
/// keep, while that store cannot be read, which holds a gate on each gating
/// event its manifest lists, as the shipped build would load it and run its
/// gates (the review of step 28's close, L1). The user is told once of
/// each: two lines, each naming its extension and the gate it holds.
#[test]
fn a_session_without_the_engine_holds_what_its_view_finds_and_tells_the_user_once() {
    let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "without_engine_view");
    let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-26T10:00:00Z").unwrap());
    let config = scratch.dir("config");
    scratch.file(
        "config/extensions/tracker/extension.toml",
        manifest_file("tracker", "\"tool.before\""),
    );
    scratch.file("config/extensions/tracker/main.js", "tracker");
    let workspace = scratch.dir("work");
    scratch.file(
        "work/.cineris/extensions/theirs/extension.toml",
        manifest_file("theirs", "\"prompt.submit\", \"tool.after\""),
    );
    scratch.file("work/.cineris/extensions/theirs/main.js", "theirs");
    let resolver = Arc::new(
        cineris_base::PathResolver::new(&workspace, cineris_base::path::PathOptions::new())
            .unwrap(),
    );
    let state = cineris_config::StateDir::new(scratch.dir("state"));
    let mine =
        cineris_ext::discover::discover(&[cineris_ext::discover::Root::user(&config)], &resolver);
    let theirs = crate::ext::found::in_project(&resolver, &clock).0;
    let ([mine], [theirs]) = (&mine.found[..], &theirs.found[..]) else {
        panic!("{:?} {:?}", mine.found, theirs.found);
    };
    cineris_policy::test_support::Person::new().pin(
        &PinStore::new(state.pins_file(), Arc::clone(&clock)),
        &definition(mine),
    );
    RegistrationCache::new(state.registration_cache_file())
        .put(
            &mine.snapshot.hash(),
            Entry::new(
                &mine.manifest,
                registration(vec![hook("tool.before", &["list"], false)]),
                None,
            ),
        )
        .unwrap();
    // A trust store that cannot be read.
    scratch.file("state/trust.toml", "this is not [a trust store");
    let view = SessionView {
        resolver,
        config_dir: Some(config),
        state: Some(state),
        project: Some(ProjectKey::new("p_1")),
        verifier: Some(cineris_policy::test_support::Person::new().verifier()),
        uncounted: None,
        for_the_run: vec![definition(theirs)],
        disabled: Vec::new(),
    };
    let holding = in_session(&view, &clock, None);
    assert_eq!(
        held(&holding),
        [
            (
                "tracker/hook.tool.before",
                LoopEvent::ToolBefore,
                Some(vec!["list".to_owned()])
            ),
            ("theirs/hook.prompt.submit", LoopEvent::PromptSubmit, None),
        ]
    );
    let [told_mine, told_theirs] = &holding.told[..] else {
        panic!("{:#?}", holding.told);
    };
    assert!(
        told_mine.starts_with("Your extension `tracker`")
            && told_mine.contains("Its gate on `tool.before` holds"),
        "{told_mine}"
    );
    assert!(
        told_theirs.starts_with("This project's extension `theirs`")
            && told_theirs.contains("Its gate on `prompt.submit` holds")
            && !told_theirs.contains("cannot be judged"),
        "{told_theirs}"
    );
}

/// A trusted project's extension approved for this session, by a yes the
/// trust store could not keep, stands approved while that store cannot be
/// read, its pin the session's: the session's approval is looked at before
/// the store's error, as the session's finding looks at it, so that
/// `extensions/status` does not say "cannot be judged" of one the session
/// loaded (the review of step 28's close, L1). One not approved for the
/// session still cannot be judged.
#[test]
fn an_extension_approved_for_the_session_stands_approved_while_the_trust_store_cannot_be_read() {
    let approved = found(Source::Project, "theirs", &["prompt.submit"]);
    let other = found(Source::Project, "other", &["prompt.submit"]);
    let project = discovered(Source::Project, vec![approved.clone(), other]);
    let for_the_run = [definition(&approved)];
    let listed = judged(&Judging {
        user: None,
        project: Some(&project),
        project_trusted: true,
        pins: Ok(&cineris_policy::UserPins::default()),
        pins_held: &[],
        trust: Err("the trust store is broken".to_owned()),
        for_the_run: &for_the_run,
        disabled: &[],
        cache: None,
        confined: false,
    });
    let standing = |name: &str| {
        listed
            .iter()
            .find(|one| one.name == name)
            .map(|one| (one.standing.clone(), one.pin.clone()))
            .unwrap()
    };
    assert_eq!(
        standing("theirs"),
        (
            ExtensionStanding::Approved,
            Some((approved.snapshot.hash(), "for this session".to_owned()))
        )
    );
    assert_eq!(
        standing("other"),
        (
            ExtensionStanding::Unjudged("the trust store is broken".to_owned()),
            None
        )
    );
}

/// `extensions/status` in a build without the script engine: an extension
/// approved as it is found is not `ready`, since nothing of it is loaded,
/// and says why; what does not stand approved says what it said.
#[test]
fn status_says_an_approved_extension_is_not_loaded_without_the_engine() {
    let (state, error) = component_state(
        &ExtensionStanding::Approved,
        None,
        not_loaded(&ExtensionStanding::Approved),
    );
    assert_eq!(state, ComponentState::Failed);
    assert_eq!(
        error.as_deref(),
        Some("approved, and not loaded: this build of Cineris has no script engine, which runs it")
    );
    for standing in [
        ExtensionStanding::Changed,
        ExtensionStanding::NotApproved,
        ExtensionStanding::TurnedOff,
        ExtensionStanding::Gone,
        ExtensionStanding::ProjectNotTrusted,
    ] {
        assert_eq!(not_loaded(&standing), None, "{standing:?}");
    }
    // With the engine, the same extension is ready.
    assert_eq!(
        component_state(&ExtensionStanding::Approved, None, None).0,
        ComponentState::Ready
    );
}
