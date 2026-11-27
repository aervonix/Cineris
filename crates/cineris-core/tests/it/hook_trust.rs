//! A trusted project's approved hook commands against what its file does
//! to them and against its trust taken away (the review of step 28,
//! findings 4 and 5).
//!
//! - **An approved gate does not lapse.** Once a project's hook is
//!   approved, its entry changed (its `observe`, its event, its tools, or
//!   only its time), taken out, turned off (`enabled = false`) or no longer
//!   compiling holds a failing gate on `tool.before` for every call and on
//!   `prompt.submit`: what it gated when approved is not known (its pin
//!   keeps its hash alone), and the project's file, which the model may
//!   have written, lifts none of it. The person lifts it: a yes to the
//!   changed entry, or a yes that takes a removed one's approval away
//!   (the trust question, `cineris permissions trust`).
//! - **A project's hooks leave with its trust,** at the reload's look that
//!   sees it taken away, its held gates with them: the person's act, not
//!   the model's.

use cineris_core::ext::HookOutcome;
use cineris_policy::Definition;
use cineris_protocol::event::{Event, Notice, TurnOutcome};
use cineris_protocol::methods::{ConfigReloadParams, Op, TrustResourceKind};
use serde_json::json;

use crate::extensions_found::{open_answering, project_key, state, trust_store, yes};
use crate::extensions_trust::not_now;
use crate::hook_commands::{entry, for_edit, this_binary};
use crate::{Front, World, calls, says};

/// A project's gate on `tool.before` for the tools `tools` (TOML), with
/// `extra` keys: its program fails, so it refuses what it is on.
fn gate(tools: &str, extra: &str) -> String {
    entry(
        "gate",
        "tool.before",
        &this_binary("fail", &[]),
        &format!("tools = [{tools}]\n{extra}"),
    )
}

/// A `config/reload` of what changed, as the interface's look makes it.
async fn reload(
    front: &Front,
) -> Result<cineris_protocol::methods::OpResult, cineris_core::OpError> {
    front
        .op(Op::ConfigReload(ConfigReloadParams {
            changed_only: true,
            ..ConfigReloadParams::default()
        }))
        .await
}

/// Why the last prompt given back to `front`'s client was refused.
fn returned(front: &Front) -> String {
    front.drain();
    front
        .events()
        .into_iter()
        .rev()
        .find_map(|event| match event.event {
            Event::Notice(Notice::PromptReturned(returned)) => Some(returned.message),
            _ => None,
        })
        .expect("a prompt given back")
}

/// The read's outcome in a session of a new core, opened answering the
/// trust question with `answer`: the outcome and what the model was shown,
/// with how many questions were put.
fn read_once(
    world: &World,
    answer: cineris_protocol::methods::TrustResponse,
) -> (String, String, usize) {
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(world, &core, answer).await;
        let ended = front.turn("read it").await;
        assert_eq!(ended.outcome, TurnOutcome::Completed, "{ended:?}");
        let (tool, outcome, text) = front.results().first().cloned().expect("the read");
        assert_eq!(tool, "read");
        front.close().await;
        (outcome.to_string(), text, asked.len())
    })
}

