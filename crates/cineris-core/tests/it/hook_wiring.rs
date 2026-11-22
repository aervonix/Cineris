//! The core's hooks at the loop's points and at the session's own
//! (`docs/steps/28/answered.md`, point 6), and a project's hooks through
//! trust.
//!
//! The tests that start no hook's process are few and long: each is one
//! world whose sessions go through one story, so that the integration tier
//! pays for few worlds (the A/B of step 28's second pull request found a
//! world and a session for each rule). The few that start one (`sys_…`)
//! are each turns of one session.

use std::time::Duration;

use cineris_core::ext::{HookOutcome, HookRun};
use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::event::{Event, Notice, TurnOutcome};
use cineris_protocol::lease::Level;
use cineris_protocol::methods::{
    ConfigReloadParams, Op, OpResult, SessionRef, TrustResourceKind, TurnStartParams,
};
use cineris_protocol::session::SessionSettings;
use cineris_protocol::{Item, Part};
use serde_json::json;

use crate::extensions_found::{open_answering, project_key, start, trust_store, yes};
use crate::extensions_trust::{keep, not_now};
use crate::hook_commands::{Lingering, entry, for_edit, script, this_binary};
use crate::{Front, World, calls, says};

/// An entry that does not compile, on `event`, for `tools`: it is on its
/// event all the same, as a hook that fails, and starts nothing.
fn refused_entry(name: &str, event: &str, tools: &str) -> String {
    format!(
        "[hooks.{name}]\nevent = \"{event}\"\n{tools}command = [\"check\", \"--file={{path}}\"]\n"
    )
}

/// A rule that refuses every start of this test binary: a hook that would
/// start it fails, with why, and starts nothing.
fn no_start_of_this_binary() -> String {
    let program = std::env::current_exe().expect("the test binary has a path");
    format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"deny\"\n",
        program.display()
    )
}

/// An event of the session's turn's end, for `Core::run_hook_command`.
fn turn_end() -> HookRun {
    HookRun {
        input: json!({"event": "turn.end"}),
        ..HookRun::default()
    }
}

/// Whether one of `front`'s notices says `words`.
fn told(front: &Front, words: &str) -> bool {
    front.drain();
    front.notices().iter().any(|notice| notice.contains(words))
}

/// Yields until `ready` says so: the runtime's other tasks reach the state
/// it names, with no count of yields that a slow machine could fall short
/// of.
async fn until(ready: impl Fn() -> bool) {
    while !ready() {
        tokio::task::yield_now().await;
    }
}

/// Polls `operation` until `ready` says so, the runtime's other tasks
/// running between polls. Fails when the operation ends first.
async fn drive_until<F: std::future::Future>(
    mut operation: std::pin::Pin<&mut F>,
    ready: impl Fn() -> bool,
) {
    while !ready() {
        let mut once = std::pin::pin!(tokio::task::yield_now());
        let finished = std::future::poll_fn(|context| {
            if operation.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(true);
            }
            once.as_mut().poll(context).map(|()| false)
        })
        .await;
        assert!(
            !finished,
            "the operation ended before the state it was driven to"
        );
    }
}

/// How many of the timer's running waits are `length` long.
fn waits_of(world: &World, length: Duration) -> usize {
    world
        .timer
        .running()
        .iter()
        .filter(|wait| **wait == length)
        .count()
}

