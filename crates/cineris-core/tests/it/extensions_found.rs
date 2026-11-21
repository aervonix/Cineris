//! The extensions a session finds and loads (step 28): the user's checked
//! against their pins, a project's through its trust question, what each
//! registered taken from the registration cache, an extension the cache
//! misses registered before the first turn, and a change disabling one
//! (`docs/steps/28/answered.md`, points 4 and 5). End to end, through the
//! core, as `extensions` is: the extension's folders, the pins, the trust
//! store and the cache are real files of the test's machine.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_base::{ContentHash, PathOptions, PathResolver, SharedClock};
use cineris_config::StateDir;
use cineris_core::{ClientInfo, Core, Outgoing};
use cineris_ext::cache::{Entry, RegistrationCache};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_policy::{Definition, PinStore, Standing, TrustStore};
use cineris_protocol::ProjectKey;
use cineris_protocol::event::Event;
use cineris_protocol::methods::{
    Op, OpResult, ServerRequest, ServerResponse, SessionNewParams, TrustRequest, TrustResourceKind,
    TrustResponse,
};
use cineris_protocol::session::SessionSettings;
use cineris_testkit::{FakeClock, FakeTimer, Hold, Response, block_on};
use serde_json::json;

use crate::{Answering, Front, World, calls, poll_once, says};

/// The name of every extension here.
pub(crate) const NAME: &str = "probe";

/// A turn whose model looks for the extension's tools, then ends.
pub(crate) fn searches() -> [Response; 2] {
    [
        calls(&[("s1", "tool_search", json!({"query": "probe/"}))]),
        says("Done."),
    ]
}

/// `count` turns that each look for the extension's tools.
pub(crate) fn searches_times(count: usize) -> Vec<Response> {
    (0..count).flat_map(|_| searches()).collect()
}

/// Writes the extension `probe` in `folder`, with these capabilities and a
/// tool `echo` described as `said`, which gives its arguments back.
pub(crate) fn write_extension(folder: &Path, capabilities: &str, said: &str) {
    write_described(folder, "A test extension", capabilities, said);
}

/// [`write_extension`], with the manifest's description as TOML text.
pub(crate) fn write_described(folder: &Path, description: &str, capabilities: &str, said: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"{NAME}\"\nversion = \"1\"\ndescription = \"{description}\"\nmain = \
             \"main.js\"\n[capabilities]\n{capabilities}\n"
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

/// The user's `probe`: in the folder of extensions of the world's
/// configuration directory.
pub(crate) fn users(world: &World) -> PathBuf {
    world.dirs().config.unwrap().join("extensions").join(NAME)
}

/// The user's `probe` as the user is shown it: under `~`, with `/`.
pub(crate) fn users_shown(world: &World) -> String {
    let below = users(world)
        .strip_prefix(&world.home)
        .unwrap()
        .components()
        .map(|name| name.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    format!("~/{below}")
}

/// The project's `probe`.
pub(crate) fn projects(world: &World) -> PathBuf {
    world.workspace.join(".cineris/extensions").join(NAME)
}

pub(crate) fn state(world: &World) -> StateDir {
    let state = StateDir::from_dirs(&world.dirs()).unwrap();
    std::fs::create_dir_all(state.root()).unwrap();
    state
}

pub(crate) fn clock(world: &World) -> SharedClock {
    Arc::new(world.clock.clone())
}

/// The definition of the extension in `folder`, as it is now.
pub(crate) fn definition(folder: &Path) -> Definition {
    Definition {
        kind: TrustResourceKind::Extension,
        name: NAME.to_owned(),
        hash: Snapshot::read(folder).unwrap().hash(),
    }
}

/// Pins the user's extension in `folder` as it is now, as approving it
/// does.
pub(crate) fn pin(world: &World, folder: &Path) {
    PinStore::new(state(world).pins_file(), clock(world))
        .pin(&definition(folder))
        .unwrap();
}

/// The world's registration cache.
pub(crate) fn cache_file(world: &World) -> RegistrationCache {
    RegistrationCache::new(state(world).registration_cache_file())
}

/// Registers the extension in `folder` as approving it does, in a
/// throwaway runtime (one runtime made), and keeps what it registered in
/// the cache. Its hash.
pub(crate) fn cache(world: &World, folder: &Path) -> ContentHash {
    let snapshot = Arc::new(Snapshot::read(folder).unwrap());
    let timer = FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH));
    let (manifest, registration) =
        block_on(cineris_ext::engine::register(Arc::clone(&snapshot), &timer)).unwrap();
    cache_file(world)
        .put(&snapshot.hash(), Entry::new(&manifest, registration, None))
        .unwrap();
    snapshot.hash()
}