/// A project's gate on `read`, approved as `cineris permissions trust`
/// approves it, refuses the read. Then its file changes it in each way
/// that, were the held gate made from the changed entry, would move it off
/// the read (`observe`, its event, its tools), and in a way that would not
/// (its time); takes it out; turns it off; and breaks it. Each time,
/// answered not now, the session holds every prompt, the gate refusing it,
/// and says how to lift it, naming neither the entry nor `enabled = false`
/// as a way out. A yes to a changed entry lifts it: the read runs; a yes to
/// the question that lists a removed one takes its approval away: the read
/// runs, and the next session is not asked. Before the fix, every change
/// but the time let the read run, and a removed or turned off gate held
/// nothing, unsaid.
#[test]
fn an_approved_projects_gate_holds_whatever_its_file_does_to_it() {
    let read = || calls(&[("c1", "read", json!({"path": "a.txt"}))]);
    let world = World::new(
        "hook_trust_held",
        [
            read(),
            says("Done."),
            read(),
            says("Done."),
            read(),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    world.project_config(&gate("\"read\"", ""));
    let _ = state(&world);
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    assert_eq!(hooks.len(), 1, "{hooks:?}");
    core.trust_project_with(&key, &[], &hooks).unwrap();

    // Approved as it is: the gate refuses the read, and nothing is asked.
    let (outcome, text, asked) = read_once(&world, not_now());
    assert_eq!((outcome.as_str(), asked), ("denied", 0), "{text}");

    let cases = [
        ("observe", gate("\"read\"", "observe = true"), "has changed"),
        (
            "its event",
            entry(
                "gate",
                "tool.after",
                &this_binary("fail", &[]),
                "tools = [\"read\"]",
            ),
            "has changed",
        ),
        ("its tools", gate("\"edit\"", ""), "has changed"),
        (
            "its time",
            gate("\"read\"", "timeout = \"6s\""),
            "has changed",
        ),
        (
            "taken out",
            String::new(),
            "is no longer in the project's file",
        ),
        (
            "turned off",
            gate("\"read\"", "enabled = false"),
            "turned off",
        ),
        (
            "broken",
            "[hooks.gate]\nevent = \"tool.before\"\ncommand = [\"check\", \"--file={path}\"]\n"
                .to_owned(),
            "no longer compiles",
        ),
    ];
    for (case, config, said) in cases {
        world.project_config(&config);
        let core = world.core();
        world.run(async {
            let (front, _) = open_answering(&world, &core, not_now()).await;
            let ended = front.turn("read it").await;
            assert_eq!(ended.outcome, TurnOutcome::Denied, "{case}: {ended:?}");
            let why = returned(&front);
            for words in ["`gate`", said, "holds every call and every prompt"] {
                assert!(why.contains(words), "{case}: {words}: {why}");
            }
            assert!(!why.contains("Correct its entry"), "{case}: {why}");
            let told = front
                .notices()
                .into_iter()
                .filter(|notice| notice.contains("holds every tool call and every prompt"))
                .count();
            assert_eq!(told, 1, "{case}: {:?}", front.notices());
            assert!(front.results().is_empty(), "{case}: nothing ran");
            front.close().await;
        });
    }

    // Changed, and approved as it is now: on `edit` alone, the read runs.
    world.project_config(&gate("\"edit\"", ""));
    let (outcome, text, asked) = read_once(&world, yes());
    assert_eq!((outcome.as_str(), asked), ("success", 1), "{text}");

    // Taken out, and its approval taken away by a yes to the question that
    // lists it as removed: the read runs, and nothing is asked again.
    world.project_config("");
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        let [question] = asked.as_slice() else {
            panic!("one question: {asked:?}");
        };
        let [listed] = question.resources.as_slice() else {
            panic!("the removed hook alone: {:?}", question.resources);
        };
        assert_eq!(
            (&listed.kind, listed.name.as_str()),
            (&TrustResourceKind::Hook, "gate")
        );
        for words in [
            "Removed:",
            "is no longer in the project's file",
            "A yes takes its approval away",
        ] {
            assert!(listed.detail.contains(words), "{words}: {}", listed.detail);
        }
        front.turn("read it").await;
        let (_, outcome, text) = front.results().first().cloned().expect("the read");
        assert_eq!(outcome, "success", "{text}");
        front.close().await;
    });
    assert!(
        trust_store(&world)
            .project(&project_key(&world))
            .unwrap()
            .pinned_of(&TrustResourceKind::Hook)
            .is_empty(),
        "its pin is gone"
    );
    let (_, asked) = world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        front.close().await;
        ((), asked)
    });
    assert!(asked.is_empty(), "{asked:?}");
}

