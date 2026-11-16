//! The trust question a session's open asks (since step 23): what it
//! lists, and what each of the owner's three answers does
//! (`docs/steps/23/answered.md`; `docs/architecture/06-permissions-and-sandbox.md`,
//! "Project trust").

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use cortex_base::{PathOptions, PathResolver};
use cortex_core::{Client, ClientInfo, Core, OpError, Outgoing};
use cortex_policy::{Definition, TrustStore};
use cortex_protocol::lease::{Effect, Rule, RuleAction};
use cortex_protocol::methods::{
    Op, OpResult, ServerRequest, ServerResponse, SessionNewParams, TrustRequest, TrustResourceKind,
    TrustResponse,
};
use cortex_protocol::model::ModelRole;
use cortex_protocol::session::{SessionSettings, SessionState};

use crate::{MODEL, OTHER_MODEL, World};

/// A project with a rule of its own, one in a profile it defines, a model
/// and a first limit: something of each kind the question lists.
const PROJECT: &str = "[model]\nmain = \"claude-haiku-5-5\"\n\n\
    [[permissions.rules]]\neffect = \"write\"\nmatch = \".github/**\"\naction = \"allow\"\n\n\
    [profiles.ci]\n[[profiles.ci.permissions.rules]]\neffect = \"exec\"\nmatch = \"make \
    deploy\"\naction = \"allow\"\n\n[limits.turn]\ntime = \"10m\"\n";

/// What a client does with the trust question.
#[derive(Clone)]
enum Answer {
    With(TrustResponse),
    /// Changes the project's files, then trusts it.
    ChangeThenTrust,
    /// Goes away.
    Leave,
}

fn trust_it() -> Answer {
    Answer::With(TrustResponse {
        trusted: true,
        never: false,
    })
}

fn not_now() -> Answer {
    Answer::With(TrustResponse::default())
}

fn never() -> Answer {
    Answer::With(TrustResponse {
        trusted: false,
        never: true,
    })
}

/// Opens a session in the world's workspace as `info`, answering a trust
/// question with `answer`: the open's state and the questions asked.
async fn open(
    world: &World,
    core: &Core,
    info: ClientInfo,
    answer: Answer,
) -> (Result<SessionState, OpError>, Vec<TrustRequest>) {
    let client = core.connect(info);
    let opening = core.handle(
        client.id,
        Op::SessionNew(SessionNewParams {
            workspace: world.workspace.to_str().unwrap().to_owned(),
            settings: SessionSettings::default(),
            title: None,
            config: serde_json::Map::new(),
            ephemeral: false,
        }),
    );
    let mut asked = Vec::new();
    let result = {
        let answering = answering(world, core, &client, &answer, &mut asked);
        let mut opening = std::pin::pin!(opening);
        let mut answering = std::pin::pin!(answering);
        std::future::poll_fn(|context| {
            let _ = answering.as_mut().poll(context);
            opening.as_mut().poll(context)
        })
        .await
    };
    let state = result.map(|result| match result {
        OpResult::SessionNew(result) => result.session,
        other => panic!("not the result of session/new: {other:?}"),
    });
    (state, asked)
}

async fn answering(
    world: &World,
    core: &Core,
    client: &Client,
    answer: &Answer,
    asked: &mut Vec<TrustRequest>,
) {
    loop {
        match client.queue.next().await {
            Some(Outgoing::Request {
                id,
                request: ServerRequest::Trust(request),
            }) => {
                asked.push(request);
                match answer {
                    Answer::With(response) => core
                        .respond(client.id, &id, ServerResponse::Trust(response.clone()))
                        .expect("the question is open"),
                    Answer::ChangeThenTrust => {
                        // `git pull` brings a new rule while the dialog is
                        // open.
                        world.project_config(&format!(
                            "{PROJECT}\n[[permissions.rules]]\neffect = \"exec\"\nmatch = \
                             \"curl *\"\naction = \"allow\"\n"
                        ));
                        core.respond(
                            client.id,
                            &id,
                            ServerResponse::Trust(TrustResponse {
                                trusted: true,
                                never: false,
                            }),
                        )
                        .expect("the question is open");
                    }
                    Answer::Leave => core.disconnect(client.id).await,
                }
            }
            Some(_) => {}
            None => std::future::pending::<()>().await,
        }
    }
}

fn model(state: &SessionState) -> &str {
    state.roles[&ModelRole::Main].model.as_str()
}

fn store(world: &World) -> (TrustStore, cortex_protocol::ProjectKey) {
    let state = world.dirs().state.unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let store = TrustStore::new(state.join("trust.toml"), Arc::new(world.clock.clone()));
    let key = PathResolver::new(&world.workspace, PathOptions::new())
        .unwrap()
        .root()
        .project_key();
    (store, key)
}