/// The project's key, as a session in the world's workspace has it.
pub(crate) fn project_key(world: &World) -> ProjectKey {
    PathResolver::new(&world.workspace, PathOptions::new())
        .unwrap()
        .root()
        .project_key()
}

pub(crate) fn trust_store(world: &World) -> TrustStore {
    TrustStore::new(state(world).trust_file(), clock(world))
}

/// Trusts the project with `definitions` pinned, as a yes to its question
/// pins them.
pub(crate) fn trust(world: &World, definitions: &[Definition]) {
    trust_store(world)
        .trust(&project_key(world), "~/work/app", definitions, false)
        .unwrap();
}

pub(crate) fn yes() -> TrustResponse {
    TrustResponse {
        trusted: true,
        never: false,
        keep: false,
    }
}

/// Opens a session in the world's workspace as a user's front end that
/// answers the trust question with `answer`: the front, and the questions
/// it was asked.
pub(crate) async fn open_answering(
    world: &World,
    core: &Core,
    answer: TrustResponse,
) -> (Front, Vec<TrustRequest>) {
    open_as(world, core, ClientInfo::user(), answer).await
}

/// Opens a session in the world's workspace as a user's front end that
/// answers a trust question not now: the extensions' tests' own start, a
/// session kept in memory only ([`open_doing`]).
pub(crate) async fn start(world: &World, core: &Core) -> Front {
    open_answering(world, core, TrustResponse::default())
        .await
        .0
}

/// [`open_answering`], as a client of `info`.
pub(crate) async fn open_as(
    world: &World,
    core: &Core,
    info: ClientInfo,
    answer: TrustResponse,
) -> (Front, Vec<TrustRequest>) {
    open_doing(world, core, info, &|| {}, answer).await
}

/// [`open_as`], doing `meanwhile` while each trust question is open,
/// before it is answered.
pub(crate) async fn open_doing(
    world: &World,
    core: &Core,
    info: ClientInfo,
    meanwhile: &dyn Fn(),
    answer: TrustResponse,
) -> (Front, Vec<TrustRequest>) {
    let client = core.connect(info);
    let opening = core.handle(
        client.id,
        Op::SessionNew(SessionNewParams {
            cwd: None,
            workspace: world.workspace.to_str().unwrap().to_owned(),
            settings: SessionSettings::default(),
            title: None,
            config: serde_json::Map::new(),
            // Kept in memory only: what these tests check is not the log,
            // and its writes are much of a session's cost on Windows.
            ephemeral: true,
        }),
    );
    let mut asked = Vec::new();
    let mut read = Vec::new();
    let result = {
        let answering = async {
            loop {
                match client.queue.next().await {
                    Some(Outgoing::Request {
                        id,
                        request: ServerRequest::Trust(request),
                    }) => {
                        asked.push(request);
                        meanwhile();
                        core.respond(client.id, &id, ServerResponse::Trust(answer.clone()))
                            .expect("the question is open");
                    }
                    Some(other) => read.push(other),
                    None => std::future::pending::<()>().await,
                }
            }
        };
        let mut opening = std::pin::pin!(opening);
        let mut answering = std::pin::pin!(answering);
        std::future::poll_fn(|context| {
            let _ = answering.as_mut().poll(context);
            opening.as_mut().poll(context)
        })
        .await
    };
    let Ok(OpResult::SessionNew(result)) = result else {
        panic!("the session does not open: {result:?}");
    };
    let front = Front {
        core: core.clone(),
        client,
        session: result.session.session_id.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::new(Vec::new()),
        answering: std::cell::RefCell::new(Answering::default()),
    };
    (front, asked)
}

/// Whether the session's last `tool_search` found the extension's tool.
pub(crate) fn found_probe(front: &Front) -> bool {
    let results = front.results();
    let (_, outcome, text) = results.last().expect("a tool ran");
    assert_eq!(*outcome, "success", "{results:?}");
    text.contains("probe/echo")
}