/// `cineris permissions trust`'s way (`Core::project_removed`,
/// `Core::trust_project_dropping`): a trusted project's approved hook that
/// is turned off and its approved extension whose folder is gone (the twin
/// of the hook's: `approve` kept a gone extension's pin, so trusting the
/// project again did not take it away) are each listed as removed, a hook
/// or an extension, and saying so; trusting the project with them takes
/// each approval away, and leaves the rest approved.
#[test]
fn trusting_a_project_again_takes_away_the_approvals_shown_as_removed() {
    let world = World::new("hook_trust_dropping", []);
    world.project_config(
        &[
            gate("\"read\"", ""),
            entry("lint", "turn.end", &this_binary("fail", &[]), ""),
        ]
        .concat(),
    );
    let _ = state(&world);
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    assert_eq!(hooks.len(), 2, "{hooks:?}");
    core.trust_project_with(&key, &[], &hooks).unwrap();
    let gone = Definition::of(TrustResourceKind::Extension, "tracker", ["gone"]);
    trust_store(&world)
        .approve(&project_key(&world), std::slice::from_ref(&gone), false)
        .unwrap();
    assert_eq!(
        core.project_removed(&key).unwrap().len(),
        1,
        "the extension alone"
    );

    world.project_config(
        &[
            gate("\"read\"", "enabled = false"),
            entry("lint", "turn.end", &this_binary("fail", &[]), ""),
        ]
        .concat(),
    );
    let removed = core.project_removed(&key).unwrap();
    let listed: Vec<(TrustResourceKind, &str)> = removed
        .iter()
        .map(|one| (one.kind.clone(), one.name.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (TrustResourceKind::Extension, "tracker"),
            (TrustResourceKind::Hook, "gate")
        ]
    );
    assert!(
        removed[0]
            .detail
            .contains("is no longer in its folder of extensions"),
        "{}",
        removed[0].detail
    );
    assert!(
        removed[0].detail.contains("What it gated is not known"),
        "{}",
        removed[0].detail
    );
    assert!(
        removed[1].detail.contains("turned off"),
        "{}",
        removed[1].detail
    );
    let (trusted, dropped) = core
        .trust_project_dropping(&key, &[], &[], &removed)
        .unwrap();
    assert!(trusted.was_trusted);
    assert_eq!(dropped, 2);
    assert!(core.project_removed(&key).unwrap().is_empty());
    let stored = trust_store(&world).project(&project_key(&world)).unwrap();
    let names: Vec<String> = stored
        .pinned_of(&TrustResourceKind::Hook)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["lint"], "the rest stays approved");
}

/// A trusted project's held gate (its approved gate changed, answered not
/// now) refuses every prompt; its trust taken away from outside
/// (`cineris permissions untrust` in another terminal), the reload's look
/// takes the project's hooks and their held gates off the session at once,
/// and says so once: the prompt goes through. Trust taken away is the
/// person's act, which no file of the project's can do, and an untrusted
/// project's hooks are nothing to its session (finding 5, and its bearing
/// on finding 4's held gates).
#[test]
fn a_projects_held_gates_leave_with_its_trust() {
    let world = World::new("hook_trust_untrusted_gates", [says("Done.")]);
    world.project_config(&gate("\"read\"", ""));
    let _ = state(&world);
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    core.trust_project_with(&key, &[], &hooks).unwrap();
    world.project_config(&gate("\"read\"", "observe = true"));
    let core = world.core();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        assert_eq!(asked.len(), 1);
        let ended = front.turn("held").await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        assert!(trust_store(&world).revoke(&project_key(&world)).unwrap());
        let reloaded = front
            .op(Op::ConfigReload(ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            }))
            .await;
        assert!(reloaded.is_ok(), "{reloaded:?}");
        let ended = front.turn("free").await;
        assert_eq!(ended.outcome, TurnOutcome::Completed, "{ended:?}");
        let said = front
            .notices()
            .into_iter()
            .filter(|notice| notice.contains("no longer trusted, so its hooks no longer run"))
            .count();
        assert_eq!(said, 1, "{:?}", front.notices());
        front.close().await;
    });
}

