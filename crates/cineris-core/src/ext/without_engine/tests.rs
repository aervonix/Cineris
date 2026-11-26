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
        pins.pin(&definition(one)).unwrap();
    }
    let elsewhere = |name: &str| Definition {
        kind: TrustResourceKind::Extension,
        name: name.to_owned(),
        hash: ContentHash::of(name.as_bytes()),
    };
    pins.pin(&elsewhere("changed")).unwrap();
    pins.pin(&elsewhere("gone")).unwrap();
    pins.pin(&elsewhere("vanished")).unwrap();
    let user_pins = pins.pins().unwrap();
    let theirs = found(Source::Project, "theirs", &["prompt.submit"]);
    let taken = found(Source::Project, "Cached", &["tool.before"]);
    let key = ProjectKey::new("p_1");
    let trust_store = TrustStore::new(scratch.join("trust.toml"), Arc::clone(&clock));
    trust_store
        .trust(
            &key,
            "~/work",
            &[definition(&theirs), definition(&taken)],
            false,
        )
        .unwrap();
    let trust = trust_store.project(&key).unwrap();
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