/// The notices that name the extension `probe`.
pub(crate) fn about_probe(front: &Front) -> Vec<String> {
    front
        .notices()
        .into_iter()
        .filter(|notice| notice.contains("`probe`"))
        .collect()
}

/// Makes `link`, a link to the folder `target`. `false` when this machine
/// cannot (Windows without the right to), where the test then passes
/// having checked nothing; a CI runner can, and must.
#[expect(
    clippy::print_stdout,
    reason = "a test that checks nothing on this machine says so, as the test runner shows it"
)]
pub(crate) fn make_link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(target, link);
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            println!("skipped: cannot make a link here ({error})");
            false
        }
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

// ------------------------------------------------------------ the user's own

/// A user's extension stands by its pin, a session after another in one
/// world and one core (folded for Windows' tier 2):
/// - never pinned, it is not loaded, and the session says so with the
///   command that approves it, its folder under `~`;
/// - pinned, it is loaded from the cache with no engine started, and its
///   first call makes its runtime, from the folder whose hash is pinned;
/// - changed since its pin, it is not loaded, its script never evaluated,
///   and the session says so with the command;
/// - turned off by `[extensions.probe] enabled = false`, it is not loaded,
///   and nothing is said or asked.
#[test]
fn a_users_extension_stands_by_its_pin() {
    let mut responses = searches_times(2);
    responses.push(calls(&[(
        "c1",
        "tool_call",
        json!({"tool": "probe/echo", "arguments": {"x": 1}}),
    )]));
    responses.push(says("Done."));
    responses.extend(searches_times(2));
    let world = World::new("found_users_standing", responses);
    let folder = users(&world);
    let command = format!("`cineris ext add {}`", users_shown(&world));
    write_extension(&folder, "", "Gives its arguments back");
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        // Never pinned.
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].contains("you have not approved it"), "{told:?}");
        assert!(told[0].contains(&command), "{told:?}");
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
    pin(&world, &folder);
    cache(&world, &folder);
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        // Pinned: found, judged and registered from the cache, no runtime
        // made and no script evaluated.
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        assert_eq!(cineris_ext::engine::runtimes_made(), made);
        assert!(about_probe(&front).is_empty(), "{:?}", front.notices());
        // Its first call makes its runtime.
        front.answering(Answering::allow_all());
        front.turn("echo").await;
        let results = front.results();
        assert_eq!(results[1].1, "success", "{results:?}");
        assert!(results[1].2.contains("\"got\""), "{results:?}");
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made + 1);
    // Changed after it was approved.
    write_extension(&folder, "", "the second");
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(
            told[0].contains("has changed since you approved it"),
            "{told:?}"
        );
        assert!(told[0].contains(&command), "{told:?}");
        front.close().await;
    });
    // The changed script was never evaluated.
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
    // Turned off, though pinned as it is now.
    world.user_config("[extensions.probe]\nenabled = false\n");
    pin(&world, &folder);
    cache(&world, &folder);
    let made = cineris_ext::engine::runtimes_made();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        // Nothing is said of it, and nothing asked: it is the user's word.
        assert!(about_probe(&front).is_empty(), "{:?}", front.notices());
        assert!(front.approvals().is_empty());
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
}

// ------------------------------------------------------------ the cache

#[test]
fn an_approved_extension_the_cache_misses_is_registered_and_the_first_turn_waits_for_it() {
    let world = World::new("found_cache_missed", searches());
    let folder = users(&world);
    write_extension(&folder, "", "Gives its arguments back");
    pin(&world, &folder);
    let hash = definition(&folder).hash;
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    let registering = Hold::new(cineris_core::ext::BEFORE_AN_EXTENSION_IS_REGISTERED);
    world.run(async {
        let front = start(&world, &core).await;
        let mut prompt = std::pin::pin!(front.prompt("look for the probe"));
        registering.drive_until_reached(prompt.as_mut()).await;
        // The registration is held: the prompt waits for it, no turn has
        // started and no request has gone out.
        assert!(poll_once(prompt.as_mut()).await.is_pending());
        front.drain();
        assert!(
            !front
                .events()
                .iter()
                .any(|event| matches!(event.event, Event::TurnStarted(_))),
            "a turn started before the extension was registered"
        );
        assert!(world.provider.requests().is_empty());
        // A turn that waits says why.
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.starts_with("Registering the extension `probe`")),
            "{:?}",
            front.notices()
        );
        assert!(cache_file(&world).load().cache.get(&hash).is_none());
        assert_eq!(cineris_ext::engine::runtimes_made(), made);
        registering.release();
        prompt.await;
        front.turns(1).await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    // Evaluated once, in a throwaway runtime, and written to the cache.
    assert_eq!(cineris_ext::engine::runtimes_made(), made + 1);
    let kept = cache_file(&world).load();
    let entry = kept.cache.get(&hash).expect("the registration is kept");
    assert_eq!(entry.name, NAME);
    assert_eq!(entry.registration.tools[0].name, "echo");
}