/// A user's hooks that start nothing, in one world: a rule refuses every
/// start of this binary, so that each hook is told apart by why it failed.
/// The hooks the test runs by hand are on `compaction.before`, which no
/// turn here reaches, so that the loop starts none of them beside a phase.
///
/// The first session, at `supervised`:
/// - its first prompt waits for its `session.start` hook that is waited
///   for (one the rule refuses, which the user is told of): the prompt is
///   held at the wait while the hook is not yet heard;
/// - a refused entry whose `tools` cannot be read holds `tool.before` for
///   every call (failing closed), with why; and a reload that takes it out
///   leaves it in force, the hooks' configuration taking effect in the
///   next session, so that no gate lapses and none is given that the
///   session was not opened with;
/// - a hook a rule denies, or a rule of the managed configuration asks
///   about, starts nothing, and says which;
/// - an event of more than 1 MiB is no hook's input;
/// - two starts with a write root never run at once: they share one
///   baseline of the look at the protected paths, so the first holds it at
///   the point after it is taken while the second waits for it;
/// - a prompt is text and images: a part of another kind is refused before
///   the loop has it.
///
/// The second, opened after the reload: the refused entry is gone, so the
/// calls run; an entry that uses `{path}` does not run for a call that
/// names no file within the workspace (a listing, which names a folder; a
/// read outside it, whose file it is never handed: settled by the
/// maintainers), while a gating one that lists no `tools` is warned of once
/// as the session opens; a waited `turn.end` hook has at most four runs
/// going, and a fifth is not started, and says so; and a run let go before
/// it is first polled (the review of step 28's second pull request, M3)
/// starts nothing: it never reaches the baseline its write root would take.
///
/// The third, at `read-only`: a hook with `write = true` is given no write
/// root, so its start does not take the baseline.
///
/// The fourth, opened after a reload that adds nine observers of
/// `session.start` with `write = true`: one holds the baseline at its
/// point, seven wait for it, and the ninth waits for a place among the
/// observers within its own time, and fails when that is up; a hook that is
/// waited for starts all the same, as the observers have a pool of their
/// own (the re-check of that review: they starved a gate, which failed
/// closed). The close lets go of what waits, which starts nothing, and
/// waits for what runs (M1, M3).
///
/// The fifth and sixth end together when the core shuts down: neither's
/// `session.end` hooks wait for the other's.
///
/// And an observer of `session.start` that fails, never waited for, is
/// logged.
#[test]
fn a_users_hooks_hold_their_events_and_start_nothing_a_rule_holds_back() {
    use cineris_core::ext::{
        A_CLOSE_WAITS_FOR_A_RUN, A_FIRST_TURN_WAITS, A_HOOK_WRITES, A_SESSION_ENDS,
        A_SESSION_START_HOOK_IS_HEARD,
    };
    use cineris_testkit::Hold;

    let world = World::new("hook_users", []);
    let outside = world.dir.join("outside.txt");
    std::fs::write(&outside, "outside\n").unwrap();
    for id in ["c1", "c2"] {
        world
            .provider
            .push(calls(&[(id, "list", json!({"path": "."}))]));
        world.provider.push(says("Done."));
    }
    world.provider.push(calls(&[
        ("c3", "list", json!({"path": "."})),
        ("c4", "read", json!({"path": outside.display().to_string()})),
    ]));
    world.provider.push(says("Done."));
    // A script the managed configuration asks about, which the user's rule
    // does not name: it is never started.
    let held = script(&world.dir.join("hooks"), "held", None, "{}");
    let file = world.dirs().managed_config.unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        format!(
            "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"ask\"\n",
            held.display()
        ),
    )
    .unwrap();
    let config = |gate: bool, scribes: bool| {
        let fails = this_binary("fail", &[]);
        let mut entries = vec![
            if gate {
                refused_entry("gate", "tool.before", "tools = [\"not a name\"]\n")
            } else {
                String::new()
            },
            entry("ready", "session.start", &fails, ""),
            entry("watch", "session.start", &fails, "observe = true"),
            entry(
                "per-file",
                "tool.before",
                &this_binary("fail", &["{path}"]),
                "",
            ),
            entry("cap", "turn.end", &fails, ""),
            entry("first", "compaction.before", &fails, "write = true"),
            entry("second", "compaction.before", &fails, "write = true"),
            entry(
                "held",
                "compaction.before",
                &[held.display().to_string(), "x".to_owned()],
                "",
            ),
            entry("bye", "session.end", &fails, "observe = true"),
        ];
        if scribes {
            for number in 1..=9 {
                let time = if number == 9 { "90s" } else { "2m" };
                entries.push(entry(
                    &format!("scribe{number}"),
                    "session.start",
                    &fails,
                    &format!("observe = true\nwrite = true\ntimeout = \"{time}\""),
                ));
            }
        }
        format!("{}{}", no_start_of_this_binary(), entries.concat())
    };
    world.user_config(&config(true, false));
    let core = world.core();
    let warnings = crate::extensions::Warnings::default();
    let logged = std::sync::Arc::clone(&warnings.0);
    let logged_says = |words: &[&str]| {
        logged
            .lock()
            .unwrap()
            .iter()
            .any(|line| words.iter().all(|word| line.contains(word)))
    };
    tracing::subscriber::with_default(warnings, || {
        world.run(async {
            // The first prompt waits for the `session.start` hook, which is
            // not heard yet.
            let hears = Hold::new(A_SESSION_START_HOOK_IS_HEARD);
            let waits = Hold::new(A_FIRST_TURN_WAITS);
            let front = start(&world, &core).await;
            let mut first = std::pin::pin!(front.prompt("look"));
            waits.drive_until_reached(first.as_mut()).await;
            hears.drive_until_reached(first.as_mut()).await;
            waits.release();
            drop(waits);
            hears.release();
            drop(hears);
            first.await;
            front.turns(1).await;
            assert!(
                told(&front, "The hook `ready` failed"),
                "{:?}",
                front.notices()
            );
            assert!(
                told(&front, "Your hook `gate` does not run"),
                "{:?}",
                front.notices()
            );
            // The refused gate on every call.
            let results = front.results();
            let (_, outcome, text) = &results[0];
            assert_eq!(*outcome, "denied", "{text}");
            assert!(text.contains("cannot run"), "{text}");
            // The entry is taken out, and the configuration read again.
            world.user_config(&config(false, false));
            let OpResult::ConfigReload(reloaded) = front
                .op(Op::ConfigReload(ConfigReloadParams::default()))
                .await
                .unwrap()
            else {
                panic!("not the result of config/reload");
            };
            let said = format!("{reloaded:?}");
            assert!(said.contains("sessions opened from now on"), "{said}");
            front.turn("look again").await;
            let results = front.results();
            let (_, outcome, text) = &results[1];
            assert_eq!(*outcome, "denied", "the gate holds for the session: {text}");

            // A rule's deny, a managed `ask`: nothing is started.
            for (hook, said) in [
                ("cap", "a permission rule refuses it"),
                ("held", "a rule of the managed configuration"),
            ] {
                let ran = core
                    .run_hook_command(&front.session, hook, turn_end())
                    .await
                    .unwrap();
                let HookOutcome::Failed(why) = &ran else {
                    panic!("{hook}: {ran:?}");
                };
                assert!(why.contains(said), "{hook}: {why}");
            }
            // An event past its cap.
            let ran = core
                .run_hook_command(
                    &front.session,
                    "cap",
                    HookRun {
                        input: json!({"text": "a".repeat(1024 * 1024)}),
                        ..HookRun::default()
                    },
                )
                .await
                .unwrap();
            let HookOutcome::Failed(why) = &ran else {
                panic!("{ran:?}");
            };
            assert!(why.contains("a hook is given at most 1 MiB"), "{why}");

            // Two starts with a write root, one at a time: the first holds
            // the baseline at its point, and the second waits for it
            // within its own time (its five seconds on the clock).
            let writes = Hold::new(A_HOOK_WRITES);
            let mut one =
                std::pin::pin!(core.run_hook_command(&front.session, "first", turn_end()));
            let mut two =
                std::pin::pin!(core.run_hook_command(&front.session, "second", turn_end()));
            let mut both = std::pin::pin!(async {
                let mut done = (None, None);
                std::future::poll_fn(|context| {
                    if done.0.is_none()
                        && let std::task::Poll::Ready(ran) = one.as_mut().poll(context)
                    {
                        done.0 = Some(ran);
                    }
                    if done.1.is_none()
                        && let std::task::Poll::Ready(ran) = two.as_mut().poll(context)
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
            drive_until(both.as_mut(), || {
                waits_of(&world, Duration::from_secs(5)) == 1
            })
            .await;
            assert_eq!(writes.arrivals(), 1, "two starts with a write root at once");
            writes.release();
            let (one, two) = both.await;
            for ran in [one, two] {
                assert!(matches!(ran, Some(Ok(HookOutcome::Failed(_)))), "{ran:?}");
            }
            assert_eq!(writes.arrivals(), 2);
            drop(writes);

            // A prompt of another part than text and images.
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
            assert_eq!(front.ends().len(), 2, "the loop had the prompt");
            front.close().await;

            // A session opened after the reload has no such gate; and an
            // entry that uses `{path}` does not run where no file within the
            // workspace is named.
            let front = start(&world, &core).await;
            front.answering(crate::Answering::allow_all());
            front.drain();
            let warned: Vec<String> = front
                .notices()
                .into_iter()
                .filter(|notice| notice.contains("uses `{path}` and lists no `tools`"))
                .collect();
            assert_eq!(warned.len(), 1, "{:?}", front.notices());
            front.turn("look once more").await;
            let results = front.results();
            for (tool, outcome, text) in &results {
                assert_eq!(*outcome, "success", "{tool}: {text}");
            }
            assert_eq!(results.len(), 2);
            // A waited `turn.end` hook's fifth run at once.
            let said = core
                .hook_runs_at_its_point(&front.session, "cap", 5)
                .await
                .unwrap();
            let (four, fifth) = said.split_at(4);
            for one in four {
                assert!(
                    one.as_deref()
                        .is_some_and(|why| why.contains("a permission rule refuses it")),
                    "{said:?}"
                );
            }
            assert!(
                fifth[0]
                    .as_deref()
                    .is_some_and(|why| why.contains("4 runs of it are going on")),
                "{said:?}"
            );
            // A run let go before it is first polled starts nothing: the
            // close waits for its task, which never reaches the baseline.
            let writes = Hold::new(A_HOOK_WRITES).skipping(usize::MAX);
            core.let_go_of_a_hook_run(&front.session, "first").unwrap();
            front.close().await;
            assert_eq!(
                writes.arrivals(),
                0,
                "a run let go before it was polled went on to start"
            );
            drop(writes);

            // At `read-only`, no write root.
            let writes = Hold::new(A_HOOK_WRITES).skipping(usize::MAX);
            let front = Front::start_as(
                &world,
                &core,
                ClientInfo::user(),
                SessionSettings {
                    level: Some(Level::ReadOnly),
                    ..SessionSettings::default()
                },
            )
            .await
            .expect("the session starts");
            let ran = core
                .run_hook_command(&front.session, "first", turn_end())
                .await;
            assert!(matches!(ran, Ok(HookOutcome::Failed(_))), "{ran:?}");
            assert_eq!(writes.arrivals(), 0, "a write root at read-only");
            // The observers the fourth session opens with.
            world.user_config(&config(false, true));
            front
                .op(Op::ConfigReload(ConfigReloadParams::default()))
                .await
                .unwrap();
            front.close().await;
            drop(writes);

            // Nine observers with a write root: one holds the baseline,
            // seven wait for it (each its two minutes on the clock), and the
            // ninth waits for a place (its ninety seconds).
            let writes = Hold::new(A_HOOK_WRITES);
            let front = start(&world, &core).await;
            let mut idle = std::pin::pin!(std::future::pending::<()>());
            writes.drive_until_reached(idle.as_mut()).await;
            until(|| {
                waits_of(&world, Duration::from_secs(120)) == 7
                    && waits_of(&world, Duration::from_secs(90)) == 1
            })
            .await;
            // A hook that is waited for starts all the same; were it kept
            // out, its five seconds would be moved past, and it would fail
            // closed.
            let mut gated =
                std::pin::pin!(core.run_hook_command(&front.session, "cap", turn_end()));
            let mut moved = false;
            let gated = std::future::poll_fn(|context| {
                if let std::task::Poll::Ready(done) = gated.as_mut().poll(context) {
                    return std::task::Poll::Ready(done);
                }
                if !moved && waits_of(&world, Duration::from_secs(5)) > 0 {
                    moved = true;
                    world.timer.advance(Duration::from_secs(5));
                }
                context.waker().wake_by_ref();
                std::task::Poll::Pending
            })
            .await
            .unwrap();
            let HookOutcome::Failed(why) = &gated else {
                panic!("{gated:?}");
            };
            assert!(
                why.contains("a permission rule refuses it"),
                "a hook that is waited for was kept out by observers: {why}"
            );
            // The ninth observer's time is up while it waits for a place.
            world.timer.advance(Duration::from_secs(90));
            until(|| logged_says(&["scribe9", "did not start within its 90000 ms"])).await;
            assert!(
                logged_says(&["scribe9", "other hooks were running"]),
                "the ninth observer did not wait for a place: {:?}",
                logged.lock().unwrap()
            );
            // The close lets go of the seven that wait, and waits for the
            // one at the baseline.
            let closing = Hold::new(A_CLOSE_WAITS_FOR_A_RUN);
            let mut close = std::pin::pin!(front.close());
            closing.drive_until_reached(close.as_mut()).await;
            closing.release();
            drop(closing);
            writes.release();
            close.await;
            assert_eq!(
                writes.arrivals(),
                1,
                "a start the close let go of went on to the baseline"
            );
            drop(writes);

            // Two sessions' ends, together.
            let ends = Hold::new(A_SESSION_ENDS);
            let five = start(&world, &core).await;
            let six = start(&world, &core).await;
            let mut down = std::pin::pin!(core.shutdown());
            ends.drive_until_reached_times(2, down.as_mut()).await;
            ends.release();
            down.await;
            drop((five, six));
        });
    });
    assert!(
        logged_says(&["a `session.start` observer failed", "watch"]),
        "{:?}",
        logged.lock().unwrap()
    );
}

/// A project that is not trusted changes nothing in a session by entries
/// of it that do not compile (the review of step 28's second pull request,
/// H1, found that one gated every call): with a refused `tool.before`
/// entry and a refused `prompt.submit` one in it, the prompt is sent and
/// the call runs as with no hooks at all, and the user is told of neither,
/// nor warned of an entry of it that relies on `{path}`. A session with no
/// hooks keeps nothing of a prompt once it is recorded (M5): its ticket,
/// which is all the core keeps of it (the ticket and the client it came
/// from, never the prompt's parts), is let go.
#[test]
fn an_untrusted_projects_refused_entries_change_nothing() {
    let world = World::new(
        "hook_untrusted_refused",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    world.project_config(
        &[
            refused_entry("gate", "tool.before", ""),
            refused_entry("check", "prompt.submit", ""),
            entry(
                "per-file",
                "tool.before",
                &this_binary("fail", &["{path}"]),
                "",
            ),
        ]
        .concat(),
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let ended = front.turn("read it").await;
        assert_eq!(ended.outcome, TurnOutcome::Completed, "{ended:?}");
        assert_eq!(
            core.prompt_tickets(&front.session).unwrap(),
            (1, 0),
            "one ticket was handed out, and none is kept once the prompt is recorded"
        );
        let results = front.results();
        let [(tool, outcome, text)] = results.as_slice() else {
            panic!("{results:?}");
        };
        assert_eq!(tool, "read");
        assert_eq!(*outcome, "success", "{text}");
        assert!(
            !told(&front, "`gate`") && !told(&front, "`check`") && !told(&front, "`per-file`"),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
}

/// A project's hooks reach a session only through trust, in one world, a
/// core for each run of the harness the story needs (a core keeps a "not
/// now" or a "keep" for its run).
///
/// 1. A yes the trust store cannot keep: the question lists the project's
///    own hook in words, and neither one of a user's name nor one turned
///    off (`enabled = false`); the yes approves the hooks shown, by their
///    hashes, for that session alone, and says so; the hook is the
///    session's, and its start is refused all the same, as the engine
///    would ask about it at `supervised` and a project's hook never puts a
///    question; an entry of the project's that does not compile is told
///    once, the project trusted; one turned off is not the session's.
/// 2. Not now: a project not trusted runs none of its hooks, and tells of
///    none of its entries. Beside it, a prompt the user's refused
///    `prompt.submit` entry refuses is given back whole only to the client
///    that sent it: another client of the session is told of it without
///    the prompt, under the same `seq`.
/// 3. `--trust-project` approves no hook that is not pinned.
/// 4. `cineris permissions trust`'s part of the core lists the project's
///    hooks as the question does, its environment said, and pins those it
///    is given, so that a session is not asked about them again.
/// 5. `--trust-project` runs a hook pinned as it is.
/// 6. A trusted project with a hook that is new is asked about it, and
///    nothing else; keep trusting it without the hook leaves the hook off,
///    and the core asks no more in its run.
/// 7. A yes pins the hook as it was shown: a session opened again is not
///    asked about it.
#[test]
fn a_projects_hooks_reach_a_session_only_through_trust() {
    let world = World::new("hook_project", []);
    world.user_config(
        &[
            refused_entry("mine", "turn.end", ""),
            refused_entry("check", "prompt.submit", ""),
        ]
        .concat(),
    );
    let project = |more: &str| {
        [
            entry(
                "lint",
                "tool.after",
                &this_binary("report", &["{path}"]),
                "tools = [\"edit\"]",
            ),
            entry("mine", "turn.end", &this_binary("hang", &[]), ""),
            entry(
                "off",
                "turn.end",
                &this_binary("fail", &[]),
                "enabled = false",
            ),
            refused_entry("odd", "tool.after", ""),
            more.to_owned(),
        ]
        .concat()
    };
    world.project_config(&project(""));
    let lint = || for_edit(&["src/a.rs"]);
    let never_asks = |ran: Result<HookOutcome, cineris_core::OpError>| {
        let Ok(HookOutcome::Failed(why)) = &ran else {
            panic!("{ran:?}");
        };
        assert!(why.contains("a project's hook never asks"), "{why}");
    };

    // 1. A yes the store cannot keep.
    let state = crate::extensions_found::state(&world);
    let lock = state.root().join("trust.toml.lock");
    std::fs::create_dir_all(&lock).unwrap();
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        let [question] = asked.as_slice() else {
            panic!("one question: {asked:?}");
        };
        let hooks: Vec<_> = question
            .resources
            .iter()
            .filter(|resource| resource.kind == TrustResourceKind::Hook)
            .collect();
        let [listed] = hooks.as_slice() else {
            panic!("the project's own hook alone: {:?}", question.resources);
        };
        assert_eq!(listed.name, "lint");
        assert!(
            listed.detail.contains("on `tool.after`"),
            "{}",
            listed.detail
        );
        assert!(listed.detail.contains("for `edit`"), "{}", listed.detail);
        assert!(
            told(&front, "its rules and the hooks you were shown"),
            "{:?}",
            front.notices()
        );
        assert!(
            told(&front, "This project's hook `mine` does not run"),
            "{:?}",
            front.notices()
        );
        assert!(
            told(&front, "This project's hook `odd` does not run"),
            "{:?}",
            front.notices()
        );
        never_asks(core.run_hook_command(&front.session, "lint", lint()).await);
        assert!(
            core.run_hook_command(&front.session, "off", turn_end())
                .await
                .is_err(),
            "a hook turned off is the session's"
        );
        front.close().await;
    });
    std::fs::remove_dir_all(&lock).unwrap();
    assert!(
        !trust_store(&world)
            .project(&project_key(&world))
            .unwrap()
            .is_trusted()
    );

    // 2. Not now; and a prompt given back.
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(
            core.run_hook_command(&front.session, "lint", lint())
                .await
                .is_err()
        );
        assert!(!told(&front, "`odd`"), "{:?}", front.notices());
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
        assert_eq!(
            core.prompt_tickets(&front.session).unwrap(),
            (1, 0),
            "one ticket was handed out, and none is kept once the prompt is given back"
        );
        front.close().await;
    });

    // 3. `--trust-project`, nothing pinned.
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let (front, _) = open_answering(&world, &core, not_now()).await;
        assert!(
            core.run_hook_command(&front.session, "lint", lint())
                .await
                .is_err()
        );
        front.close().await;
    });

    // 4. `cineris permissions trust`.
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    let [listed] = hooks.as_slice() else {
        panic!("{hooks:?}");
    };
    assert_eq!(listed.name, "lint");
    assert!(!listed.changed);
    assert!(
        listed
            .detail
            .contains("It runs with your environment, including any tokens in it."),
        "{}",
        listed.detail
    );
    let trusted = core.trust_project_with(&key, &[], &hooks).unwrap();
    assert_eq!(trusted.hooks, ["lint"]);
    assert!(core.project_hook_commands(&key).unwrap().is_empty());
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        never_asks(core.run_hook_command(&front.session, "lint", lint()).await);
        front.close().await;
    });

    // 5. `--trust-project`, pinned as it is.
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let (front, _) = open_answering(&world, &core, not_now()).await;
        never_asks(core.run_hook_command(&front.session, "lint", lint()).await);
        front.close().await;
    });

    // 6. A new hook, kept off.
    world.project_config(&project(&entry(
        "fmt",
        "turn.end",
        &this_binary("fail", &[]),
        "",
    )));
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
            .map(|resource| (resource.kind.clone(), resource.name.clone()))
            .collect();
        assert_eq!(
            kinds,
            [(TrustResourceKind::Hook, "fmt".to_owned())],
            "{:?}",
            question.resources
        );
        assert!(
            core.run_hook_command(&front.session, "fmt", turn_end())
                .await
                .is_err()
        );
        front.close().await;
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert!(asked.is_empty(), "asked again in the core's run: {asked:?}");
        front.close().await;
    });

    // 7. A yes pins it.
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert_eq!(asked.len(), 1, "{asked:?}");
        front.close().await;
    });
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        never_asks(
            core.run_hook_command(&front.session, "fmt", turn_end())
                .await,
        );
        front.close().await;
    });
}

