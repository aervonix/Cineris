//! The core's hooks at the loop's points and at the session's own
//! (`docs/steps/28/answered.md`, point 6), and a project's hooks through
//! trust: the tests that start no hook's process, and the few that start
//! one (`sys_…`), each as turns of one session.

use cineris_core::ext::{HookOutcome, HookRun};
use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::event::{Event, Notice};
use cineris_protocol::lease::{Budget, Effect, HardLimits, Lease, Level, NetworkAccess, PathSet};
use cineris_protocol::lease::{Rule, RuleAction, SandboxPolicy};
use cineris_protocol::methods::{
    ApprovalChoice, ClientRole, ConfigReloadParams, Op, OpResult, SessionRef, TrustResourceKind,
    TurnStartParams,
};
use cineris_protocol::{Item, Part};
use serde_json::json;

use crate::extensions_found::{open_answering, project_key, trust, trust_store, yes};
use crate::extensions_trust::{keep, not_now};
use crate::hook_commands::{entry, script, this_binary};
use crate::{Front, World, calls, says};

/// An entry that does not compile, on `event`, for `tools`: it is on its
/// event all the same, as a hook that fails, and starts nothing.
fn refused_entry(name: &str, event: &str, tools: &str) -> String {
    format!(
        "[hooks.{name}]\nevent = \"{event}\"\n{tools}command = [\"check\", \"--file={{path}}\"]\n"
    )
}

/// A refused gate holds its event: the call it is on is refused, with why;
/// and a reload that takes the entry out leaves the session as it opened,
/// the hooks' configuration taking effect in the next session, so that no
/// gate lapses and none is given that the session was not opened with.
#[test]
fn a_refused_gate_holds_its_event_and_a_reload_leaves_it_in_force() {
    let world = World::new(
        "hook_refused_gate",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Done."),
            calls(&[("c2", "read", json!({"path": "a.txt"}))]),
            says("Done."),
            calls(&[("c3", "read", json!({"path": "a.txt"}))]),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    world.user_config(&refused_entry(
        "gate",
        "tool.before",
        "tools = [\"read\"]\n",
    ));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.drain();
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("Your hook `gate` does not run")),
            "{:?}",
            front.notices()
        );
        front.turn("read it").await;
        let results = front.results();
        let (_, outcome, text) = &results[0];
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("cannot run"), "{text}");
        // The entry is taken out, and the configuration read again.
        world.user_config("");
        let OpResult::ConfigReload(reloaded) = front
            .op(Op::ConfigReload(ConfigReloadParams::default()))
            .await
            .unwrap()
        else {
            panic!("not the result of config/reload");
        };
        let said = format!("{reloaded:?}");
        assert!(said.contains("sessions opened from now on"), "{said}");
        front.turn("read it again").await;
        let results = front.results();
        let (_, outcome, text) = &results[1];
        assert_eq!(*outcome, "denied", "the gate holds for the session: {text}");
        front.close().await;
    });
    // A session opened after it has no such hook.
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("read it once more").await;
        let results = front.results();
        let (_, outcome, text) = &results[0];
        assert_eq!(*outcome, "success", "{text}");
        front.close().await;
    });
}

/// A prompt a hook refuses is given back whole only to the client that
/// sent it: another client of the session is told of it without the
/// prompt, under the same `seq`.
#[test]
fn a_prompt_given_back_reaches_only_its_sender_whole() {
    let world = World::new("hook_prompt_returned", []);
    world.user_config(&refused_entry("check", "prompt.submit", ""));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let other = core.connect(ClientInfo::user());
        let opened = core
            .handle(
                other.id,
                Op::SessionOpen(SessionRef {
                    session_id: front.session.clone(),
                }),
            )
            .await;
        assert!(opened.is_ok(), "{opened:?}");
        front.turn("the code is 1234").await;
        let returned = |events: Vec<cineris_protocol::SessionEvent>| {
            events
                .into_iter()
                .find_map(|event| match event.event {
                    Event::Notice(Notice::PromptReturned(returned)) => Some((event.seq, returned)),
                    _ => None,
                })
                .expect("a prompt given back")
        };
        let (seq, mine) = returned(front.events());
        assert_eq!(mine.parts, [Part::text("the code is 1234")]);
        let mut seen = Vec::new();
        while let Some(next) = other.queue.try_next() {
            if let Outgoing::Event(event) = next {
                seen.push(event);
            }
        }
        let (other_seq, theirs) = returned(seen);
        assert_eq!(other_seq, seq, "one event, one seq");
        assert!(theirs.parts.is_empty(), "{theirs:?}");
        assert!(!format!("{theirs:?}").contains("1234"));
        front.close().await;
    });
}