#[test]
fn a_cache_set_aside_is_said_once_and_what_is_approved_is_registered_again() {
    let world = World::new("found_set_aside", searches_times(2));
    let folder = users(&world);
    write_extension(&folder, "", "Gives its arguments back");
    pin(&world, &folder);
    let file = state(&world).registration_cache_file();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{\"version\": 1, \"entries\": {}, \"later\": true}").unwrap();
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        for session in 0..2 {
            let front = start(&world, &core).await;
            front.turn("look for the probe").await;
            assert!(found_probe(&front), "{:?}", front.results());
            let said: Vec<String> = front
                .notices()
                .into_iter()
                .filter(|notice| notice.contains("set aside"))
                .collect();
            // Said by the session that found it set aside, and by no other.
            assert_eq!(said.len(), usize::from(session == 0), "{said:?}");
            // With its files under `~`, never the home directory's path.
            let home = world.home.display().to_string();
            for said in &said {
                assert!(said.contains("`~/") && !said.contains(&home), "{said}");
            }
            front.close().await;
        }
    });
    // Registered again by the first session only: the second read it from
    // the new cache.
    assert_eq!(cineris_ext::engine::runtimes_made(), made + 1);
    let kept = cache_file(&world);
    assert!(kept.load().cache.get(&definition(&folder).hash).is_some());
    assert!(kept.set_aside_path().exists());
}

/// The cache is taken and kept as the pins say, a session after another in
/// one world and one core (folded for Windows' tier 2):
/// - an entry under the extension's hash that names another extension is no
///   entry of it: the extension is registered again, and kept as it is;
/// - pruning keeps every trusted project's entries, since projects share
///   the cache, and takes out a hash no pin names;
/// - a trust store that cannot be read prunes nothing.
#[test]
fn the_registration_cache_is_taken_and_pruned_as_the_pins_say() {
    let world = World::new("found_cache_kept", searches_times(3));
    let folder = users(&world);
    write_extension(&folder, "", "Gives its arguments back");
    pin(&world, &folder);
    let entry = |name: &str, tools: Vec<cineris_ext::registration::ToolDef>| {
        let manifest = Manifest::parse(&format!(
            "api = 1\nname = \"{name}\"\nversion = \"1\"\ndescription = \"x\"\nmain = \"main.js\"\n"
        ))
        .unwrap();
        Entry::new(
            &manifest,
            cineris_ext::registration::Registration {
                tools,
                ..cineris_ext::registration::Registration::default()
            },
            None,
        )
    };
    // Under the extension's hash, an entry that names another extension and
    // another tool: not what this folder registered.
    let mine = definition(&folder).hash;
    let planted = vec![cineris_ext::registration::ToolDef {
        name: "planted".to_owned(),
        description: "planted".to_owned(),
        input: json!({"type": "object"}),
        read_only: true,
    }];
    cache_file(&world)
        .put(&mine, entry("other", planted))
        .unwrap();
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        let results = front.results();
        assert!(found_probe(&front), "{results:?}");
        assert!(!results[0].2.contains("planted"), "{results:?}");
        front.close().await;
    });
    // Registered again, and kept as what it is.
    assert_eq!(cineris_ext::engine::runtimes_made(), made + 1);
    assert_eq!(
        cache_file(&world)
            .load()
            .cache
            .get(&mine)
            .map(|entry| entry.name.as_str()),
        Some(NAME)
    );
    // Another project's extension, trusted there; and one no pin names.
    let theirs = Definition::of(TrustResourceKind::Extension, "tracker", ["theirs"]);
    trust_store(&world)
        .trust(
            &ProjectKey::new("another"),
            "~/another",
            std::slice::from_ref(&theirs),
            false,
        )
        .unwrap();
    let gone = Definition::of(TrustResourceKind::Extension, "gone", ["old"]).hash;
    cache_file(&world)
        .put(&theirs.hash, entry("tracker", Vec::new()))
        .unwrap();
    cache_file(&world)
        .put(&gone, entry("gone", Vec::new()))
        .unwrap();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    let kept = cache_file(&world).load().cache;
    assert!(kept.get(&mine).is_some(), "the user's own is kept");
    assert!(
        kept.get(&theirs.hash).is_some(),
        "another trusted project's is kept"
    );
    assert!(kept.get(&gone).is_none(), "one no pin names is pruned");
    // An entry no pin the session can read names, in a trust store this
    // build does not understand: nothing is pruned.
    let unread = Definition::of(TrustResourceKind::Extension, "tracker", ["unread"]).hash;
    cache_file(&world)
        .put(&unread, entry("tracker", Vec::new()))
        .unwrap();
    std::fs::write(state(&world).trust_file(), "version = 9\n").unwrap();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    assert!(cache_file(&world).load().cache.get(&unread).is_some());
}