/// A `session.start` hook's context reaches the model before the first
/// prompt, labelled as the hook's; an observer of it runs beside the
/// session, and the close stops it and waits for it (the review of step
/// 28's second pull request, M1: one outlived its session); a
/// `session.end` hook runs as the session closes, and one that runs on is
/// stopped when the close's five seconds on the session's clock are up
/// (M8): the clock is moved past its own time in the same step, and it is
/// the close's stop that ends it, not its time.
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
    let lingers = Lingering::new(&tools, "lingers");
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
                "lingers",
                "session.start",
                &lingers.command,
                "observe = true\ntimeout = \"10m\"",
            ),
            entry(
                "ends",
                "session.end",
                &this_binary("write", &[&ended.display().to_string()]),
                "observe = true",
            ),
            // The first of the session's `session.end` hooks by name, so
            // that the close's wait is on it when its five seconds are up.
            entry(
                "drags",
                "session.end",
                &this_binary("hang", &[]),
                "observe = true\ntimeout = \"7s\"",
            ),
        ]
        .concat(),
    );
    let core = world.core();
    let warnings = crate::extensions::Warnings::default();
    let logged = std::sync::Arc::clone(&warnings.0);
    let _logging = tracing::subscriber::set_default(warnings);
    world.run(async {
        let front = Front::start(&world, &core).await;
        // The observer is running.
        world.timer.sleeping_for(Duration::from_secs(600)).await;
        let lingering = lingers.started();
        front.turn("hello").await;
        let items = front.items();
        let at = |notice: bool| {
            items.iter().position(|item| match item {
                Item::Notice(_) => notice,
                Item::UserMessage(_) => !notice,
                _ => false,
            })
        };
        assert!(
            at(true) < at(false),
            "the first prompt went before the `session.start` hook: {items:?}"
        );
        let notice = items
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
        // The close: the `drags` hook runs on until the close's five
        // seconds are up, moved once `ends` has written its file and `drags`
        // runs; and past its own seven in the same step, so that a close
        // with no bound of its own has it end by its time instead.
        let mut close = std::pin::pin!(front.close());
        let mut up = std::pin::pin!(async {
            while !(ended.exists() && world.timer.running().contains(&Duration::from_secs(7))) {
                tokio::task::yield_now().await;
            }
            world.timer.advance(Duration::from_secs(5));
            world.timer.advance(Duration::from_secs(2));
        });
        let mut advanced = false;
        std::future::poll_fn(|context| {
            if !advanced {
                advanced = up.as_mut().poll(context).is_ready();
            }
            close.as_mut().poll(context)
        })
        .await;
        Lingering::assert_gone(
            lingering,
            "the `session.start` observer outlived its session",
        );
    });
    assert!(ended.exists(), "the `session.end` hook ran at the close");
    let logged = logged.lock().unwrap();
    let drags: Vec<&String> = logged
        .iter()
        .filter(|line| line.contains("a `session.end` hook failed") && line.contains("drags"))
        .collect();
    assert!(
        drags.len() == 1 && drags[0].contains("interrupted") && !drags[0].contains("ran past"),
        "the close's five seconds did not stop the hook: {logged:?}"
    );
}