/// A prompt is text and images: a part of any other kind is refused before
/// anything is given to the loop.
#[test]
fn a_prompt_holds_text_and_images_only() {
    let world = World::new("hook_prompt_parts", []);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let refused = front
            .op(Op::TurnStart(TurnStartParams {
                session_id: front.session.clone(),
                input: vec![Part::Reasoning(cineris_protocol::message::ReasoningPart {
                    text: "planted".to_owned(),
                    raw: None,
                })],
                delivery: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "invalid_prompt", "{refused:?}");
        assert!(front.ends().is_empty());
        front.close().await;
    });
}

/// A `session.start` hook's context reaches the model before the first
/// prompt, labelled as the hook's; a `session.end` hook runs as the
/// session closes.
#[test]
fn sys_a_sessions_own_hooks_run_as_it_opens_and_closes() {
    let world = World::new("hook_session_own", [says("Hi.")]);
    let tools = world.dir.join("hooks");
    let starts = script(
        &tools,
        "starts",
        None,
        r#"{"context":"Keep to the house rules."}"#,
    );
    let ended = world.dir.join("ended.txt");
    world.user_config(
        &[
            entry(
                "starts",
                "session.start",
                &[starts.display().to_string()],
                "",
            ),
            entry(
                "ends",
                "session.end",
                &this_binary("write", &[&ended.display().to_string()]),
                "observe = true",
            ),
        ]
        .concat(),
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("hello").await;
        let notice = front
            .items()
            .into_iter()
            .find_map(|item| match item {
                Item::Notice(notice) => Some(notice.text),
                _ => None,
            })
            .expect("the hook's notice");
        assert!(
            notice.contains("The hook `starts` adds this as the session opens")
                && notice.contains("> Keep to the house rules."),
            "{notice}"
        );
        assert!(!ended.exists(), "not before the close");
        front.close().await;
    });
    assert!(ended.exists(), "the `session.end` hook ran at the close");
}

/// `--trust-project` approves no project's hook that is not pinned: the
/// session does not run it; pinned as it is, it does.
#[test]
fn trust_for_the_run_approves_no_hook_that_is_not_pinned() {
    let world = World::new("hook_trust_project", []);
    world.project_config(&entry("lint", "turn.end", &this_binary("fail", &[]), ""));
    let run = || HookRun {
        input: json!({"event": "turn.end"}),
        ..HookRun::default()
    };
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        assert!(
            core.run_hook_command(&front.session, "lint", run())
                .await
                .is_err()
        );
        front.close().await;
    });
    let lint = cineris_ext::hook::CommandHook::compile(
        "lint",
        cineris_ext::source::Source::Project,
        &cineris_ext::hook::Entry {
            event: Some("turn.end".to_owned()),
            command: this_binary("fail", &[]),
            ..cineris_ext::hook::Entry::default()
        },
    )
    .unwrap();
    trust(&world, &[lint.definition()]);
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        // Found, and refused at its start: the engine would ask, and a
        // project's hook never does.
        let ran = core.run_hook_command(&front.session, "lint", run()).await;
        let Ok(HookOutcome::Failed(why)) = &ran else {
            panic!("{ran:?}");
        };
        assert!(why.contains("a project's hook never asks"), "{why}");
        front.close().await;
    });
}