/// 06-permissions-and-sandbox ("A session runs a project's hook only while
/// the project is trusted") and 08-configuration ("Lowered trust takes at
/// once everything trust gave … a hook"): a project's approved hook command
/// starts its program while the project is trusted; its trust taken away
/// from outside, seen by the reload's look, it is no longer the session's.
/// Before the fix it went on starting its program until the session closed
/// (the review of step 28, finding 5).
#[test]
fn sys_a_projects_hook_command_stops_with_its_trust() {
    let world = World::new("hook_trust_untrust", [says("Done.")]);
    let program = std::env::current_exe().expect("the test binary has a path");
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"allow\"\n",
        program.display()
    ));
    world.project_config(&entry(
        "lint",
        "tool.after",
        &this_binary("fail", &[]),
        "tools = [\"edit\"]",
    ));
    let _ = state(&world);
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    core.trust_project_with(&key, &[], &hooks).unwrap();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        let before = core
            .run_hook_command(&front.session, "lint", for_edit(&["src/a.rs"]))
            .await;
        let Ok(HookOutcome::Failed(why)) = &before else {
            panic!("trusted, its program ran, and failed as its mode says: {before:?}");
        };
        assert!(!why.contains("never asks"), "{why}");
        assert!(trust_store(&world).revoke(&project_key(&world)).unwrap());
        let reloaded = front
            .op(Op::ConfigReload(ConfigReloadParams {
                changed_only: true,
                ..ConfigReloadParams::default()
            }))
            .await;
        assert!(reloaded.is_ok(), "{reloaded:?}");
        let after = core
            .run_hook_command(&front.session, "lint", for_edit(&["src/a.rs"]))
            .await;
        assert!(
            after.is_err(),
            "untrusted, its hook is no longer the session's: {after:?}"
        );
        front.close().await;
    });
}

/// A user's `tool.before` hook that takes `{path}` is handed a file the
/// engine asks about for what it is (here `.env`, which looks like it holds
/// secrets) only after the person's yes to the call's question, and before
/// the call runs (the review of step 28, finding 14, and the owner's
/// choice): while the question is open it has not run; after a yes it runs
/// with the file and the read runs; after a no it never runs; and after a
/// yes it can still refuse the call, which then reads nothing. A file asked
/// about for no such reason is handed to it before anything, as before.
/// Before the fix it ran with `.env` before the question was put.
#[test]
fn sys_a_users_hook_is_handed_a_secret_file_only_after_the_yes() {
    let read = |path: &str| calls(&[("c1", "read", json!({"path": path}))]);
    let world = World::new(
        "hook_trust_secret",
        [
            read(".env"),
            says("Done."),
            read(".env"),
            says("Done."),
            read(".env"),
            says("Done."),
            read("a.txt"),
            says("Done."),
        ],
    );
    world.put(".env", "KEY=1\n");
    world.put("a.txt", "plain\n");
    let program = std::env::current_exe().expect("the test binary has a path");
    let report = world.dir.join("report.json");
    let scanner =
        crate::hook_commands::script(&world.dir.join("hooks"), "scanner", Some(&report), "{}");
    let hook = |mode: &str| {
        let command = if mode == "report" {
            vec![scanner.display().to_string(), "{path}".to_owned()]
        } else {
            this_binary("fail", &["{path}"])
        };
        entry("scanner", "tool.before", &command, "tools = [\"read\"]")
    };
    let allow = format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"allow\"\n",
        program.display()
    );
    // An observer that takes `{path}` too: it decides nothing, so it does
    // not run for a call whose file waits for the yes (the review of step
    // 28's fixes: no test held it back).
    let watched = world.dir.join("watched.json");
    let watcher =
        crate::hook_commands::script(&world.dir.join("hooks"), "watcher", Some(&watched), "{}");
    let watcher = entry(
        "watcher",
        "tool.before",
        &[watcher.display().to_string(), "{path}".to_owned()],
        "tools = [\"read\"]\nobserve = true",
    );
    world.user_config(&format!("{allow}{}{watcher}", hook("report")));
    let core = world.core();
    let seen_at_the_question = std::rc::Rc::new(std::cell::Cell::new(None));
    world.run(async {
        let front = crate::extensions_found::start(&world, &core).await;
        // A yes: not run while it is asked, run with the file after it.
        let seen = std::rc::Rc::clone(&seen_at_the_question);
        let at_the_question = report.clone();
        front.answering(crate::Answering::with(move |_| {
            seen.set(Some(at_the_question.exists()));
            cineris_protocol::methods::ApprovalChoice::AllowOnce
        }));
        front.turn("read the secret").await;
        assert_eq!(
            seen_at_the_question.get(),
            Some(false),
            "not before the answer"
        );
        let reported = std::fs::read_to_string(&report).expect("it ran after the yes");
        assert!(reported.contains(".env"), "{reported}");
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "success", "{text}");
        std::fs::remove_file(&report).unwrap();

        // A no: it never runs.
        front.answering(crate::Answering::deny_all());
        front.turn("read it again").await;
        assert!(!report.exists(), "a no runs it no more");
        let (_, outcome, _) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "denied");
        // The close waits for every run the session started: an observer
        // started for either read would have written by now.
        front.close().await;
    });
    assert!(
        !watched.exists(),
        "an observer was handed the secret file: {:?}",
        std::fs::read_to_string(&watched)
    );

    // A yes, and the hook refuses after it: the call reads nothing.
    world.user_config(&format!("{allow}{}", hook("fail")));
    let core = world.core();
    world.run(async {
        let front = crate::extensions_found::start(&world, &core).await;
        front.answering(crate::Answering::allow_all());
        front.turn("read the secret").await;
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("`scanner`"), "{text}");
        assert!(!text.contains("KEY=1"), "{text}");
        front.close().await;
    });

    // A file asked about for nothing: handed to it at once, unasked, and
    // to the observer.
    world.user_config(&format!("{allow}{}{watcher}", hook("report")));
    let core = world.core();
    world.run(async {
        let front = crate::extensions_found::start(&world, &core).await;
        front.turn("read the plain file").await;
        let reported = std::fs::read_to_string(&report).expect("it ran");
        assert!(reported.contains("a.txt"), "{reported}");
        front.close().await;
    });
    let observed = std::fs::read_to_string(&watched).expect("the observer ran");
    assert!(observed.contains("a.txt"), "{observed}");
}

