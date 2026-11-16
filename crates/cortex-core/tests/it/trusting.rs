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
    /// Ends the core, answering nothing.
    ShutDown,
    /// Takes the project's trust away, as another client or process
    /// would (marking it never with `refuse`), then trusts it.
    UntrustThenTrust {
        refuse: bool,
    },
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
                    Answer::ShutDown => core.shutdown().await,
                    Answer::UntrustThenTrust { refuse } => {
                        let (store, key) = store(world);
                        if *refuse {
                            store.refuse(&key, "~/work/app").unwrap();
                        } else {
                            assert!(store.revoke(&key).unwrap());
                        }
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

/// A yes to a project whose rules changed, whose trust another client or
/// process took away while the question was open, is not used: nothing is
/// written, a never said meanwhile stays on disk, and the session opens
/// with the trust as it stands now (the review of step 23's fixes: a yes to
/// changed rules is no yes to the whole project, git included, nor past a
/// never). A project revoked meanwhile is asked about afresh.
#[test]
fn a_yes_after_the_projects_trust_changed_meanwhile_is_not_used() {
    for refuse in [false, true] {
        let world = world("trusting_changed_untrusted");
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
        let (state, asked) = world.run(open(
            &world,
            &core,
            ClientInfo::user(),
            Answer::UntrustThenTrust { refuse },
        ));
        assert_eq!(asked.len(), 1);
        assert!(asked[0].resources.iter().any(|resource| resource.changed));
        let state = state.expect("the session opens");
        assert_eq!(model(&state), MODEL, "refuse={refuse}");
        let stored = store.project(&key).unwrap();
        assert!(!stored.is_trusted(), "refuse={refuse}");
        assert_eq!(stored.is_refused(), refuse, "the never stays");
        assert_eq!(
            state.trust,
            Some(if refuse {
                cortex_protocol::session::TrustStanding::Never
            } else {
                cortex_protocol::session::TrustStanding::Untrusted
            })
        );
        let (_, again) = world.run(open(&world, &core, ClientInfo::user(), not_now()));
        assert_eq!(again.len(), usize::from(!refuse), "refuse={refuse}");
    }
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

/// With the profile locked by the managed file, the project's other
/// profiles can never be selected: their rules are not listed, and the
/// locked profile's rules are not listed under their names (the review of
/// step 23).
#[test]
fn a_profile_the_managed_file_locks_out_is_not_listed() {
    let world = World::new("trusting_locked_profile", []);
    let managed = world.dirs().managed_config.unwrap();
    std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
    std::fs::write(
        &managed,
        "profile = \"review\"\n\n[managed]\nlocked = [\"profile\"]\n",
    )
    .unwrap();
    world.project_config(
        "[[profiles.review.permissions.rules]]\neffect = \"write\"\nmatch = \"docs/**\"\naction = \
         \"allow\"\n\n[[profiles.ci.permissions.rules]]\neffect = \"exec\"\nmatch = \"make \
         deploy\"\naction = \"allow\"\n\n[profiles.review.retry]\nattempts = 3\n\n\
         [profiles.ci.retry]\nattempts = 2\n",
    );
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), not_now()));
    state.expect("the session opens");
    assert_eq!(asked.len(), 1);
    let rules: Vec<(String, String)> = asked[0]
        .resources
        .iter()
        .filter(|resource| resource.kind == TrustResourceKind::Rules)
        .map(|resource| (resource.name.clone(), resource.detail.clone()))
        .collect();
    assert_eq!(
        rules,
        [(
            "profiles.review.permissions.rules".to_owned(),
            "allow write docs/**".to_owned()
        )]
    );
    // Nor are its settings, and the locked profile's are not listed under
    // its name.
    let settings: Vec<String> = asked[0]
        .resources
        .iter()
        .filter(|resource| resource.kind == TrustResourceKind::Settings)
        .map(|resource| resource.name.clone())
        .collect();
    assert!(
        settings.contains(&"profiles.review.retry.attempts".to_owned()),
        "{settings:?}"
    );
    assert!(
        !settings.iter().any(|name| name.starts_with("profiles.ci.")),
        "{settings:?}"
    );
}