fn world(test: &str) -> World {
    let world = World::new(test, []);
    world.project_config(PROJECT);
    world
}

/// The question lists every rule, in its profile, and every setting held
/// back, the first limit among them; a yes pins what it showed, and the
/// session is put together trusted.
#[test]
fn a_yes_pins_what_was_shown_and_the_session_is_trusted() {
    let world = world("trusting_yes");
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    let state = state.expect("the session opens");
    assert_eq!(asked.len(), 1);
    let listed: Vec<(TrustResourceKind, String, String)> = asked[0]
        .resources
        .iter()
        .map(|resource| {
            (
                resource.kind.clone(),
                resource.name.clone(),
                resource.detail.clone(),
            )
        })
        .collect();
    let has = |kind: TrustResourceKind, name: &str, detail: &str| {
        listed
            .iter()
            .any(|(k, n, d)| *k == kind && n == name && d.contains(detail))
    };
    assert!(
        has(
            TrustResourceKind::Rules,
            "permissions.rules",
            "allow write .github/**"
        ),
        "{listed:#?}"
    );
    assert!(
        has(
            TrustResourceKind::Rules,
            "profiles.ci.permissions.rules",
            "allow exec make deploy"
        ),
        "{listed:#?}"
    );
    assert!(
        has(TrustResourceKind::Settings, "model.main", OTHER_MODEL),
        "{listed:#?}"
    );
    assert!(
        has(
            TrustResourceKind::Settings,
            "limits.turn.time",
            "first limit"
        ),
        "{listed:#?}"
    );
    // Not a git repository: git is not listed.
    assert!(
        !listed
            .iter()
            .any(|(kind, ..)| *kind == TrustResourceKind::Repository),
        "{listed:#?}"
    );
    assert_eq!(model(&state), OTHER_MODEL, "the project's model applies");
    let (store, key) = store(&world);
    let trust = store.project(&key).unwrap();
    assert!(trust.is_trusted());
    // A later core asks nothing, and is trusted.
    let later = world.core();
    let (state, asked) = world.run(open(&world, &later, ClientInfo::user(), not_now()));
    assert!(asked.is_empty(), "{asked:#?}");
    assert_eq!(model(&state.expect("the session opens")), OTHER_MODEL);
}

/// Not now: untrusted, nothing stored, and not asked again by the same
/// core; a later core asks again.
#[test]
fn not_now_is_untrusted_and_not_asked_again_in_this_process() {
    let world = world("trusting_not_now");
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), not_now()));
    assert_eq!(asked.len(), 1);
    assert_eq!(model(&state.expect("the session opens")), MODEL);
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    assert!(asked.is_empty(), "asked twice in one process");
    assert_eq!(model(&state.expect("the session opens")), MODEL);
    let (store, key) = store(&world);
    assert!(!store.project(&key).unwrap().is_trusted());
    assert!(!store.project(&key).unwrap().is_refused());
    let later = world.core();
    let (_, asked) = world.run(open(&world, &later, ClientInfo::user(), not_now()));
    assert_eq!(asked.len(), 1, "a later process asks again");
}

/// Never: untrusted, stored, and not asked at later starts; trusting the
/// project (`cortex permissions trust`) lifts it.
#[test]
fn never_is_kept_across_starts_until_the_project_is_trusted() {
    let world = world("trusting_never");
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), never()));
    assert_eq!(asked.len(), 1);
    let state = state.expect("the session opens");
    assert_eq!(model(&state), MODEL);
    assert_eq!(
        state.trust,
        Some(cortex_protocol::session::TrustStanding::Never),
        "the session it was said in says it"
    );
    let (store, key) = store(&world);
    assert!(store.project(&key).unwrap().is_refused());
    let later = world.core();
    let (state, asked) = world.run(open(&world, &later, ClientInfo::user(), trust_it()));
    assert!(asked.is_empty(), "a refused project was asked about");
    assert_eq!(model(&state.expect("the session opens")), MODEL);
    let project = later.project(&world.workspace).unwrap();
    assert_eq!(
        later.project_trust(&project).unwrap(),
        cortex_core::TrustState::Never
    );
    // Forgotten, the next start asks again; never again keeps it.
    assert!(later.forget_project(&project).unwrap());
    let (_, asked) = world.run(open(&world, &world.core(), ClientInfo::user(), never()));
    assert_eq!(asked.len(), 1, "a forgotten project was not asked about");
    // Trusting it lifts the never.
    later.trust_project(&project).unwrap();
    assert!(!store.project(&key).unwrap().is_refused());
    let (state, _) = world.run(open(&world, &world.core(), ClientInfo::user(), not_now()));
    assert_eq!(model(&state.expect("the session opens")), OTHER_MODEL);
}