/// `cineris permissions trust`'s part of the core lists a project's hooks
/// as the trust question does, its environment said, and pins those it is
/// given, so that a session is not asked about them again.
#[test]
fn permissions_trust_lists_and_pins_a_projects_hooks() {
    let world = World::new("hook_permissions_trust", []);
    world.project_config(&entry("lint", "turn.end", &this_binary("fail", &[]), ""));
    let core = world.core();
    let project = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&project).unwrap();
    let [lint] = hooks.as_slice() else {
        panic!("{hooks:?}");
    };
    assert_eq!(lint.name, "lint");
    assert!(!lint.changed);
    assert!(
        lint.detail
            .contains("It runs with your environment, including any tokens in it."),
        "{}",
        lint.detail
    );
    let trusted = core.trust_project_with(&project, &[], &hooks).unwrap();
    assert_eq!(trusted.hooks, ["lint"]);
    assert!(core.project_hook_commands(&project).unwrap().is_empty());
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        let ran = core
            .run_hook_command(
                &front.session,
                "lint",
                HookRun {
                    input: json!({"event": "turn.end"}),
                    ..HookRun::default()
                },
            )
            .await;
        assert!(matches!(ran, Ok(HookOutcome::Failed(_))), "{ran:?}");
        front.close().await;
    });
}

/// A yes the trust store cannot keep approves the hooks shown, by their
/// hashes, for that session alone, and says so.
#[test]
fn a_yes_the_store_cannot_keep_approves_the_hooks_shown_for_the_session() {
    let world = World::new("hook_yes_unkept", []);
    world.project_config(&entry("lint", "turn.end", &this_binary("fail", &[]), ""));
    let state = crate::extensions_found::state(&world);
    std::fs::create_dir_all(state.root().join("trust.toml.lock")).unwrap();
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert_eq!(asked.len(), 1);
        front.drain();
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("its rules and the hooks you were shown")),
            "{:?}",
            front.notices()
        );
        let ran = core
            .run_hook_command(
                &front.session,
                "lint",
                HookRun {
                    input: json!({"event": "turn.end"}),
                    ..HookRun::default()
                },
            )
            .await;
        assert!(matches!(ran, Ok(HookOutcome::Failed(_))), "{ran:?}");
        front.close().await;
    });
    assert!(
        !trust_store(&world)
            .project(&project_key(&world))
            .unwrap()
            .is_trusted()
    );
}

/// A trusted project with a hook that is new is asked about it, and
/// nothing else; keep trusting it without the hook leaves the hook off,
/// and the core asks no more in its run.
#[test]
fn a_trusted_project_is_asked_about_a_new_hook_and_keep_leaves_it_off() {
    let world = World::new("hook_keep_new", []);
    trust(&world, &[]);
    world.project_config(&entry("lint", "turn.end", &this_binary("fail", &[]), ""));
    let run = || HookRun {
        input: json!({"event": "turn.end"}),
        ..HookRun::default()
    };
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, keep()).await;
        let [question] = asked.as_slice() else {
            panic!("one question: {asked:?}");
        };
        assert!(question.trusted);
        let kinds: Vec<_> = question
            .resources
            .iter()
            .map(|resource| resource.kind.clone())
            .collect();
        assert_eq!(kinds, [TrustResourceKind::Hook], "{:?}", question.resources);
        assert!(
            core.run_hook_command(&front.session, "lint", run())
                .await
                .is_err()
        );
        front.close().await;
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert!(asked.is_empty(), "asked again in the core's run: {asked:?}");
        front.close().await;
    });
}