/// A yes the trust store cannot keep, given after the project's rules
/// changed while the question was open, trusts the session as a yes the
/// store kept would: what the new rules could loosen is set aside (the
/// review of step 23's fixes).
#[test]
fn a_yes_the_store_cannot_keep_after_the_rules_changed_sets_them_aside() {
    let world = world("trusting_store_fails_changed");
    let state = world.dirs().state.unwrap();
    std::fs::create_dir_all(state.join("trust.toml.lock")).unwrap();
    let core = world.core();
    let (state, asked) = world.run(open(
        &world,
        &core,
        ClientInfo::user(),
        Answer::ChangeThenTrust,
    ));
    assert_eq!(asked.len(), 1);
    let state = state.expect("the session opens");
    assert_eq!(model(&state), MODEL, "the project's settings were used");
    assert_eq!(
        state.trust,
        Some(cortex_protocol::session::TrustStanding::RulesChanged)
    );
}

/// The core's end withdraws an open trust question (the review of step
/// 23): the open ends, refused as the core is ending, and does not wait
/// for a person.
#[test]
fn the_core_ending_ends_a_trust_question() {
    let world = world("trusting_shutdown");
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), Answer::ShutDown));
    assert_eq!(asked.len(), 1);
    assert_eq!(
        state.map(|_| ()).map_err(|error| error.code),
        Err("shutting_down".to_owned())
    );
}

/// The core begins to end after an open has looked at the project's trust
/// and before its question is put: the question, put after the end
/// withdrew what was open, is taken back at once, so the open ends refused
/// and an answer is refused (the review of step 23's fixes).
#[test]
fn a_trust_question_put_as_the_core_ends_is_taken_back() {
    let mut world = world("trusting_looked_shutdown");
    // Each piece of blocking work waits one turn: polled once, the open
    // waits for its look, and the core ends before the look is made.
    world.one_turn_each();
    let core = world.core();
    let client = core.connect(ClientInfo::user());
    let (answered, state) = world.run(async {
        let mut opening = std::pin::pin!(core.handle(
            client.id,
            Op::SessionNew(SessionNewParams {
                workspace: world.workspace.to_str().unwrap().to_owned(),
                settings: SessionSettings::default(),
                title: None,
                config: serde_json::Map::new(),
                ephemeral: false,
            }),
        ));
        std::future::poll_fn(|context| {
            let _ = opening.as_mut().poll(context);
            std::task::Poll::Ready(())
        })
        .await;
        core.shutdown().await;
        let answering = async {
            loop {
                if let Some(Outgoing::Request {
                    id,
                    request: ServerRequest::Trust(_),
                }) = client.queue.next().await
                {
                    return core.respond(
                        client.id,
                        &id,
                        ServerResponse::Trust(TrustResponse::default()),
                    );
                }
            }
        };
        let mut answering = std::pin::pin!(answering);
        let mut answered = None;
        let state = std::future::poll_fn(|context| {
            if answered.is_none()
                && let std::task::Poll::Ready(result) = answering.as_mut().poll(context)
            {
                answered = Some(result.map_err(|error| error.code));
            }
            opening.as_mut().poll(context)
        })
        .await;
        // The open may end in the poll that puts the question, before the
        // answering half has looked: the question is in the queue all the
        // same, and is answered now.
        if answered.is_none() {
            while let Some(outgoing) = client.queue.try_next() {
                if let Outgoing::Request {
                    id,
                    request: ServerRequest::Trust(_),
                } = outgoing
                {
                    answered = Some(
                        core.respond(
                            client.id,
                            &id,
                            ServerResponse::Trust(TrustResponse::default()),
                        )
                        .map_err(|error| error.code),
                    );
                }
            }
        }
        (answered, state.map(|_| ()).map_err(|error| error.code))
    });
    assert_eq!(answered, Some(Err("request_not_open".to_owned())));
    assert_eq!(state, Err("shutting_down".to_owned()));
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

/// An answer the trust store cannot keep (its lock cannot be taken) still
/// opens the session (the review of step 23): a yes trusts the project for
/// this session alone, and a never is taken as not now.
#[test]
fn an_answer_the_store_cannot_keep_still_opens_the_session() {
    let world = world("trusting_store_fails");
    let state = world.dirs().state.unwrap();
    std::fs::create_dir_all(state.join("trust.toml.lock")).unwrap();
    let core = world.core();
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    assert_eq!(asked.len(), 1);
    let state = state.expect("a yes the store cannot keep opens the session");
    assert_eq!(model(&state), OTHER_MODEL, "trusted for this session");
    assert_eq!(
        state.trust,
        Some(cortex_protocol::session::TrustStanding::ForTheRun)
    );
    let (store, key) = store(&world);
    assert!(!store.project(&key).unwrap().is_trusted());
    // Asked again by the same core, since nothing was kept; a never that
    // cannot be kept is not now: the session opens untrusted, and the core
    // does not ask again.
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), never()));
    assert_eq!(asked.len(), 1);
    assert_eq!(model(&state.expect("the session opens")), MODEL);
    assert!(!store.project(&key).unwrap().is_refused());
    let (_, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    assert!(asked.is_empty(), "{asked:#?}");
}