/// A trust store that cannot be read says nothing of the project, and may
/// be read again (a newer build's file, its permissions, a read while
/// another process replaces it): a reload that cannot read it holds the
/// project's hooks back, its approved gate failing on what it gates, and
/// one that reads it again puts them back as they were; only a store read
/// that no longer trusts the project takes them away for the session (the
/// review of step 28's fixes: an unreadable store withdrew them, every
/// gate with them, for the rest of the session).
#[test]
fn sys_an_unreadable_trust_store_holds_a_projects_gates_until_it_is_read() {
    let read = || calls(&[("c1", "read", json!({"path": "a.txt"}))]);
    let world = World::new(
        "hook_trust_unread",
        [
            read(),
            says("Done."),
            read(),
            says("Done."),
            read(),
            says("Done."),
            read(),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    // A gate that lets the read go: it runs, writes its report and answers
    // no opinion. A project's hook never asks, so a rule allows its start.
    let report = world.dir.join("gate.json");
    let gate = crate::hook_commands::script(&world.dir.join("hooks"), "gate", Some(&report), "{}");
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{}'\naction = \"allow\"\n",
        gate.display()
    ));
    world.project_config(&entry(
        "gate",
        "tool.before",
        &[gate.display().to_string()],
        "tools = [\"read\"]",
    ));
    let trust_file = state(&world).trust_file();
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    core.trust_project_with(&key, &[], &hooks).unwrap();
    let told = |front: &Front, words: &str| {
        front.drain();
        front
            .notices()
            .iter()
            .filter(|notice| notice.contains(words))
            .count()
    };
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        front.turn("read it").await;
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "success", "{text}");
        assert!(report.exists(), "the gate ran");

        // Unreadable: held back, the gate failing.
        let kept = std::fs::read_to_string(&trust_file).unwrap();
        std::fs::write(&trust_file, "version = 99\n").unwrap();
        assert!(reload(&front).await.is_ok(), "the reload answers");
        front.turn("read it").await;
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("the trust store cannot be read"), "{text}");
        assert_eq!(told(&front, "The trust store cannot be read"), 1);

        // Read again, trusting it: as it was.
        std::fs::write(&trust_file, kept).unwrap();
        assert!(reload(&front).await.is_ok(), "the reload answers");
        std::fs::remove_file(&report).unwrap();
        front.turn("read it").await;
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "success", "{text}");
        assert!(report.exists(), "the gate ran again");
        assert_eq!(told(&front, "The trust store can be read again"), 1);

        // Read, and no longer trusting it: withdrawn.
        assert!(trust_store(&world).revoke(&project_key(&world)).unwrap());
        assert!(reload(&front).await.is_ok(), "the reload answers");
        std::fs::remove_file(&report).unwrap();
        front.turn("read it").await;
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "success", "{text}");
        assert!(!report.exists(), "withdrawn, the gate ran no more");
        front.close().await;
    });
}