/// A replacement is a person's to allow: a supervisor whose grant's rules
/// cover the replaced call is not put it, its allow of it is not taken, and
/// the call does not run on its word. The user's denial settles it.
#[test]
fn sys_a_replacement_is_answered_by_a_person_and_never_by_a_supervisors_grant() {
    let world = World::new(
        "hook_replacement_supervisor",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Done."),
        ],
    );
    world.put("a.txt", "the model's\n");
    world.put("b.txt", "the hook's\n");
    let swap = script(
        &world.dir.join("hooks"),
        "swap",
        None,
        r#"{"arguments":{"path":"b.txt"}}"#,
    );
    world.user_config(&entry(
        "swap",
        "tool.before",
        &[swap.display().to_string()],
        "tools = [\"read\"]",
    ));
    let everything = PathSet {
        all: true,
        roots: Vec::new(),
    };
    let grant: Lease = cineris_policy::standing_lease(
        cineris_protocol::ids::LeaseId::new("l_grant"),
        cineris_policy::StandingTerms {
            level: Level::Autonomous,
            rules: vec![Rule {
                effect: Effect::Read,
                pattern: "**".to_owned(),
                action: RuleAction::Allow,
            }],
            sandbox: SandboxPolicy {
                read: everything.clone(),
                write: everything,
                protect: PathSet::default(),
                network: NetworkAccess::Off,
            },
            models: Vec::new(),
            budget: Budget::default(),
            limits: HardLimits::default(),
        },
        cineris_policy::SandboxState::Enforcing,
    )
    .expect("a lease");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let supervisor = core.connect_with_grant(
            ClientInfo {
                role: ClientRole::Supervisor,
                ..ClientInfo::user()
            },
            Some(grant),
        );
        let opened = core
            .handle(
                supervisor.id,
                Op::SessionOpen(SessionRef {
                    session_id: front.session.clone(),
                }),
            )
            .await;
        assert!(opened.is_ok(), "{opened:?}");
        front.prompt("read it").await;
        let (id, request) = front.approval().await;
        assert!(request.replaced.is_some(), "{request:?}");
        // The supervisor was not put it.
        let mut put = false;
        while let Some(next) = supervisor.queue.try_next() {
            put |= matches!(next, Outgoing::Request { .. });
        }
        assert!(!put, "a replacement was put to the supervisor");
        // Its allow is not taken.
        let taken = core.respond(
            supervisor.id,
            &id,
            cineris_protocol::methods::ServerResponse::Approval(
                cineris_protocol::methods::ApprovalResponse {
                    choice: ApprovalChoice::AllowOnce,
                    note: None,
                },
            ),
        );
        assert!(taken.is_err(), "the supervisor's allow was taken");
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        let results = front.results();
        let (_, outcome, text) = &results[0];
        assert_eq!(*outcome, "denied", "{text}");
        assert!(!text.contains("the hook's"), "{text}");
        front.close().await;
    });
}

/// An entry that uses `{path}` does not run for a call that names no file
/// within the workspace (settled by the maintainers: it says nothing
/// there): not for a listing, which names a folder, nor for a read of a
/// file outside the workspace, which it is never handed. A gating one that
/// lists no `tools` is warned of once, as the session opens.
#[test]
fn a_hook_that_uses_path_does_not_run_for_a_call_that_names_no_file_within_the_workspace() {
    let world = World::new("hook_path_none", []);
    let outside = world.dir.join("outside.txt");
    std::fs::write(&outside, "outside\n").unwrap();
    world.provider.push(calls(&[
        ("c1", "list", json!({"path": "."})),
        ("c2", "read", json!({"path": outside.display().to_string()})),
    ]));
    world.provider.push(says("Done."));
    // It would refuse every call it ran for, as it fails.
    world.user_config(&entry(
        "per-file",
        "tool.before",
        &this_binary("fail", &["{path}"]),
        "",
    ));
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(crate::Answering::allow_all());
        front.drain();
        let warned: Vec<String> = front
            .notices()
            .into_iter()
            .filter(|notice| notice.contains("uses `{path}` and lists no `tools`"))
            .collect();
        assert_eq!(warned.len(), 1, "{:?}", front.notices());
        front.turn("look").await;
        let results = front.results();
        for (tool, outcome, text) in &results {
            assert_eq!(*outcome, "success", "{tool}: {text}");
        }
        assert_eq!(results.len(), 2);
        front.close().await;
    });
}