#[cfg(target_os = "linux")]
mod confined {
    //! Where the sandbox enforces.

    use super::*;
    use crate::sandbox::{enforcing_here, launcher};

    /// A hook whose look stops a turn outside any (here `session.start`,
    /// before the first) is told to the user at once, and the next prompt
    /// is refused with it and given back, so that the user can look. The
    /// session's `session.end` hooks, a command's and a script's, are of no
    /// turn, so that stop refuses neither: both run as the session closes
    /// (the full review of the script hooks, L8: both had `t_0`).
    #[test]
    fn sys_a_hooks_stop_outside_a_turn_is_told_and_refuses_the_next_prompt() {
        if !enforcing_here() {
            return;
        }
        let world = World::new("hook_stop_outside", [says("Hi.")]);
        world.put(".github/README", "ci\n");
        world.user_config(
            &[
                entry(
                    "scribble",
                    "session.start",
                    &this_binary("write", &[".github/workflows/x.yml"]),
                    "write = true",
                ),
                entry(
                    "goodbye",
                    "session.end",
                    &this_binary("write", &["ended.txt"]),
                    "write = true",
                ),
            ]
            .concat(),
        );
        let farewell = crate::ext_exec::extension_running(
            &world,
            "probe",
            "hooks = [\"session.end\"]",
            "cineris.on('session.end', async () => { \
               await cineris.state.set('ended', 'yes', { scope: 'project' }); });\n",
        );
        let core = world.core_with(|options| {
            options.launcher = Some(launcher());
            options.extensions = vec![farewell];
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            let ended = front.turn("hello").await;
            assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
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
        assert!(
            world.read("ended.txt").is_some(),
            "the `session.end` command was refused by a `session.start` hook's stop"
        );
        let kept = crate::ext_state::project_files(&world);
        assert!(
            kept.iter().any(|file| std::fs::read_to_string(file)
                .is_ok_and(|text| text.contains("\"ended\":\"yes\""))),
            "the `session.end` script hook was refused by a `session.start` hook's stop: {kept:?}"
        );
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
        world.run(async {
            let front = Front::start(&world, &core).await;
            let closed = core
                .run_hook_command(&front.session, "closed", turn_end())
                .await;
            let Ok(HookOutcome::Failed(why)) = &closed else {
                panic!("{closed:?}");
            };
            assert!(why.contains("exited with 6"), "{why}");
            let open = core
                .run_hook_command(&front.session, "open", turn_end())
                .await;
            assert!(matches!(open, Ok(HookOutcome::Observed)), "{open:?}");
            front.close().await;
        });
        drop(listener);
    }
}