/// A project's hook whose program is running when the project's trust is
/// taken away is stopped by the reload that sees it, its process with it,
/// though its own time is far off; the call it gated is refused, and the
/// turn goes on (the review of step 28's fixes: nothing tested that runs
/// already going stop). Its time is on the session's clock, which does not
/// move: only the withdrawal ends it.
#[test]
fn sys_a_projects_hook_run_going_on_stops_when_its_trust_is_taken_away() {
    const STUCK: std::time::Duration = std::time::Duration::from_secs(50);
    let world = World::new(
        "hook_trust_run_stops",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            says("Done."),
        ],
    );
    world.put("a.txt", "the file\n");
    let stuck = crate::hook_commands::Lingering::new(&world.dir.join("hooks"), "stuck");
    let program = std::env::current_exe().expect("the test binary has a path");
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = '{} *'\naction = \"allow\"\n\
         [[permissions.rules]]\neffect = \"exec\"\nmatch = '{}'\naction = \"allow\"\n",
        program.display(),
        stuck.command[0]
    ));
    world.project_config(&entry(
        "stuck",
        "tool.before",
        &stuck.command,
        "tools = [\"read\"]\ntimeout = \"50s\"",
    ));
    let _ = state(&world);
    let core = world.core();
    let key = core.project(&world.workspace).unwrap();
    let hooks = core.project_hook_commands(&key).unwrap();
    core.trust_project_with(&key, &[], &hooks).unwrap();
    world.run(async {
        let (front, asked) = open_answering(&world, &core, yes()).await;
        assert!(asked.is_empty(), "{asked:?}");
        let started = std::cell::Cell::new(None);
        let mut turn = std::pin::pin!(front.turn("read it"));
        let mut untrust = std::pin::pin!(async {
            // The loop's wait for it, and its run's own time: its program
            // has started.
            while world
                .timer
                .running()
                .iter()
                .filter(|wait| **wait == STUCK)
                .count()
                < 2
            {
                tokio::task::yield_now().await;
            }
            started.set(stuck.started());
            assert!(trust_store(&world).revoke(&project_key(&world)).unwrap());
            let reloaded = front
                .op(Op::ConfigReload(ConfigReloadParams {
                    changed_only: true,
                    ..ConfigReloadParams::default()
                }))
                .await;
            assert!(reloaded.is_ok(), "{reloaded:?}");
        });
        let mut untrusted = false;
        let ended = std::future::poll_fn(|context| {
            if !untrusted {
                untrusted = untrust.as_mut().poll(context).is_ready();
            }
            turn.as_mut().poll(context)
        })
        .await;
        assert!(untrusted, "the trust was taken away while it ran");
        assert_eq!(ended.outcome, TurnOutcome::Completed, "{ended:?}");
        let (_, outcome, text) = front.results().last().cloned().expect("the read");
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("`stuck`"), "{text}");
        front.close().await;
        crate::hook_commands::Lingering::assert_gone(
            started.get(),
            "the project's hook ran on after its trust was taken away",
        );
    });
}