/// Two hooks with a write root never run at once: they share one baseline
/// of the look at the protected paths. The first holds it at the point
/// after it is taken, and the second does not reach it until the first is
/// let go. Their starts are then refused by a rule, so nothing is started.
#[test]
fn hooks_with_a_write_root_run_one_at_a_time() {
    let world = World::new("hook_writes_one_at_a_time", []);
    let program = std::env::current_exe().expect("the test binary has a path");
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"deny\"\n{}",
        program.display(),
        [
            entry(
                "first",
                "turn.end",
                &this_binary("fail", &[]),
                "write = true"
            ),
            entry(
                "second",
                "turn.end",
                &this_binary("fail", &[]),
                "write = true"
            ),
        ]
        .concat()
    ));
    let core = world.core();
    let writes = cineris_testkit::Hold::new(cineris_core::ext::A_HOOK_WRITES);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let run = || HookRun {
            input: json!({"event": "turn.end"}),
            ..HookRun::default()
        };
        let mut first = std::pin::pin!(core.run_hook_command(&front.session, "first", run()));
        let mut second = std::pin::pin!(core.run_hook_command(&front.session, "second", run()));
        let mut both = std::pin::pin!(async {
            let mut done = (None, None);
            std::future::poll_fn(|context| {
                if done.0.is_none()
                    && let std::task::Poll::Ready(ran) = first.as_mut().poll(context)
                {
                    done.0 = Some(ran);
                }
                if done.1.is_none()
                    && let std::task::Poll::Ready(ran) = second.as_mut().poll(context)
                {
                    done.1 = Some(ran);
                }
                if done.0.is_some() && done.1.is_some() {
                    std::task::Poll::Ready(())
                } else {
                    std::task::Poll::Pending
                }
            })
            .await;
            done
        });
        writes.drive_until_reached(both.as_mut()).await;
        // Everything else runs as far as it can: the other start waits for
        // the baseline, and does not reach the point.
        for _ in 0..64 {
            let mut once = std::pin::pin!(async {
                tokio::task::yield_now().await;
            });
            let _ = std::future::poll_fn(|context| {
                let _ = both.as_mut().poll(context);
                once.as_mut().poll(context)
            })
            .await;
        }
        assert_eq!(writes.arrivals(), 1, "two starts with a write root at once");
        writes.release();
        let (first, second) = both.await;
        for ran in [first, second] {
            assert!(matches!(ran, Some(Ok(HookOutcome::Failed(_)))), "{ran:?}");
        }
        assert_eq!(writes.arrivals(), 2);
        front.close().await;
    });
}

/// At `read-only` a hook with `write = true` is given no write root: its
/// start does not take the baseline that a start with one holds.
#[test]
fn a_write_hook_has_no_write_root_at_read_only() {
    let world = World::new("hook_write_read_only", []);
    let program = std::env::current_exe().expect("the test binary has a path");
    // Refused by a rule, so that nothing is started at all.
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"deny\"\n{}",
        program.display(),
        entry(
            "scribble",
            "turn.end",
            &this_binary("fail", &[]),
            "write = true",
        )
    ));
    let core = world.core();
    let writes = cineris_testkit::Hold::new(cineris_core::ext::A_HOOK_WRITES).skipping(usize::MAX);
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::user(),
            cineris_protocol::session::SessionSettings {
                level: Some(Level::ReadOnly),
                ..cineris_protocol::session::SessionSettings::default()
            },
        )
        .await
        .expect("the session starts");
        let ran = core
            .run_hook_command(
                &front.session,
                "scribble",
                HookRun {
                    input: json!({"event": "turn.end"}),
                    ..HookRun::default()
                },
            )
            .await;
        assert!(matches!(ran, Ok(HookOutcome::Failed(_))), "{ran:?}");
        assert_eq!(writes.arrivals(), 0, "a write root at read-only");
        front.close().await;
    });
}