// ------------------------------------------------------------ a project's

#[test]
fn a_projects_extensions_are_not_looked_in_while_it_is_not_trusted() {
    let world = World::new("found_project_untrusted", searches());
    write_extension(&projects(&world), "", "the project's");
    // A folder that is no extension: a look in the folder would say so.
    world.put(".cineris/extensions/broken/extension.toml", "not = [toml");
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        // A client that answers no trust question: nothing looks.
        let client = core.connect(ClientInfo::headless());
        let opened = core
            .handle(
                client.id,
                Op::SessionNew(SessionNewParams {
                    cwd: None,
                    workspace: world.workspace.to_str().unwrap().to_owned(),
                    settings: SessionSettings::default(),
                    title: None,
                    config: serde_json::Map::new(),
                    ephemeral: true,
                }),
            )
            .await;
        let Ok(OpResult::SessionNew(opened)) = opened else {
            panic!("the session does not open: {opened:?}");
        };
        let front = Front {
            core: core.clone(),
            client,
            session: opened.session.session_id.clone(),
            state: opened.session,
            seen: std::cell::RefCell::new(Vec::new()),
            answered: std::cell::RefCell::new(Vec::new()),
            answering: std::cell::RefCell::new(Answering::default()),
        };
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        assert!(
            front
                .notices()
                .iter()
                .all(|notice| !notice.contains("broken") && !notice.contains("`probe`")),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
}

#[test]
fn a_projects_extension_is_listed_by_the_trust_question_in_words_and_loaded_after_a_yes() {
    let world = World::new("found_project_yes", searches_times(2));
    let folder = projects(&world);
    // A description that holds what acts unseen: a line break and a
    // right-to-left override.
    write_described(
        &folder,
        "Tracks issues\\nfor you\\u202Etxt.exe",
        "exec = [\"gh\"]\nexec_network = true\nread = [\"src/**\"]\nwrite = [\"out/**\"]\nnet \
         = [\"api.example.com\", \"*.example.org\"]\nhooks = [\"tool.before\"]",
        "the projects",
    );
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        let [request] = asked.as_slice() else {
            panic!("one question: {asked:?}");
        };
        let listed: Vec<_> = request
            .resources
            .iter()
            .filter(|resource| resource.kind == TrustResourceKind::Extension)
            .collect();
        let [resource] = listed.as_slice() else {
            panic!("the extension is listed once: {:?}", request.resources);
        };
        assert_eq!(resource.name, NAME);
        assert!(!resource.changed);
        let detail = &resource.detail;
        // This session's programs are not confined (the test's core has no
        // launcher, as on Windows): said before anything else.
        let first = format!(
            "`probe` in `.cineris/extensions/probe`. {} It may run `gh`.",
            cineris_core::ext::NOT_CONFINED
        );
        assert!(detail.starts_with(&first), "{detail:?}");
        for words in [
            "It may read the workspace's files that match `src/**`.",
            "It may write the workspace's files that match `out/**`.",
            "It may reach `api.example.com`; any host under `example.org`.",
            cineris_ext::event::Event::ToolBefore.sees(),
            "(`tool.before`)",
            "Its version is `1`.",
            // What acts unseen is shown as an escape, inside the quotes of
            // the author's words, on a line of their own.
            "\nIts author describes it, in their words:\n\"Tracks issues\\nfor you\\u{202e}txt.exe\"",
        ] {
            assert!(detail.contains(words), "{words:?} is not in {detail:?}");
        }
        assert!(
            !detail.contains('\u{202e}') && !detail.contains("issues\nfor"),
            "{detail:?}"
        );
        // The yes pinned it as it was listed; the session registers it,
        // since the cache misses it, and loads it.
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    let stored = trust_store(&world).project(&project_key(&world)).unwrap();
    assert_eq!(stored.standing(&definition(&folder)), Standing::Approved);
    // The next session is asked nothing, and loads it from the cache.
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, TrustResponse::default()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
}

#[test]
fn a_projects_extension_changed_since_it_was_approved_is_disabled_and_asked_about_as_changed() {
    let world = World::new("found_project_changed", searches_times(2));
    let folder = projects(&world);
    write_extension(&folder, "", "the first");
    trust(&world, &[definition(&folder)]);
    cache(&world, &folder);
    write_extension(&folder, "", "the second");
    let made = cineris_ext::engine::runtimes_made();
    let core = world.core();
    world.run(async {
        // Not now: the session opens with the project trusted, and its
        // changed extension disabled.
        let not_now = TrustResponse::default();
        let (front, asked) = open_answering(&world, &core, not_now).await;
        let [request] = asked.as_slice() else {
            panic!("one question: {asked:?}");
        };
        assert!(
            request.trusted,
            "the project is trusted, and asked what is new"
        );
        let [resource] = request.resources.as_slice() else {
            panic!("only the extension is new: {:?}", request.resources);
        };
        assert_eq!(
            (
                resource.kind.clone(),
                resource.name.as_str(),
                resource.changed
            ),
            (TrustResourceKind::Extension, NAME, true)
        );
        assert!(
            resource
                .detail
                .contains("It has changed since you approved it."),
            "{}",
            resource.detail
        );
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(
            told[0].contains("has changed since you approved it"),
            "{told:?}"
        );
        assert!(told[0].contains("trust question"), "{told:?}");
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
    // Another core asks again, and a yes approves the change.
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        front.turn("look for the probe").await;
        assert!(found_probe(&front), "{:?}", front.results());
        front.close().await;
    });
    let stored = trust_store(&world).project(&project_key(&world)).unwrap();
    assert_eq!(stored.standing(&definition(&folder)), Standing::Approved);
}

#[test]
fn a_projects_extension_of_a_users_name_is_not_used() {
    let world = World::new("found_same_name", searches());
    write_extension(&users(&world), "", "one of mine");
    pin(&world, &users(&world));
    cache(&world, &users(&world));
    // The project is trusted, and its extension is not approved: it would
    // be new, and asked about.
    write_extension(&projects(&world), "", "one of theirs");
    trust(&world, &[]);
    let core = world.core();
    world.run(async {
        // Not asked about: whatever the answer, it is not used.
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.turn("look for the probe").await;
        let results = front.results();
        assert!(results[0].2.contains("one of mine"), "{results:?}");
        assert!(!results[0].2.contains("one of theirs"), "{results:?}");
        let told = about_probe(&front);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].contains("is not used"), "{told:?}");
        assert!(told[0].contains("your own extension"), "{told:?}");
        front.close().await;
    });
}

// ------------------------------------------------------------ where they are

#[test]
fn a_projects_folder_of_extensions_behind_a_link_out_of_the_workspace_is_not_looked_in() {
    let world = World::new("found_project_link_out", searches());
    let outside = world.dir.join("outside").join("extensions");
    let folder = outside.join(NAME);
    write_extension(&folder, "", "from outside");
    // Trusted, and even pinned and cached as it is: what a repository's
    // link reaches is not the repository's.
    trust(&world, &[definition(&folder)]);
    cache(&world, &folder);
    std::fs::create_dir_all(world.workspace.join(".cineris")).unwrap();
    if !make_link(&outside, &world.workspace.join(".cineris/extensions")) {
        return;
    }
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look for the probe").await;
        assert!(!found_probe(&front), "{:?}", front.results());
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("leads out of the workspace")),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
}