/// An answer that says both "trust it" and "never" is refused where it is
/// given, and the question stays open for one that can be read (the review
/// of step 23: it was taken as a yes).
#[test]
fn an_answer_of_both_trust_and_never_is_refused() {
    let world = world("trusting_both");
    let core = world.core();
    let client = core.connect(ClientInfo::user());
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
    let answering = async {
        loop {
            if let Some(Outgoing::Request {
                id,
                request: ServerRequest::Trust(_),
            }) = client.queue.next().await
            {
                let both = core.respond(
                    client.id,
                    &id,
                    ServerResponse::Trust(TrustResponse {
                        trusted: true,
                        never: true,
                    }),
                );
                let refused = both.expect_err("a contradictory answer is refused");
                assert_eq!(refused.code, "invalid_answer");
                core.respond(
                    client.id,
                    &id,
                    ServerResponse::Trust(TrustResponse::default()),
                )
                .expect("the question is still open");
                std::future::pending::<()>().await;
            }
        }
    };
    let result = world.run(async {
        let mut opening = std::pin::pin!(opening);
        let mut answering = std::pin::pin!(answering);
        std::future::poll_fn(|context| {
            let _ = answering.as_mut().poll(context);
            opening.as_mut().poll(context)
        })
        .await
    });
    let Ok(OpResult::SessionNew(result)) = result else {
        panic!("{result:?}");
    };
    assert_eq!(model(&result.session), MODEL, "not trusted");
}

/// A profile only the project's files define can be selected (the review of
/// step 23): the core starts with it, the question lists the settings each
/// of the project's profiles holds back, the selected one's and the
/// others', and a yes applies the selected one's. `permissions trust` lists
/// the same.
#[test]
fn a_profile_only_the_project_defines_can_be_selected_and_its_settings_are_shown() {
    let world = World::new("trusting_project_profile", []);
    world.project_config(&format!(
        "[profiles.review]\nmodel.main = \"{OTHER_MODEL}\"\n\n[profiles.ci]\nretry.attempts = 2\n\
         [profiles.ci.shell]\nenv_remove = [\"*\"]\n"
    ));
    let core = world.core_with(|options| options.profile = Some("review".to_owned()));
    let (state, asked) = world.run(open(&world, &core, ClientInfo::user(), trust_it()));
    assert_eq!(asked.len(), 1);
    let settings: Vec<(String, String)> = asked[0]
        .resources
        .iter()
        .filter(|resource| resource.kind == TrustResourceKind::Settings)
        .map(|resource| (resource.name.clone(), resource.detail.clone()))
        .collect();
    let names: Vec<&str> = settings.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "profiles.ci.retry.attempts",
            "profiles.ci.shell.env_remove",
            "profiles.review.model.main"
        ],
        "{settings:#?}"
    );
    assert!(settings[1].1.contains("stop them all"), "{settings:#?}");
    assert_eq!(model(&state.expect("the session opens")), OTHER_MODEL);

    let project = core.project(&world.workspace).unwrap();
    let trusted = core.trust_project(&project).unwrap();
    let listed: Vec<(Option<&str>, String)> = trusted
        .settings
        .iter()
        .map(|(profile, setting)| (profile.as_deref(), setting.key.to_string()))
        .collect();
    assert_eq!(
        listed,
        [
            (Some("ci"), "retry.attempts".to_owned()),
            (Some("ci"), "shell.env_remove".to_owned()),
            (Some("review"), "model.main".to_owned())
        ]
    );
}