/// An observer of `session.start` is not waited for, and a failure of it
/// is logged.
#[test]
fn a_failing_observer_of_session_start_is_logged() {
    let world = World::new("hook_start_observer", [says("Hi.")]);
    let program = std::env::current_exe().expect("the test binary has a path");
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"deny\"\n{}",
        program.display(),
        entry(
            "watch",
            "session.start",
            &this_binary("fail", &[]),
            "observe = true",
        )
    ));
    let core = world.core();
    let warnings = crate::extensions::Warnings::default();
    let logged = std::sync::Arc::clone(&warnings.0);
    tracing::subscriber::with_default(warnings, || {
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("hello").await;
            front.close().await;
        });
    });
    let logged = logged.lock().unwrap();
    assert!(
        logged.iter().any(
            |line| line.contains("a `session.start` observer failed") && line.contains("watch")
        ),
        "{logged:?}"
    );
}

#[cfg(target_os = "linux")]
mod confined {
    //! Where the sandbox enforces.

    use super::*;
    use crate::sandbox::{enforcing_here, launcher};

    /// A hook whose look stops a turn outside any (here `session.start`,
    /// before the first) is told to the user at once, and the next prompt
    /// is refused with it and given back, so that the user can look.
    #[test]
    fn sys_a_hooks_stop_outside_a_turn_is_told_and_refuses_the_next_prompt() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("hook_stop_outside", [says("Hi.")]);
        world.put(".github/README", "ci\n");
        world.user_config(&entry(
            "scribble",
            "session.start",
            &this_binary("write", &[".github/workflows/x.yml"]),
            "write = true",
        ));
        let core = world.core_with(|options| options.launcher = Some(launcher()));
        world.run(async {
            let front = Front::start(&world, &core).await;
            let ended = front.turn("hello").await;
            assert_eq!(
                ended.outcome,
                cineris_protocol::event::TurnOutcome::Denied,
                "{ended:?}"
            );
            let notices = front.notices();
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.contains("The hook `scribble` was stopped")),
                "{notices:?}"
            );
            let returned = front
                .events()
                .into_iter()
                .find_map(|event| match event.event {
                    Event::Notice(Notice::PromptReturned(returned)) => Some(returned),
                    _ => None,
                });
            let returned = returned.expect("the prompt given back");
            assert!(returned.message.contains("stopped a turn"), "{returned:?}");
            assert!(world.read(".github/workflows/x.yml").is_none(), "put back");
            // Once: the prompt sent again goes on.
            front.turn("hello again").await;
            assert_eq!(
                world.provider.requests().len(),
                1,
                "the model was asked once"
            );
            front.close().await;
        });
    }

    /// A hook without `network` does not reach the network where commands
    /// are confined; one with it does.
    #[test]
    fn sys_a_hook_reaches_the_network_only_with_network() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("hook_network", []);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connect = format!(
            "if (exec 3<>/dev/tcp/127.0.0.1/{port}) 2>/dev/null; then exit 0; else exit 6; fi"
        );
        let command = ["bash".to_owned(), "-c".to_owned(), connect];
        // Observers: the test runner's lines that this binary, as the
        // launcher, writes first are no answer, and an observer's is not
        // read.
        world.user_config(
            &[
                entry("closed", "turn.end", &command, "observe = true"),
                entry(
                    "open",
                    "turn.end",
                    &command,
                    "observe = true\nnetwork = true",
                ),
            ]
            .concat(),
        );
        let core = world.core_with(|options| options.launcher = Some(launcher()));
        let run = || HookRun {
            input: json!({"event": "turn.end"}),
            ..HookRun::default()
        };
        world.run(async {
            let front = Front::start(&world, &core).await;
            let closed = core.run_hook_command(&front.session, "closed", run()).await;
            let Ok(HookOutcome::Failed(why)) = &closed else {
                panic!("{closed:?}");
            };
            assert!(why.contains("exited with 6"), "{why}");
            let open = core.run_hook_command(&front.session, "open", run()).await;
            assert!(matches!(open, Ok(HookOutcome::Observed)), "{open:?}");
            front.close().await;
        });
        drop(listener);
    }
}