/// A project trusted with its rules changed since is asked about again,
/// its rules marked changed, and a yes approves them.
#[test]
fn a_project_whose_rules_changed_is_asked_about_them() {
    let world = world("trusting_changed");
    let (store, key) = store(&world);
    let seen = Rule {
        effect: Effect::Write,
        pattern: "docs/**".to_owned(),
        action: RuleAction::Allow,
    };
    store
        .trust(&key, "~/work/app", &[Definition::of_rules(&[seen])])
        .unwrap();
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    assert_eq!(asked.len(), 1);
    let rules: Vec<_> = asked[0]
        .resources
        .iter()
        .filter(|resource| resource.kind == TrustResourceKind::Rules)
        .collect();
    assert!(!rules.is_empty());
    assert!(rules.iter().all(|resource| resource.changed), "{rules:#?}");
    assert_eq!(model(&state.expect("the session opens")), OTHER_MODEL);
    let (state, asked) = world.run(open(&world, &world.core(), ClientInfo::user(), not_now()));
    assert!(asked.is_empty(), "{asked:#?}");
    assert_eq!(model(&state.expect("the session opens")), OTHER_MODEL);
}

/// Never, said of a trusted project whose rules changed, is in force in the
/// session it was said in: the project's trust is gone there, not only at
/// the next start.
#[test]
fn never_of_a_trusted_project_is_in_force_at_once() {
    let world = world("trusting_changed_never");
    let (store, key) = store(&world);
    let seen = Rule {
        effect: Effect::Write,
        pattern: "docs/**".to_owned(),
        action: RuleAction::Allow,
    };
    store
        .trust(&key, "~/work/app", &[Definition::of_rules(&[seen])])
        .unwrap();
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), never()));
    assert_eq!(asked.len(), 1);
    let state = state.expect("the session opens");
    assert_eq!(model(&state), MODEL, "the project's settings were used");
    assert_eq!(
        state.trust,
        Some(cortex_protocol::session::TrustStanding::Never)
    );
}

/// A change to the project's rules while the question is open is not
/// pinned: what was shown is, and the session reads the change as rules
/// changed.
#[test]
fn a_change_while_the_question_is_open_is_not_pinned() {
    let world = world("trusting_changed_meanwhile");
    let core = world.core();
    let (state, asked) = world.run(open(
        &world,
        &core,
        ClientInfo::user(),
        Answer::ChangeThenTrust,
    ));
    assert_eq!(asked.len(), 1);
    let (store, key) = store(&world);
    assert!(store.project(&key).unwrap().is_trusted());
    // The rule added meanwhile was not seen: what it would loosen is set
    // aside, the model with it.
    assert_eq!(model(&state.expect("the session opens")), MODEL);
}

/// Headless mode answers no question: it is never asked, as before.
#[test]
fn a_headless_client_is_never_asked() {
    let world = world("trusting_headless");
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::headless(), trust_it()));
    assert!(asked.is_empty());
    assert_eq!(model(&state.expect("the session opens")), MODEL);
}

/// A client that goes away while it is asked gets no session, and the
/// core goes on: another opens one.
#[test]
fn a_client_that_leaves_while_asked_gets_no_session() {
    let world = world("trusting_leaves");
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), Answer::Leave));
    assert_eq!(asked.len(), 1);
    assert!(state.is_err(), "{state:?}");
    let (state, _) = world.run(open(&world, &core, ClientInfo::user(), not_now()));
    assert!(state.is_ok(), "{state:?}");
}

fn git(dir: &Path, arguments: &[&str]) -> bool {
    Command::new("git")
        .args(["-c", "init.defaultBranch=main"])
        .args(arguments)
        .current_dir(dir)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// In a git repository, the question lists git, which trust lets read the
/// repository's own configuration; a project with nothing else to trust is
/// asked about it all the same.
#[test]
fn sys_in_a_repository_git_is_listed() {
    let world = World::new("trusting_git", []);
    if !git(&world.workspace, &["init", "-q"]) {
        return;
    }
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), not_now()));
    state.expect("the session opens");
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0]
            .resources
            .iter()
            .any(|resource| resource.kind == TrustResourceKind::Repository),
        "{:#?}",
        asked[0].resources
    );
}

/// A folder with nothing to trust (no git, no project file) asks nothing.
#[test]
fn a_folder_with_nothing_to_trust_asks_nothing() {
    let world = World::new("trusting_nothing", []);
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    state.expect("the session opens");
    assert!(asked.is_empty(), "{asked:#?}");
}
