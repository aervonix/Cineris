//! An extension's hooks (step 28): the functions a loaded extension's
//! script registered with `cineris.on`, on the loop's points beside the hook
//! commands and on the session's own events (`docs/steps/28/answered.md`,
//! points 2, 6 and 8; `docs/architecture/12-extensions.md`, "Tier 3:
//! hooks"). End to end, through the core: the user's extension found,
//! judged by its pin and taken from the registration cache, as
//! `extensions_found` has it, each rule's story the turns of one session.
//! None starts a process: a hook's `cineris.exec` is a phase of the hook
//! commands' chained `sys_` test.
//!
//! The rules are parts of three tests, each with a world of its own, so
//! that a test's process starts once for many of them (a test process's
//! first core, session and turn cost a hosted Windows runner about 8 ms
//! more than the next ones). A rule that leaves a runtime start or a
//! thread running past its session, or counts the starts let go or the
//! threads left running, is a test of its own: what one rule left in the
//! process would be counted with another's. The parts that count the
//! runtimes the process has made come first in their tests.

use std::path::PathBuf;
use std::time::Duration;

use cineris_core::ClientInfo;
use cineris_protocol::event::{Event, Notice, TurnOutcome};
use cineris_protocol::methods::{ConfigReloadParams, Op, OpResult, TrustResponse, TurnStartParams};
use cineris_protocol::{Part, SessionId};
use cineris_testkit::{Hold, Response};
use serde_json::{Value, json};

use crate::extensions_found::{
    cache, clock, definition, make_link, open_as, pin, start, state, users,
};
use crate::{Front, Outcome, World, calls, poll_once, says, settle};

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

/// [`write`], handed to the core as approved (`CoreOptions::extensions`),
/// so that its hooks are on their events as the session opens, with no
/// turn to wait for them.
fn handed(world: &World, capabilities: &str, script: &str) -> cineris_core::ext::ApprovedExtension {
    let folder = write(world, capabilities, script);
    let snapshot = std::sync::Arc::new(cineris_ext::folder::Snapshot::read(&folder).unwrap());
    let (manifest, registration) = world
        .run(cineris_ext::engine::register(
            std::sync::Arc::clone(&snapshot),
            &world.timer,
        ))
        .unwrap();
    cineris_core::ext::ApprovedExtension {
        source: cineris_ext::source::Source::User,
        folder,
        hash: snapshot.hash(),
        manifest,
        registration,
    }
}

/// A prompt of `text` to `session`, as `turn/start` takes it.
fn prompt_op(session: &SessionId, text: &str) -> Op {
    Op::TurnStart(TurnStartParams {
        session_id: session.clone(),
        input: vec![Part::text(text)],
        delivery: None,
        command: None,
    })
}

/// Writes the user's extension `probe`, with these capabilities and
/// `script` as its main module. Its folder.
fn write(world: &World, capabilities: &str, script: &str) -> PathBuf {
    let folder = users(world);
    write_in(&folder, capabilities, script);
    folder
}

/// [`write`], into `folder`.
fn write_in(folder: &std::path::Path, capabilities: &str, script: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain \
             = \"main.js\"\n[capabilities]\n{capabilities}\n"
        ),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), script).unwrap();
}

/// [`write`], approved as it is: pinned, and what it registers cached.
fn approved(world: &World, capabilities: &str, script: &str) -> PathBuf {
    let folder = write(world, capabilities, script);
    pin(world, &folder);
    cache(world, &folder);
    folder
}

/// A turn whose model calls `tool` once with `arguments`, then ends.
fn one_call(id: &str, tool: &str, arguments: Value) -> [Response; 2] {
    [calls(&[(id, tool, arguments)]), says("Done.")]
}

/// How the session's last call ended, and what the model was shown.
fn last(front: &Front) -> (Outcome, String) {
    let results = front.results();
    let (_, outcome, text) = results.last().expect("a call was made").clone();
    (outcome, text)
}

/// Whether one of `front`'s notices says `words`.
fn told(front: &Front, words: &str) -> bool {
    front.notices().iter().any(|notice| notice.contains(words))
}

/// How many of `front`'s notices say `words`.
fn told_times(front: &Front, words: &str) -> usize {
    front
        .notices()
        .iter()
        .filter(|notice| notice.contains(words))
        .count()
}

/// Why the last prompt given back to `front`'s client was refused.
fn returned(front: &Front) -> String {
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

/// The user's configuration turns the extension `probe` off, and the core
/// reads it again: an operation of the core's own, which no hook gates.
async fn turn_it_off(world: &World, front: &Front) {
    world.user_config("[extensions.probe]\nenabled = false\n");
    let reloaded = front
        .op(Op::ConfigReload(ConfigReloadParams::default()))
        .await;
    assert!(
        matches!(reloaded, Ok(OpResult::ConfigReload(_))),
        "{reloaded:?}"
    );
}

/// Runs `rule`, one of the rules a folded test checks, each with a world of
/// its own: a rule that fails names itself after the panic's message, so
/// that the test's failure says which rule broke.
fn part(name: &str, rule: fn()) {
    struct Named<'a>(&'a str);
    impl Drop for Named<'_> {
        #[expect(
            clippy::print_stderr,
            reason = "a failing rule names itself where the test runner shows a failure"
        )]
        fn drop(&mut self) {
            if std::thread::panicking() {
                eprintln!("the rule that failed: `{}`", self.0);
            }
        }
    }
    let _named = Named(name);
    rule();
}

/// Runs each rule named, a function of this file, as a [`part`] of one
/// test, in order.
macro_rules! parts {
    ($($rule:ident),+ $(,)?) => {
        $(part(stringify!($rule), $rule);)+
    };
}

/// An extension's hooks on their events, as the loop reads what they
/// answer, and none in a turn already stopped: each part below is a rule,
/// with a world, a core and sessions of its own. The two parts that count
/// the runtimes this process has made come first, before any other part
/// has made one.
#[test]
fn an_extensions_hooks_are_on_their_events_and_the_loop_reads_them() {
    parts![
        an_extensions_hooks_runs_are_its_own_and_bounded,
        a_session_end_hook_starts_its_runtime_as_the_session_closes,
        an_extensions_hook_is_on_its_event_and_its_answer_is_read,
        an_extensions_hook_waiting_for_its_runtime_spends_its_own_time,
        a_hooks_host_calls_put_no_question,
        an_extensions_hooks_are_on_their_events_only_while_it_stands_approved,
        an_extensions_hooks_run_as_its_session_opens_and_closes,
        a_hooks_time_at_its_point_runs_from_when_it_was_asked,
        a_withdrawn_extensions_runs_stop,
        a_hooks_run_let_go_while_its_runtime_starts_ends_at_once,
        after_a_stop_of_the_turn_its_extensions_hooks_run_nothing,
        a_hook_that_finds_its_turn_stopped_is_no_stop_of_its_own,
        a_turn_the_loops_own_stop_ends_refuses_its_hooks,
    ];
}

/// An extension's gates hold, failing, while it does not stand approved as
/// it is found, and the way out of them stays open: each part below is a
/// rule, with a world, a core and sessions of its own. The part that counts
/// the runtimes this process has made comes first, before any other part
/// has made one.
#[test]
fn an_extensions_gates_hold_while_it_does_not_stand_approved() {
    parts![
        an_extensions_gates_are_held_while_it_does_not_stand_approved,
        a_changed_extensions_gates_hold_and_the_way_out_stays_open,
        an_extension_never_registered_holds_the_gates_its_manifest_lists,
        a_removed_extensions_gates_hold_until_it_is_turned_off,
        an_extension_that_cannot_be_loaded_holds_its_gates,
        a_changed_extension_whose_registration_is_not_known_holds_both_gates,
        a_session_whose_finding_failed_starts_no_turn,
        under_trust_for_the_run_a_changed_projects_gate_names_no_question,
    ];
}

/// While the pin store, the trust store or the registration cache cannot be
/// read, the gates known of an extension hold, and no more: each part below
/// is a rule, with a world of its own, so that no store one part breaks on
/// purpose is another's.
#[test]
fn a_store_that_cannot_be_read_holds_the_gates_known_of_an_extension() {
    parts![
        an_unreadable_pin_store_holds_the_gates_of_the_users_extensions,
        an_unreadable_pin_store_falls_back_to_the_manifest_where_the_cache_has_none,
        a_same_named_cache_entry_does_not_lift_a_gate_under_an_unreadable_pin_store,
        a_gate_held_from_the_cache_is_kept_when_the_pin_store_breaks,
        an_extension_unpinned_gets_no_gate_back_when_the_pin_store_breaks,
        an_unreadable_trust_store_holds_the_gates_of_a_projects_extensions,
        an_unreadable_trust_store_gates_nothing_of_a_project_the_session_never_loaded,
        an_extension_approved_for_the_run_keeps_its_gates_while_the_store_is_unread,
        an_extension_never_registered_keeps_its_gates_while_the_trust_store_is_unread,
        a_gate_held_from_the_cache_is_kept_when_the_trust_store_breaks,
        a_changed_extension_with_its_cache_set_aside_holds_both_gates,
        a_changed_extension_the_session_saw_register_no_gate_holds_none,
    ];
}

/// Hooks on `list` that answer as the call's path says, an observer of
/// them, and a `tool.after` hook on `read`.
const ANSWERS: &str = "cineris.on('tool.before', { tools: ['list'] }, (e) => {\n\
       switch (e.arguments.path) {\n\
         case 'deny': return { decision: 'deny', reason: 'saw ' + e.event + ' of ' + e.tool + \
       ' ' + JSON.stringify(e.arguments) + ' in ' + e.turn_id + ', a plan of ' + typeof e.plan + \
       ', a session of ' + typeof e.session_id + ', a workspace of ' + typeof e.workspace };\n\
         case 'quiet': return undefined;\n\
         case 'null': return null;\n\
         case 'bad': return { decision: 'maybe' };\n\
         case 'throw': throw new Error('went wrong');\n\
         default: return {};\n\
       }\n\
     });\n\
     cineris.on('tool.before', { tools: ['list'], observe: true }, () => \
       ({ decision: 'deny', reason: 'an observer' }));\n\
     cineris.on('tool.after', { tools: ['read'] }, (e) => e.arguments.path === 'big.md'\n\
       ? { context: 'x'.repeat(70 * 1024) }\n\
       : { context: 'noted ' + e.result.outcome + ' of ' + e.tool });\n";

/// An extension's hooks are on their events, and the loop reads what they
/// answer as it reads a hook command's, turns of one session:
/// - a hook is given the event's object, and its `deny` refuses the call,
///   its reason under its name, `probe/hook.tool.before`;
/// - `undefined` and `null` are no opinion: the call runs;
/// - an answer outside its event's contract, and a script that throws, are
///   failures, which refuse where a hook gates;
/// - an observer on the same call, the extension's second hook on the
///   event (`probe/hook.tool.before.2`), is run and not heard: its `deny`
///   refuses nothing;
/// - a `tool.after` hook's context is a note on the result, and an answer
///   larger than 64 KiB is a failure, shown to the user, that adds nothing.
fn an_extensions_hook_is_on_its_event_and_its_answer_is_read() {
    let mut responses = Vec::new();
    for (id, path) in [
        ("c1", "deny"),
        ("c2", "quiet"),
        ("c3", "null"),
        ("c4", "bad"),
        ("c5", "throw"),
    ] {
        responses.extend(one_call(id, "list", json!({"path": path})));
    }
    responses.extend(one_call("c6", "read", json!({"path": "notes.md"})));
    responses.extend(one_call("c7", "read", json!({"path": "big.md"})));
    let world = World::new("ext_hooks_answers", responses);
    for folder in ["deny", "quiet", "null", "bad", "throw"] {
        world.put(&format!("{folder}/a.txt"), "a\n");
    }
    world.put("notes.md", "notes\n");
    world.put("big.md", "big\n");
    approved(&world, "hooks = [\"tool.before\", \"tool.after\"]", ANSWERS);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;

        front.turn("deny it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(
            text.contains(
                "the hook `probe/hook.tool.before` refused it: saw tool.before of list \
                 {\"path\":\"deny\"} in t_1, a plan of string, a session of string, a workspace \
                 of string"
            ),
            "{text}"
        );
        assert!(
            !text.contains("an observer"),
            "an observer was heard: {text}"
        );

        for quiet in ["quiet", "null"] {
            front.turn(quiet).await;
            let (outcome, text) = last(&front);
            assert_ne!(outcome, "denied", "{quiet}: {text}");
        }

        front.turn("bad").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(
            text.contains("`probe/hook.tool.before` failed, and a hook that fails refuses")
                && text.contains("`maybe`"),
            "{text}"
        );

        front.turn("throw").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("went wrong"), "{text}");

        front.turn("read").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        assert!(
            text.contains("`probe/hook.tool.after`") && text.contains("noted success of read"),
            "{text}"
        );

        front.turn("read the big one").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        assert!(!text.contains("xxxxxxxx"), "a note past the cap: {text}");
        assert!(
            told(&front, "`probe/hook.tool.after` failed: its answer is"),
            "{:?}",
            front.notices()
        );
        assert!(told(&front, "at most 64 KiB"), "{:?}", front.notices());
        front.close().await;
    });
}

/// Two hooks on `list`: the first computes until it is stopped, the second
/// says it ran and refuses.
const QUEUED: &str = "cineris.on('tool.before', { tools: ['list'], timeout: 3000 }, (e) => {\n\
       if (e.arguments.path !== 'spin') return undefined;\n\
       cineris.notify('first started');\n\
       for (;;) {}\n\
     });\n\
     cineris.on('tool.before', { tools: ['list'], timeout: 1000 }, () => {\n\
       cineris.notify('second ran');\n\
       return { decision: 'deny', reason: 'the second' };\n\
     });\n";

/// A hook's time starts when it is asked (`docs/architecture/12-extensions.md`,
/// "Tier 3: hooks"), turns of one session. The extension's two hooks on one
/// call wait their turns in its runtime, one call at a time: the second,
/// behind the first, which computes, is withdrawn when its second is up,
/// before it starts, and never runs; the first is stopped at its own time,
/// and its runtime with it; each refuses the call. On the next call both
/// run, the second heard, in a runtime made again.
fn an_extensions_hook_waiting_for_its_runtime_spends_its_own_time() {
    let mut responses = Vec::new();
    responses.extend(one_call("c1", "list", json!({"path": "spin"})));
    responses.extend(one_call("c2", "list", json!({"path": "."})));
    let world = World::new("ext_hooks_queued", responses);
    world.put("spin/a.txt", "a\n");
    approved(&world, "hooks = [\"tool.before\"]", QUEUED);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.prompt("spin").await;
        // The first runs: the second waits behind it, its time running.
        front
            .until(|_| told(&front, "The extension `probe` says: first started"))
            .await;
        world.timer.advance(Duration::from_secs(1));
        settle().await;
        world.timer.advance(Duration::from_secs(2));
        front.turns(1).await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        // Out of time, as the loop or the runtime says it, whichever sees
        // its time up first: both run from when the hook was asked.
        let out_of_time = |second: &str, ms: &str| {
            text.contains(&format!("did not answer within {second} s"))
                || text.contains(&format!("did not answer within its {ms} ms"))
        };
        assert!(
            text.contains("`probe/hook.tool.before.2` failed") && out_of_time("1", "1000"),
            "{text}"
        );
        assert!(
            text.contains("`probe/hook.tool.before` failed") && out_of_time("3", "3000"),
            "{text}"
        );
        assert!(
            !told(&front, "second ran"),
            "the second ran after its time: {:?}",
            front.notices()
        );

        front.turn("list it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("the second"), "{text}");
        assert!(told(&front, "second ran"), "{:?}", front.notices());
        front.close().await;
    });
}

/// A hook on `list` that tries each host call a hook might, and says what
/// became of each.
const TRIES: &str = "cineris.on('tool.before', { tools: ['list'] }, async () => {\n\
       const tried = [];\n\
       const attempt = async (what, call) => {\n\
         try { const got = await call(); tried.push(what + ': ' + \
       (typeof got === 'string' ? got.trim() : 'done')); }\n\
         catch (err) { tried.push(what + ': ' + String(err)); }\n\
       };\n\
       await attempt('fetch', () => cineris.http.fetch('https://api.example.com/x'));\n\
       await attempt('ask', () => cineris.ask('may I?'));\n\
       await attempt('write', () => cineris.fs.write('out.txt', 'x'));\n\
       await attempt('read', () => cineris.fs.read('notes.md'));\n\
       return { decision: 'deny', reason: tried.join(' | ') };\n\
     });\n";

/// What a hook's host calls may do (`docs/steps/28/answered.md`, point 2,
/// "Inside a hook"), at `supervised`: a call the engine would ask about (a
/// fetch) is refused, saying that a hook may not put a question and naming
/// the rule that would allow it, and nobody is asked; a question to the
/// user, and a write, are refused; a read the engine allows runs. In the
/// next session, a rule about the extension (`deny ext probe/*`) refuses
/// its hook's read too: a hook's host calls are decided under its name.
fn a_hooks_host_calls_put_no_question() {
    let mut responses = Vec::new();
    responses.extend(one_call("c1", "list", json!({"path": "."})));
    responses.extend(one_call("c2", "list", json!({"path": "."})));
    let world = World::new("ext_hooks_no_question", responses);
    world.put("notes.md", "the notes\n");
    approved(
        &world,
        "net = [\"api.example.com\"]\nread = [\"**\"]\nwrite = [\"**\"]\nhooks = [\"tool.before\"]",
        TRIES,
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("list it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        let fetch = text
            .split(" | ")
            .find(|part| part.contains("fetch: "))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(
            fetch.contains("a hook may not put a question")
                && fetch.contains("effect = \"network\"")
                && fetch.contains("api.example.com"),
            "{fetch}"
        );
        assert!(
            text.contains("ask: Error: a hook cannot put a question"),
            "{text}"
        );
        assert!(
            text.contains("write: Error: `cineris.fs.write` works only in a tool call"),
            "{text}"
        );
        assert!(text.contains("read: the notes"), "{text}");
        assert!(front.approvals().is_empty(), "{:?}", front.approvals());
        front.close().await;
    });
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"deny\"\n",
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("list it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("read: Error: not allowed"), "{text}");
        assert!(!text.contains("the notes"), "{text}");
        assert!(front.approvals().is_empty(), "{:?}", front.approvals());
        front.close().await;
    });
}

/// A tool whose write nobody can be asked about, and two `tool.after`
/// hooks: one that reads, one that only answers.
const STOPPED: &str = "cineris.tool({ name: 'ci', run: async () => { \
       try { await cineris.fs.write('.env', 'TOKEN=1\\n') } catch (e) { } \
       return 'went on' } });\n\
     cineris.on('tool.after', async () => ({ context: 'read ' + \
       (await cineris.fs.read('notes.md')).trim() }));\n\
     cineris.on('tool.after', () => ({ context: 'ran in a stopped turn' }));\n";

/// After a stop of the turn from a call's run, the turn's hooks of the
/// extension run nothing (the maintainers, for hooks: a stopped turn's
/// hooks do not start; `docs/architecture/03-agent-loop.md`, "A stop from a call's run").
/// With `fail_on_denial`, the extension's tool's write nobody can be asked
/// about stops the turn through the policy; the call's `tool.after` hooks,
/// in the same turn, start nothing: neither the one that would read nor the
/// one that would only answer adds a note, and the user is told each was
/// not started. Only the policy's stop of the turn says so: the call's
/// result alone ends no hook.
fn after_a_stop_of_the_turn_its_extensions_hooks_run_nothing() {
    let world = World::new(
        "ext_hooks_stopped",
        one_call(
            "c1",
            "tool_call",
            json!({"tool": "probe/ci", "arguments": {}}),
        ),
    );
    world.put("notes.md", "the notes\n");
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    approved(
        &world,
        "write = [\".env\"]\nread = [\"**\"]\nhooks = [\"tool.after\"]",
        STOPPED,
    );
    let core = world.core_with(|options| options.fail_on_denial = true);
    world.run(async {
        let (front, _) = open_as(
            &world,
            &core,
            ClientInfo::headless(),
            TrustResponse::default(),
        )
        .await;
        let end = front.turn("set up CI").await;
        assert_eq!(end.outcome, TurnOutcome::Denied, "{end:?}");
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("stopped"), "{text}");
        assert!(!text.contains("read the notes"), "a hook read: {text}");
        assert!(
            !text.contains("ran in a stopped turn"),
            "a hook ran: {text}"
        );
        for name in ["`probe/hook.tool.after`", "`probe/hook.tool.after.2`"] {
            assert!(
                front.notices().iter().any(|notice| notice.contains(name)
                    && notice.contains("was not started: its turn was stopped")),
                "{name}: {:?}",
                front.notices()
            );
        }
        front.close().await;
    });
    assert_eq!(world.read(".env"), None);
}

/// A hook that would tell it ran, on an event no turn reaches, and a
/// waited-for `turn.end` hook.
const LET_GO: &str = "cineris.on('compaction.before', () => { cineris.notify('ran after all'); });\n\
     cineris.on('turn.end', () => undefined);\n";

/// The run of an extension's hook is its own, two sessions of one world:
/// - one let go before it is first polled runs nothing (the seam's
///   contract: dropped, its future stops the hook, polled or not): no
///   runtime is made for it; and one let go before another run of the
///   extension's, which then makes its runtime and runs, is withdrawn: its
///   function never runs ahead of the other in the runtime's queue;
/// - a waited-for `turn.end` hook has at most four runs going at once, as a
///   hook command has, and a fifth is not started, and says so.
fn an_extensions_hooks_runs_are_its_own_and_bounded() {
    let world = World::new("ext_hooks_let_go", Vec::<Response>::new());
    let extension = handed(
        &world,
        "hooks = [\"compaction.before\", \"turn.end\"]",
        LET_GO,
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = start(&world, &core).await;
        core.let_go_of_a_hook_run(&front.session, "probe/hook.compaction.before")
            .unwrap();
        front.close().await;
        front.drain();
        assert!(!told(&front, "ran after all"), "{:?}", front.notices());
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made);
    world.run(async {
        let front = start(&world, &core).await;
        // Let go, then another run, which the runtime takes after it if it
        // was not withdrawn.
        core.let_go_of_a_hook_run(&front.session, "probe/hook.compaction.before")
            .unwrap();
        let said = core
            .hook_runs_at_its_point(&front.session, "probe/hook.turn.end", 1)
            .await
            .unwrap();
        assert_eq!(said, [None]);
        let said = core
            .hook_runs_at_its_point(&front.session, "probe/hook.turn.end", 5)
            .await
            .unwrap();
        let (four, fifth) = said.split_at(4);
        assert!(four.iter().all(Option::is_none), "{said:?}");
        assert!(
            fifth[0]
                .as_deref()
                .is_some_and(|why| why.contains("4 runs of it are going on")),
            "{said:?}"
        );
        front.close().await;
        front.drain();
        assert!(!told(&front, "ran after all"), "{:?}", front.notices());
    });
}

/// A `tool.after` hook on `read` that notes it saw the call.
const NOTES: &str =
    "cineris.on('tool.after', { tools: ['read'] }, () => ({ context: 'the probe saw it' }));\n";

/// An extension's hooks are its own (`docs/steps/28/answered.md`, point 5),
/// turns of one session: on their event only while the extension is loaded
/// as it was approved. Not approved, its hook adds nothing; approved during
/// the session, it is loaded at the next turn's start, and its hook adds
/// its note; changed then, it is withdrawn, and its hook with it.
fn an_extensions_hooks_are_on_their_events_only_while_it_stands_approved() {
    let mut responses = Vec::new();
    for id in ["c1", "c2", "c3"] {
        responses.extend(one_call(id, "read", json!({"path": "notes.md"})));
    }
    let world = World::new("ext_hooks_approved", responses);
    world.put("notes.md", "notes\n");
    let folder = write(&world, "hooks = [\"tool.after\"]", NOTES);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("read it").await;
        assert!(
            !last(&front).1.contains("the probe saw it"),
            "{:?}",
            last(&front)
        );
        pin(&world, &folder);
        front.turn("read it").await;
        assert!(
            last(&front).1.contains("the probe saw it"),
            "{:?}",
            last(&front)
        );
        write(
            &world,
            "hooks = [\"tool.after\"]",
            &format!("// changed\n{NOTES}"),
        );
        front.turn("read it").await;
        assert!(
            !last(&front).1.contains("the probe saw it"),
            "{:?}",
            last(&front)
        );
        assert!(told(&front, "is withdrawn"), "{:?}", front.notices());
        front.close().await;
    });
}

/// A tool that keeps a value of the session's, a `tool.after` hook on it
/// that reads it, two `session.start` hooks (one that adds context, one
/// that computes until its time is up) and a `session.end` hook that keeps a
/// value of the project's.
const SESSION: &str = "cineris.tool({ name: 'keep', run: async () => { \
       await cineris.state.set('kept', 'by the tool'); return 'kept'; } });\n\
     cineris.on('session.start', { timeout: 20000 }, (e) => \
       ({ context: 'opened as ' + e.event + ' of a session ' + typeof e.session_id }));\n\
     cineris.on('session.start', { timeout: 5000 }, () => { for (;;) {} });\n\
     cineris.on('tool.after', { tools: ['probe/keep'] }, async () => \
       ({ context: 'the hook reads ' + await cineris.state.get('kept') }));\n\
     cineris.on('session.end', async () => { \
       await cineris.state.set('ended', 'yes', { scope: 'project' }); });\n";

/// An extension's hooks on the session's own events (`docs/steps/28/answered.md`,
/// point 6), and its values, turns of one session:
/// - its `session.start` hooks are heard before the first turn, which
///   waits for them, saying, once it has waited long, that it waits for
///   them (not for the extensions to be found); the one that answers gives
///   its context as a notice of the hook's in the turn's first request, and
///   the one that computes is stopped at its time;
/// - a hook's `cineris.state` is its extension's: its `tool.after` hook reads
///   the value its tool kept, and its `session.end` hook, run as the
///   session closes, keeps a value in the user's `probe`'s file of the
///   project's values.
fn an_extensions_hooks_run_as_its_session_opens_and_closes() {
    let world = World::new(
        "ext_hooks_session",
        one_call(
            "c1",
            "tool_call",
            json!({"tool": "probe/keep", "arguments": {}}),
        ),
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    approved(
        &world,
        "hooks = [\"session.start\", \"tool.after\", \"session.end\"]",
        SESSION,
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let mut prompt = std::pin::pin!(front.prompt("keep it"));
        // The turn waits, and says what for once it has waited long.
        let said = cineris_core::ext::WAIT_SAID_AFTER;
        // The turn's wait, and the second hook's time, on the session's
        // clock and the engine's.
        drive_until(prompt.as_mut(), || {
            let running = world.timer.running();
            running.contains(&said)
                && running
                    .iter()
                    .filter(|wait| **wait == Duration::from_secs(5))
                    .count()
                    >= 2
        })
        .await;
        world.timer.advance(said);
        drive_until(prompt.as_mut(), || {
            front.drain();
            told(&front, cineris_core::ext::WAITING_FOR_HOOKS)
        })
        .await;
        assert!(
            !told(&front, cineris_core::ext::WAITING),
            "{:?}",
            front.notices()
        );
        // The second hook's time is up: the turn starts.
        world.timer.advance(Duration::from_secs(5) - said);
        prompt.await;
        front.turns(1).await;
        let sent = format!("{:?}", world.provider.requests()[0]);
        assert!(
            sent.contains("`probe/hook.session.start`")
                && sent.contains("opened as session.start of a session string"),
            "{:?}",
            front.notices()
        );
        assert!(
            told(&front, "`probe/hook.session.start.2` failed"),
            "{:?}",
            front.notices()
        );
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        assert!(text.contains("the hook reads by the tool"), "{text}");
        front.close().await;
    });
    let kept: Vec<(String, String)> = crate::ext_state::project_files(&world)
        .iter()
        .map(|file| {
            (
                file.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect();
    assert!(
        kept.iter()
            .any(|(name, file)| name == "user-probe.json" && file.contains("\"ended\":\"yes\"")),
        "{kept:?}"
    );
}

/// An extension that cannot be found and registered fails its session's
/// finding when the work that finds them ends before it is done (as a panic
/// in it would end it): the first prompt, waiting for the registration, is
/// refused with why, and so is the next, rather than either waiting for
/// ever or starting a turn without a gate the extension may hold.
fn a_session_whose_finding_failed_starts_no_turn() {
    let world = World::new("ext_hooks_finding_failed", Vec::<Response>::new());
    // Pinned and not cached: registered as the session opens.
    let folder = write(&world, "hooks = [\"tool.before\"]", GATES);
    pin(&world, &folder);
    let core = world.core();
    let registering = Hold::new(cineris_core::ext::BEFORE_AN_EXTENSION_IS_REGISTERED);
    world.run(async {
        let front = start(&world, &core).await;
        let mut prompt = std::pin::pin!(front.op(prompt_op(&front.session, "list it")));
        registering.drive_until_reached(prompt.as_mut()).await;
        core.end_the_finding_of(&front.session).unwrap();
        registering.release();
        for refused in [
            prompt.await,
            front.op(prompt_op(&front.session, "again")).await,
        ] {
            let refused = refused.expect_err("a turn started");
            assert_eq!(
                refused.code,
                cineris_protocol::error::code::EXTENSIONS_NOT_SETTLED,
                "{refused:?}"
            );
            assert!(refused.message.starts_with("No turn starts"), "{refused:?}");
        }
        assert!(world.provider.requests().is_empty());
        front.close().await;
    });
}

/// A gating hook on `list` that has no opinion, and a `tool.after` hook on
/// `read` that adds a note.
const GATES: &str = "cineris.on('tool.before', { tools: ['list'] }, () => undefined);\n\
     cineris.on('tool.after', { tools: ['read'] }, () => ({ context: 'the probe saw it' }));\n";

/// An extension's gates do not lapse while it stands approved and is not
/// found as approved (`docs/steps/28/answered.md`, point 5; the early
/// review of the script hooks, M1), turns of one session, and of another:
/// - changed between sessions, its gating hook is held from the
///   registration its pin names, failing, naming the extension, why, the
///   command that approves it and the setting that turns it off, and no
///   runtime is made; the user is told once a session, in words;
/// - approved again at the next turn's start, its hook joins in place of
///   the held one;
/// - changed again during the session, it is withdrawn at the next turn's
///   start, and its gate is held, while its `tool.after` hook leaves with
///   it;
/// - unreadable as it is (a link in its folder), its gate is held all the
///   same, and said so;
/// - with its pin taken away, its approval goes, and its gate with it;
/// - in another session, turned off by the user (`enabled = false`), it
///   holds nothing, unreadable and pinned as it is.
fn an_extensions_gates_are_held_while_it_does_not_stand_approved() {
    let mut responses = Vec::new();
    responses.push(calls(&[
        ("c0", "read", json!({"path": "notes.md"})),
        ("c1", "list", json!({"path": "."})),
    ]));
    responses.push(says("Done."));
    responses.extend(one_call("c2", "list", json!({"path": "."})));
    responses.push(calls(&[
        ("c3", "list", json!({"path": "."})),
        ("c4", "read", json!({"path": "notes.md"})),
    ]));
    responses.push(says("Done."));
    for id in ["c5", "c6", "c7"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_held", responses);
    world.put("notes.md", "notes\n");
    let capabilities = "hooks = [\"tool.before\", \"tool.after\"]";
    let folder = approved(&world, capabilities, GATES);
    write(&world, capabilities, &format!("// the second\n{GATES}"));
    let core = world.core();
    let made = cineris_ext::engine::runtimes_made();
    let linked = world.run(async {
        let front = start(&world, &core).await;
        front.turn("changed since").await;
        // Held from the registration the cache keeps: on `list` alone, so
        // the `read` beside it runs.
        let results = front.results();
        let outcome_of = |tool: &str| {
            results
                .iter()
                .find(|(called, _, _)| called == tool)
                .map(|(_, outcome, text)| (outcome.clone(), text.clone()))
                .unwrap_or_else(|| panic!("no `{tool}`: {results:?}"))
        };
        let (outcome, text) = outcome_of("read");
        assert_eq!(
            outcome, "success",
            "the held gate's filter is its own: {text}"
        );
        let (outcome, text) = outcome_of("list");
        assert_eq!(outcome, "denied", "{text}");
        for words in [
            "`probe/hook.tool.before` failed",
            "the extension `probe`",
            "has changed since it was approved",
            "`cineris ext add",
            "`[extensions.probe] enabled = false`",
        ] {
            assert!(text.contains(words), "{words}: {text}");
        }
        assert_eq!(cineris_ext::engine::runtimes_made(), made, "a runtime made");
        let held = "Its gate on `tool.before` holds, refusing every call it is on";
        front.drain();
        assert_eq!(told_times(&front, held), 1, "{:?}", front.notices());
        assert!(
            told(
                &front,
                &format!(
                    "Your extension `probe` (`{}`) has changed since you approved it",
                    crate::extensions_found::users_shown(&world)
                )
            ),
            "{:?}",
            front.notices()
        );

        // Approved as it is: its hook joins.
        pin(&world, &folder);
        cache(&world, &folder);
        let second = definition(&folder);
        front.turn("approved again").await;
        let (outcome, text) = last(&front);
        assert_ne!(outcome, "denied", "{text}");

        // Changed again: withdrawn, its gate held, its note gone.
        write(&world, capabilities, &format!("// the third\n{GATES}"));
        front.turn("changed again").await;
        let results = front.results();
        let (_, outcome, text) = &results[results.len() - 2];
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("has changed since it was approved"), "{text}");
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        assert!(!text.contains("the probe saw it"), "{text}");
        // Told once a session: not again for the same change of standing.
        front.drain();
        assert_eq!(told_times(&front, held), 1, "{:?}", front.notices());

        // Unreadable as it is: held all the same.
        let linked = make_link(&world.workspace, &folder.join("linked"));
        front.turn("unreadable").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        if linked {
            assert!(text.contains("cannot be read as it was approved"), "{text}");
            front.drain();
            assert!(
                told(&front, "cannot be read as you approved it (said above)"),
                "{:?}",
                front.notices()
            );
        }

        // Its pin taken away: nothing held.
        cineris_policy::PinStore::new(state(&world).pins_file(), clock(&world))
            .unpin(
                &cineris_protocol::methods::TrustResourceKind::Extension,
                "probe",
            )
            .unwrap();
        front.turn("unpinned").await;
        let (outcome, text) = last(&front);
        assert_ne!(outcome, "denied", "{text}");
        front.close().await;
        // Pinned again, for the next session, at the second's hash.
        cineris_policy::PinStore::new(state(&world).pins_file(), clock(&world))
            .pin(&second)
            .unwrap();
        linked
    });
    // Turned off by the user: nothing is held, unreadable and pinned.
    world.user_config("[extensions.probe]\nenabled = false\n");
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("turned off").await;
        let (outcome, text) = last(&front);
        assert_ne!(outcome, "denied", "linked: {linked}: {text}");
        front.close().await;
    });
}

/// A tool whose script never looks at the interrupt, and a gating hook of
/// the same extension.
const ABANDONED: &str = "cineris.tool({ name: 'spin', run: () => \
       Array.prototype.reverse.call({ length: 2 ** 53 - 1 }) });\n\
     cineris.on('tool.before', { tools: ['list'] }, () => undefined);\n";

/// An extension whose runtime cannot start again fails its gates closed,
/// turns of one session (`docs/steps/28/answered.md`, point 5): once its
/// tool's thread was left running, its gating hook's run fails, saying why
/// and the setting that turns it off, and the call it gates is refused.
#[test]
fn an_extension_whose_runtime_cannot_start_fails_its_gates() {
    let mut responses = Vec::new();
    responses.extend(one_call(
        "c1",
        "tool_call",
        json!({"tool": "probe/spin", "arguments": {}}),
    ));
    responses.extend(one_call("c2", "list", json!({"path": "."})));
    let world = World::new("ext_hooks_abandoned", responses);
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let extension = handed(
        &world,
        "hooks = [\"tool.before\"]\n\n[limits]\ntime = 5",
        ABANDONED,
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = start(&world, &core).await;
        front.prompt("spin").await;
        // The call's time, then the grace its thread is given.
        world.timer.sleeping_for(Duration::from_secs(5)).await;
        world.timer.advance(Duration::from_secs(5));
        world.timer.sleeping_for(cineris_ext::engine::GRACE).await;
        world.timer.advance(cineris_ext::engine::GRACE);
        front.turns(1).await;
        front.turn("list it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        // Only a new hash or a new process helps: the engine's own sentence,
        // which says approving it again does, is not given (the re-check of
        // the full review, L-c).
        for words in [
            "`probe/hook.tool.before` failed",
            "is not started in this process",
            "fix it and approve the fix",
            "`[extensions.probe] enabled = false`",
            "or start Cineris again",
        ] {
            assert!(text.contains(words), "{words}: {text}");
        }
        for words in ["once it is approved again", "is started again"] {
            assert!(!text.contains(words), "{words}: {text}");
        }
        front.close().await;
    });
}

/// A gate on `prompt.submit` and one on `tool.before` that have no
/// opinion, and a `tool.after` hook that notes it saw the call.
const PROMPT_GATES: &str = "cineris.on('prompt.submit', () => undefined);\n\
     cineris.on('tool.before', { tools: ['list'] }, () => undefined);\n\
     cineris.on('tool.after', { tools: ['list'] }, () => ({ context: 'the probe saw it' }));\n";

/// A changed extension's gates stay on their events, failing, and the way
/// out of them stays open (the early review of the script hooks, M1, and
/// the maintainers' call on it), two sessions of one core:
/// - changed during the session, its `prompt.submit` gate refuses every
///   prompt from the next one on, naming the extension, why and the way
///   out, with its `tool.before` gate held beside it, and the user is told
///   of both once, in words; nothing reaches the model;
/// - the core's own operations are no hook points: the user turns the
///   extension off in the configuration, which the core reads again, while
///   the gates hold;
/// - a session opened after that runs a tool call, the extension off.
fn a_changed_extensions_gates_hold_and_the_way_out_stays_open() {
    let mut responses = Vec::new();
    responses.extend(one_call("c1", "list", json!({"path": "."})));
    responses.extend(one_call("c2", "list", json!({"path": "."})));
    let world = World::new("ext_hooks_way_out", responses);
    let capabilities = "hooks = [\"prompt.submit\", \"tool.before\", \"tool.after\"]";
    approved(&world, capabilities, PROMPT_GATES);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("first").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        assert!(text.contains("the probe saw it"), "{text}");

        write(&world, capabilities, &format!("// changed\n{PROMPT_GATES}"));
        let held = "Its gates on `prompt.submit` and `tool.before` hold, refusing every prompt";
        for prompt in ["changed", "again"] {
            let ended = front.turn(prompt).await;
            assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
            front.drain();
            let why = returned(&front);
            for words in [
                "`probe/hook.prompt.submit`",
                "the extension `probe`",
                "has changed since it was approved",
                "`[extensions.probe] enabled = false`",
            ] {
                assert!(why.contains(words), "{words}: {why}");
            }
            assert_eq!(told_times(&front, held), 1, "{:?}", front.notices());
        }
        assert_eq!(
            world.provider.requests().len(),
            2,
            "a refused prompt reached the model"
        );
        turn_it_off(&world, &front).await;
        front.close().await;

        let next = start(&world, &core).await;
        next.turn("after").await;
        let (outcome, text) = last(&next);
        assert_eq!(outcome, "success", "{text}");
        assert!(!text.contains("the probe saw it"), "turned off: {text}");
        next.close().await;
    });
}

/// An extension whose main module throws as it is first evaluated.
const BROKEN: &str = "throw new Error('broken on purpose');\n";

/// Where what an extension registers was never known (its first
/// registration failed), each gating event its manifest lists holds a
/// failing gate, on every call or prompt (the maintainers' call on the
/// early review of the script hooks), two sessions of one core:
/// - its `tool.before` gate refuses every call, naming the extension, why
///   and the way out, and an event that only adds (`tool.after`) holds
///   nothing; the user is told once, in words, and it is not evaluated
///   again;
/// - the user turns it off in the configuration, which the core reads
///   again, and a session opened after that runs the call.
fn an_extension_never_registered_holds_the_gates_its_manifest_lists() {
    let mut responses = Vec::new();
    for id in ["c1", "c2", "c3"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_never_registered", responses);
    let folder = write(&world, "hooks = [\"tool.before\", \"tool.after\"]", BROKEN);
    pin(&world, &folder);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let held = "Its gate on `tool.before` holds, refusing every call it is on";
        for prompt in ["first", "again"] {
            front.turn(prompt).await;
            let (outcome, text) = last(&front);
            assert_eq!(outcome, "denied", "{text}");
            for words in [
                "`probe/hook.tool.before` failed",
                "the extension `probe`",
                "could not be registered",
                "`[extensions.probe] enabled = false`",
            ] {
                assert!(text.contains(words), "{words}: {text}");
            }
            front.drain();
            assert_eq!(told_times(&front, held), 1, "{:?}", front.notices());
        }
        assert!(
            told(&front, "could not be registered (said above)"),
            "{:?}",
            front.notices()
        );
        assert_eq!(
            told_times(&front, "could not be registered, so it is not loaded"),
            1,
            "{:?}",
            front.notices()
        );
        turn_it_off(&world, &front).await;
        front.close().await;

        let next = start(&world, &core).await;
        next.turn("after").await;
        let (outcome, text) = last(&next);
        assert_eq!(outcome, "success", "{text}");
        next.close().await;
    });
}

/// An extension removed from where it was approved, and still pinned,
/// holds its gates from what its pin's registration registered, failing
/// (the early review of the script hooks, M1, and the maintainers' call on
/// it: a deletion may be an unconfined program's, taking a gate away), two
/// sessions of one core: its folder removed, its `tool.before` gate
/// refuses, saying, to the model and to the user once, that its folder is
/// gone and how to turn it off if it was removed on purpose; turned off so
/// (`enabled = false`), a session opened after holds nothing.
fn a_removed_extensions_gates_hold_until_it_is_turned_off() {
    let mut responses = Vec::new();
    for id in ["c1", "c2", "c3"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_removed", responses);
    let folder = approved(&world, "hooks = [\"tool.before\", \"tool.after\"]", GATES);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("first").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");

        std::fs::remove_dir_all(&folder).unwrap();
        front.turn("removed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        let gone = "its folder is gone; if you removed it, turn it off with `[extensions.probe] \
                    enabled = false` (or, from the next release, `cineris ext remove probe`)";
        for words in [
            "`probe/hook.tool.before` failed",
            "the extension `probe` cannot be found where it was approved, so its gate refuses it",
            gone,
        ] {
            assert!(text.contains(words), "{words}: {text}");
        }
        front.drain();
        let told = format!(
            "Your extension `probe` cannot be found where you approved it. Its gate on \
             `tool.before` holds, refusing every call it is on: {gone}."
        );
        assert_eq!(told_times(&front, &told), 1, "{:?}", front.notices());
        turn_it_off(&world, &front).await;
        front.close().await;

        let next = start(&world, &core).await;
        next.turn("turned off").await;
        let (outcome, text) = last(&next);
        assert_eq!(outcome, "success", "{text}");
        next.close().await;
    });
}

/// An extension approved and found as approved whose capabilities do not
/// compile for the workspace (a glob whose `{` is not closed) is not
/// loaded, and its gates are held from what it registered, failing (the
/// early review of the script hooks, M1).
fn an_extension_that_cannot_be_loaded_holds_its_gates() {
    let world = World::new(
        "ext_hooks_unloaded",
        one_call("c1", "list", json!({"path": "."})),
    );
    approved(
        &world,
        "hooks = [\"tool.before\", \"tool.after\"]\nread = [\"src/{a\"]",
        GATES,
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        for words in [
            "`probe/hook.tool.before` failed",
            "could not be loaded",
            "`[extensions.probe] enabled = false`",
        ] {
            assert!(text.contains(words), "{words}: {text}");
        }
        front.drain();
        assert!(
            told(&front, "The extension `probe` is not loaded"),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
}

/// A tool whose write nobody can be asked about, and an observer on `list`
/// that reads, then says it did.
const READS_LATE: &str = "cineris.tool({ name: 'ci', run: async () => { \
       try { await cineris.fs.write('.env', 'TOKEN=1\\n') } catch (e) { } \
       return 'went on' } });\n\
     cineris.on('tool.before', { tools: ['list'], observe: true }, async () => {\n\
       const text = await cineris.fs.read('notes.md');\n\
       cineris.notify('read ' + text.trim());\n\
     });\n";

/// A hook still running when another call stops its turn, which then
/// reads, fails: the stop its read finds is the turn's, no stop of its own,
/// so nothing says it was stopped by the look, and no prompt is held back
/// (the early review of the script hooks, L3). Its run is held as it is
/// about to call its extension while the extension's tool, the round's
/// next call, has its write, which nobody can be asked about, stop the
/// turn through the policy.
fn a_hook_that_finds_its_turn_stopped_is_no_stop_of_its_own() {
    let world = World::new(
        "ext_hooks_found_stopped",
        [
            calls(&[
                ("c1", "list", json!({"path": "."})),
                (
                    "c2",
                    "tool_call",
                    json!({"tool": "probe/ci", "arguments": {}}),
                ),
            ]),
            says("Done."),
        ],
    );
    world.put("notes.md", "the notes\n");
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    approved(
        &world,
        "write = [\".env\"]\nread = [\"**\"]\nhooks = [\"tool.before\"]",
        READS_LATE,
    );
    let core = world.core_with(|options| options.fail_on_denial = true);
    world.run(async {
        let (front, _) = open_as(
            &world,
            &core,
            ClientInfo::headless(),
            TrustResponse::default(),
        )
        .await;
        let starts = Hold::new(cineris_core::ext::A_SCRIPT_HOOK_STARTS);
        let mut turn = std::pin::pin!(front.turn("look"));
        starts.drive_until_reached(turn.as_mut()).await;
        let ended = turn.await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        assert_eq!(world.read(".env"), None);
        // The turn is stopped: the held run goes on, and reads.
        let ends = Hold::new(cineris_core::ext::A_SCRIPT_HOOK_ENDS);
        starts.release();
        drop(starts);
        let mut idle = std::pin::pin!(std::future::pending::<()>());
        ends.drive_until_reached(idle.as_mut()).await;
        ends.release();
        drop(ends);
        front.drain();
        assert!(
            !told(&front, "was stopped by the look"),
            "{:?}",
            front.notices()
        );
        assert!(!told(&front, "read the notes"), "{:?}", front.notices());
        let next = front.turn("again").await;
        assert_eq!(
            next.outcome,
            TurnOutcome::Completed,
            "a stop the hook found held back a prompt: {next:?}"
        );
        front.close().await;
    });
}

/// A `turn.end` hook that says it started, then computes for ever, with a
/// time of one second.
const SPINS: &str = "cineris.on('turn.end', { timeout: 1000 }, () => {\n\
       cineris.notify('spinning');\n\
       for (;;) {}\n\
     });\n";

/// A hook's time starts when it is asked at its point too, which has no
/// limit of its own beside the hook's (the early review of the script
/// hooks, L4): of two runs asked at once, the second waits behind the
/// first, which computes; when their second is up, the first is stopped
/// and the second, its time spent in the queue, is withdrawn: both fail as
/// out of time, and neither waits for a second more.
fn a_hooks_time_at_its_point_runs_from_when_it_was_asked() {
    let world = World::new("ext_hooks_asked", Vec::<Response>::new());
    let extension = handed(&world, "hooks = [\"turn.end\"]", SPINS);
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = start(&world, &core).await;
        let mut runs =
            std::pin::pin!(core.hook_runs_at_its_point(&front.session, "probe/hook.turn.end", 2));
        drive_until(runs.as_mut(), || {
            front.drain();
            told(&front, "spinning")
        })
        .await;
        world.timer.advance(Duration::from_secs(1));
        let said = runs.await.unwrap();
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said.iter().all(|why| why
                .as_deref()
                .is_some_and(|why| why.contains("did not answer within its 1000 ms"))),
            "{said:?}"
        );
        assert_eq!(told_times(&front, "spinning"), 1, "{:?}", front.notices());
        front.close().await;
    });
}

/// A `turn.end` observer that says it started, then computes for ever,
/// with a time of twenty seconds.
const WAITS: &str = "cineris.on('turn.end', { observe: true, timeout: 20000 }, () => {\n\
       cineris.notify('waiting');\n\
       for (;;) {}\n\
     });\n";

/// An extension withdrawn from the session stops what of it still runs
/// (the early review of the script hooks, L6): a run of its `turn.end`
/// hook, behind the first turn's, which computes, ends as let go when the
/// extension, changed on disk, is withdrawn at the next turn's start, well
/// inside its own time.
fn a_withdrawn_extensions_runs_stop() {
    let world = World::new("ext_hooks_withdrawn_runs", [says("Done."), says("Done.")]);
    let capabilities = "hooks = [\"turn.end\"]";
    approved(&world, capabilities, WAITS);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("first").await;
        front.until(|_| told(&front, "waiting")).await;
        let mut runs =
            std::pin::pin!(core.hook_runs_at_its_point(&front.session, "probe/hook.turn.end", 1));
        assert!(poll_once(runs.as_mut()).await.is_pending());
        write(&world, capabilities, &format!("// changed\n{WAITS}"));
        front.turn("changed").await;
        // Past its time: a run the withdrawal did not stop runs out of it.
        world.timer.advance(Duration::from_secs(20));
        let said = runs.await.unwrap();
        assert!(
            said[0].as_deref().is_some_and(|why| why.contains("let go")),
            "{said:?}"
        );
        front.close().await;
    });
}

/// A `turn.end` hook that has no opinion.
const QUIET: &str = "cineris.on('turn.end', () => undefined);\n";

/// A hook's run let go while its extension's runtime starts ends at once,
/// and leaves the start to go on by itself (the early review of the script
/// hooks, L5): held as the runtime's main module registers its hook, the
/// run let go no longer counts among the four a waited `turn.end` hook may
/// have going, so four more start; once the start goes on, they answer.
fn a_hooks_run_let_go_while_its_runtime_starts_ends_at_once() {
    let world = World::new("ext_hooks_starting", Vec::<Response>::new());
    let extension = handed(&world, "hooks = [\"turn.end\"]", QUIET);
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = start(&world, &core).await;
        let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
        {
            let mut first = std::pin::pin!(core.hook_runs_at_its_point(
                &front.session,
                "probe/hook.turn.end",
                1
            ));
            drive_until(first.as_mut(), || registers.wait_reached(Duration::ZERO)).await;
        }
        settle().await;
        let mut four =
            std::pin::pin!(core.hook_runs_at_its_point(&front.session, "probe/hook.turn.end", 4));
        assert!(poll_once(four.as_mut()).await.is_pending());
        registers.release();
        drop(registers);
        let said = four.await.unwrap();
        assert!(said.iter().all(Option::is_none), "{said:?}");
        front.close().await;
    });
}

/// A gate on `list` that has no opinion.
const LIST_GATE: &str = "cineris.on('tool.before', { tools: ['list'] }, () => undefined);\n";

/// A changed extension whose approved registration is known nowhere (not
/// in the registration cache, never loaded in the session) holds both
/// gates, `tool.before` and `prompt.submit`, whatever its manifest as
/// found says (the maintainers' call on the early review of the script
/// hooks): changed to drop `capabilities.hooks`, it still refuses the
/// prompt, which never reaches the model; turned off, a session opened
/// after runs the call.
fn a_changed_extension_whose_registration_is_not_known_holds_both_gates() {
    let world = World::new(
        "ext_hooks_changed_unknown",
        one_call("c1", "list", json!({"path": "."})),
    );
    let folder = write(&world, "hooks = [\"tool.before\"]", LIST_GATE);
    pin(&world, &folder);
    write(&world, "", "// changed, its hooks dropped\n");
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let ended = front.turn("look").await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        front.drain();
        let why = returned(&front);
        for words in [
            "`probe/hook.prompt.submit`",
            "the extension `probe`",
            "has changed since it was approved",
        ] {
            assert!(why.contains(words), "{words}: {why}");
        }
        assert_eq!(
            told_times(
                &front,
                "Its gates on `prompt.submit` and `tool.before` hold, refusing every prompt"
            ),
            1,
            "{:?}",
            front.notices()
        );
        assert!(
            world.provider.requests().is_empty(),
            "a refused prompt reached the model"
        );
        turn_it_off(&world, &front).await;
        front.close().await;

        let next = start(&world, &core).await;
        next.turn("turned off").await;
        let (outcome, text) = last(&next);
        assert_eq!(outcome, "success", "{text}");
        next.close().await;
    });
}

/// While the store of the user's pins cannot be read, each of the user's
/// extensions found holds failing gates (the maintainers' calls on the
/// early review of the script hooks, and the full review's L1), two
/// sessions of one core:
/// - never loaded in the session (the store unreadable as it opens), it
///   holds the gates of its approved versions as the registration cache
///   keeps them, which stands in for the store (the third re-check of the
///   full review, L2), beside a gate on each gating event its manifest as
///   found lists, since the cache's entries say nothing of whose they are
///   (the fourth look, L1): every call is refused, naming why and the way
///   out, and, changed to list no hook, a `list` all the same; the user is told once, the store's file under `~` and the fix;
///   the core's own operations go on (the configuration is read again); the
///   store removed and the extension approved again, it is loaded at the
///   next turn's start, and the call runs;
/// - loaded, then the store made unreadable, it is withdrawn and keeps the
///   gates of what the session saw it register: its gate on `list` alone,
///   so a `read` runs and a `list` is refused; changed to list no hook, and
///   then removed, it keeps them all the same (the re-check of the full
///   review, M1').
fn an_unreadable_pin_store_holds_the_gates_of_the_users_extensions() {
    let mut responses = Vec::new();
    responses.extend(one_call("c1", "read", json!({"path": "notes.md"})));
    for id in ["c2", "c3", "c4", "c5"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    responses.extend(one_call("c6", "list", json!({"path": "."})));
    responses.extend(one_call("c7", "read", json!({"path": "notes.md"})));
    for id in ["c8", "c9", "c10"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_pins_unreadable", responses);
    world.put("notes.md", "notes\n");
    let folder = approved(&world, "hooks = [\"tool.before\", \"tool.after\"]", GATES);
    let pins = state(&world).pins_file();
    std::fs::write(&pins, "version = 9\n").unwrap();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let told = "could not be read, so your extensions cannot be judged";
        front.turn("read it").await;
        let (outcome, text) = last(&front);
        assert_eq!(
            outcome, "denied",
            "the manifest's gate holds beside the cache's: {text}"
        );
        for prompt in ["list it", "list again"] {
            front.turn(prompt).await;
            let (outcome, text) = last(&front);
            assert_eq!(outcome, "denied", "{prompt}: {text}");
            for words in [
                "`probe/hook.tool.before` failed",
                "the pin store could not be read",
                "repair or remove `~/",
                "then approve it again (`cineris ext add",
            ] {
                assert!(text.contains(words), "{words}: {text}");
            }
            front.drain();
            assert_eq!(told_times(&front, told), 1, "{:?}", front.notices());
        }
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains(told) && notice.contains("The pin store (`~/")),
            "{:?}",
            front.notices()
        );
        reads_the_configuration_again(&front).await;
        // Changed to list no hook: the cache's approved version holds.
        write(&world, "", "// changed, its hooks dropped\n");
        front.turn("changed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "changed: {text}");
        // The fix: the store removed, and the extension approved again.
        write(&world, "hooks = [\"tool.before\", \"tool.after\"]", GATES);
        std::fs::remove_file(&pins).unwrap();
        pin(&world, &folder);
        front.turn("fixed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        front.close().await;

        // Loaded, then the store made unreadable: what it registered holds.
        let front = start(&world, &core).await;
        front.turn("loaded").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        std::fs::write(&pins, "version = 9\n").unwrap();
        // Loaded: what it registered decides, not its manifest.
        front.turn("read it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "its gate is on `list` alone: {text}");
        front.turn("list it").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("the pin store could not be read"), "{text}");
        // Changed to list no hook, then removed: what it registered holds
        // (the re-check of the full review, M1').
        write(&world, "", "// changed, its hooks dropped\n");
        front.turn("changed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "changed: {text}");
        std::fs::remove_dir_all(&folder).unwrap();
        front.turn("removed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "removed: {text}");
        front.close().await;
    });
}

/// While the trust store cannot be read, a trusted project's extension the
/// session loaded keeps the gates it registered, failing, whatever it is
/// found as, or if it is gone (the owner's answer on the re-check of the
/// full review, L-b, and its M1'), turns of one session: loaded, the store
/// then made unreadable, its gate on `list` refuses, naming why and the way
/// out, and the user is told once that the project's extensions are not
/// loaded; changed to list no hook, then removed, it refuses all the same;
/// the core's own operations go on; the store removed, nothing is held.
fn an_unreadable_trust_store_holds_the_gates_of_a_projects_extensions() {
    let mut responses = Vec::new();
    for id in ["c1", "c2", "c3", "c4", "c5"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_trust_unreadable", responses);
    let folder = crate::extensions_found::projects(&world);
    write_in(&folder, "hooks = [\"tool.before\", \"tool.after\"]", GATES);
    crate::extensions_found::trust(
        &world,
        &[crate::extensions_trust::named_definition(&folder, "probe")],
    );
    cache(&world, &folder);
    let store = state(&world).trust_file();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("loaded").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        std::fs::write(&store, "version = 9\n").unwrap();
        let told = "this project's extensions are not loaded until it is fixed";
        let gated = |front: &Front, phase: &str| {
            let (outcome, text) = last(front);
            assert_eq!(outcome, "denied", "{phase}: {text}");
            for words in [
                "`probe/hook.tool.before` failed",
                "the trust store could not be read",
                "repair or remove `~/",
                "`cineris permissions trust`",
            ] {
                assert!(text.contains(words), "{phase}: {words}: {text}");
            }
        };
        front.turn("gated").await;
        gated(&front, "unreadable");
        // Changed to list no hook: the gates it registered hold.
        write_in(&folder, "", "// changed, its hooks dropped\n");
        front.turn("changed").await;
        gated(&front, "changed");
        // Removed: they hold all the same.
        std::fs::remove_dir_all(&folder).unwrap();
        front.turn("removed").await;
        gated(&front, "removed");
        front.drain();
        assert_eq!(told_times(&front, told), 1, "{:?}", front.notices());
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains(told) && notice.contains("The trust store (`~/")),
            "{:?}",
            front.notices()
        );
        reads_the_configuration_again(&front).await;
        // The fix: the store removed, and with it the project's trust.
        std::fs::remove_file(&store).unwrap();
        front.turn("fixed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        front.close().await;
    });
}

/// A gate on `prompt.submit` that refuses every prompt.
const REFUSES_PROMPTS: &str =
    "cineris.on('prompt.submit', () => ({ decision: 'deny', reason: 'no prompt' }));\n";

/// No repository gates a session by itself (2b-1's settled point 10): while
/// the trust store cannot be read, a project's extension the session never
/// loaded, whose manifest lists `prompt.submit`, holds nothing, and a prompt
/// goes through (the owner's answer on the re-check of the full review,
/// L-b): of a project not trusted, which nothing looks in, and of one
/// trusted for the run (`--trust-project`), which approves no extension, and
/// whose folder is looked in.
fn an_unreadable_trust_store_gates_nothing_of_a_project_the_session_never_loaded() {
    for trusted_for_the_run in [false, true] {
        let world = World::new(
            &format!("ext_hooks_trust_unread_{trusted_for_the_run}"),
            [says("Hello.")],
        );
        write_in(
            &crate::extensions_found::projects(&world),
            "hooks = [\"prompt.submit\"]",
            REFUSES_PROMPTS,
        );
        std::fs::write(state(&world).trust_file(), "version = 9\n").unwrap();
        let core = world.core_with(|options| options.trust_project = trusted_for_the_run);
        world.run(async {
            let front = start(&world, &core).await;
            let ended = front.turn("hi").await;
            assert_eq!(
                ended.outcome,
                TurnOutcome::Completed,
                "for the run: {trusted_for_the_run}: {ended:?}"
            );
            assert_eq!(world.provider.requests().len(), 1);
            if trusted_for_the_run {
                front.drain();
                assert!(
                    told(
                        &front,
                        "this project's extensions are not loaded until it is fixed"
                    ),
                    "{:?}",
                    front.notices()
                );
            }
            front.close().await;
        });
    }
}

/// The core reads the configuration again: an operation of its own, which
/// no hook gates, while gates hold.
async fn reads_the_configuration_again(front: &Front) {
    let reloaded = front
        .op(Op::ConfigReload(ConfigReloadParams::default()))
        .await;
    assert!(
        matches!(reloaded, Ok(OpResult::ConfigReload(_))),
        "{reloaded:?}"
    );
}

/// An observer on `list` that reads, then says it did, and a `turn.end`
/// hook that says which turn it ran for.
const STOPPED_BY_THE_LOOP: &str = "cineris.on('tool.before', { tools: ['list'], observe: true }, async () => {\n\
       const text = await cineris.fs.read('notes.md');\n\
       cineris.notify('read ' + text.trim());\n\
     });\n\
     cineris.on('turn.end', (e) => { cineris.notify('turn.end ran in ' + e.turn_id); });\n";

/// A turn the loop's own stop ends (a call nobody can be asked about, with
/// `fail_on_denial`) refuses its hooks as a turn the policy stopped does
/// (the maintainers' call on the early review of the script hooks): a hook
/// still running when the round's write stops the turn, which then reads,
/// is refused, and the turn's `turn.end` hook does not start; the next turn
/// is not stopped, and its own hooks run.
fn a_turn_the_loops_own_stop_ends_refuses_its_hooks() {
    let world = World::new(
        "ext_hooks_loop_stopped",
        [
            calls(&[
                ("c1", "list", json!({"path": "."})),
                (
                    "c2",
                    "write",
                    json!({"path": ".env", "content": "TOKEN=1\n"}),
                ),
            ]),
            says("Done."),
        ],
    );
    world.put("notes.md", "the notes\n");
    approved(
        &world,
        "read = [\"**\"]\nhooks = [\"tool.before\", \"turn.end\"]",
        STOPPED_BY_THE_LOOP,
    );
    let core = world.core_with(|options| options.fail_on_denial = true);
    world.run(async {
        let (front, _) = open_as(
            &world,
            &core,
            ClientInfo::headless(),
            TrustResponse::default(),
        )
        .await;
        let starts = Hold::new(cineris_core::ext::A_SCRIPT_HOOK_STARTS);
        let mut turn = std::pin::pin!(front.turn("look"));
        starts.drive_until_reached(turn.as_mut()).await;
        let ended = turn.await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        assert_eq!(world.read(".env"), None);
        let ends = Hold::new(cineris_core::ext::A_SCRIPT_HOOK_ENDS);
        starts.release();
        drop(starts);
        let mut idle = std::pin::pin!(std::future::pending::<()>());
        ends.drive_until_reached(idle.as_mut()).await;
        ends.release();
        drop(ends);
        front.drain();
        assert!(!told(&front, "read the notes"), "{:?}", front.notices());
        let next = front.turn("again").await;
        assert_eq!(next.outcome, TurnOutcome::Completed, "{next:?}");
        front.until(|_| told(&front, "turn.end ran in t_2")).await;
        assert!(
            !told(&front, "turn.end ran in t_1"),
            "a stopped turn's hook started: {:?}",
            front.notices()
        );
        front.close().await;
    });
}

/// [`LIST_GATE`]'s extension changed when its registration cache is set
/// aside: approved and cached, the cache made unreadable, then changed to
/// drop `capabilities.hooks`, it holds both gates in the next session, as
/// one never registered does (the full review of the script hooks, M1).
fn a_changed_extension_with_its_cache_set_aside_holds_both_gates() {
    let world = World::new("ext_hooks_cache_aside", Vec::<Response>::new());
    approved(&world, "hooks = [\"tool.before\"]", LIST_GATE);
    std::fs::write(state(&world).registration_cache_file(), "not json").unwrap();
    write(&world, "", "// changed, its hooks dropped\n");
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let ended = front.turn("look").await;
        assert_eq!(ended.outcome, TurnOutcome::Denied, "{ended:?}");
        front.drain();
        let why = returned(&front);
        assert!(
            why.contains("`probe/hook.prompt.submit`")
                && why.contains("has changed since it was approved"),
            "{why}"
        );
        assert!(world.provider.requests().is_empty());
        front.close().await;
    });
}

/// Under `--trust-project`, which shows no trust question, a changed
/// project's extension's held gate names none in its way out: it names
/// `cineris permissions trust`, which shows the extension to approve it
/// (the full review of the script hooks).
fn under_trust_for_the_run_a_changed_projects_gate_names_no_question() {
    let world = World::new(
        "ext_hooks_trust_for_the_run",
        one_call("c1", "list", json!({"path": "."})),
    );
    let folder = crate::extensions_found::projects(&world);
    write_in(&folder, "hooks = [\"tool.before\", \"tool.after\"]", GATES);
    crate::extensions_found::trust(
        &world,
        &[crate::extensions_trust::named_definition(&folder, "probe")],
    );
    cache(&world, &folder);
    write_in(
        &folder,
        "hooks = [\"tool.before\", \"tool.after\"]",
        &format!("// changed\n{GATES}"),
    );
    let core = world.core_with(|options| options.trust_project = true);
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("look").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(
            text.contains("has changed since it was approved")
                && text.contains("`cineris permissions trust` shows it")
                && !text.contains("trust question"),
            "{text}"
        );
        front.close().await;
    });
}

/// A project's extension approved for the session by a yes the trust store
/// could not keep, and loaded, keeps the gates it registered while the store
/// cannot be read, changed to list no hook: what the session saw it
/// register is what holds, the cache keeping nothing of a hash no pin names
/// (the re-check of the full review, M1' and L-a).
fn an_extension_approved_for_the_run_keeps_its_gates_while_the_store_is_unread() {
    let mut responses = Vec::new();
    for id in ["c1", "c2"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_for_the_run", responses);
    let folder = crate::extensions_found::projects(&world);
    write_in(&folder, "hooks = [\"tool.before\", \"tool.after\"]", GATES);
    // A yes the store cannot keep: a folder where its lock goes.
    let store = state(&world).trust_file();
    std::fs::create_dir_all(state(&world).root().join("trust.toml.lock")).unwrap();
    let core = world.core();
    world.run(async {
        let (front, asked) =
            crate::extensions_found::open_answering(&world, &core, crate::extensions_found::yes())
                .await;
        assert_eq!(asked.len(), 1);
        front.turn("loaded").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        std::fs::write(&store, "version = 9\n").unwrap();
        write_in(&folder, "", "// changed, its hooks dropped\n");
        front.turn("changed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("`probe/hook.tool.before` failed"), "{text}");
        front.close().await;
    });
}

/// A `tool.after` hook that notes it saw the call: no gate.
const ONLY_AFTER: &str =
    "cineris.on('tool.after', { tools: ['list'] }, () => ({ context: 'seen' }));\n";

/// What the session saw an extension register is known, gating hooks or
/// none: one that registered only a `tool.after` hook, changed when its
/// registration cache is set aside, holds no gate, and the next prompt runs
/// (the re-check of the full review, L-e).
fn a_changed_extension_the_session_saw_register_no_gate_holds_none() {
    let mut responses = Vec::new();
    for id in ["c1", "c2"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_seen_no_gate", responses);
    let capabilities = "hooks = [\"tool.after\"]";
    approved(&world, capabilities, ONLY_AFTER);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("loaded").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        std::fs::write(state(&world).registration_cache_file(), "not json").unwrap();
        write(&world, capabilities, &format!("// changed\n{ONLY_AFTER}"));
        let ended = front.turn("changed").await;
        assert_eq!(ended.outcome, TurnOutcome::Completed, "{ended:?}");
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        assert!(!text.contains("seen"), "withdrawn, it adds nothing: {text}");
        front.close().await;
    });
}

/// A `session.end` hook that keeps a value of the project's.
const FAREWELL: &str = "cineris.on('session.end', async () => { \
       await cineris.state.set('ended', 'yes', { scope: 'project' }); });\n";

/// A `session.end` hook of an extension nothing else of ran in the session
/// starts its runtime as the session closes, after the close has begun: its
/// start is ended by the end of its own time there, not by the close's
/// start (the re-check of the full review, L-d), so it runs and keeps its
/// value.
fn a_session_end_hook_starts_its_runtime_as_the_session_closes() {
    let world = World::new("ext_hooks_end_starts", Vec::<Response>::new());
    let extension = handed(&world, "hooks = [\"session.end\"]", FAREWELL);
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = start(&world, &core).await;
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::runtimes_made(), made + 1);
    let kept = crate::ext_state::project_files(&world);
    assert!(
        kept.iter().any(|file| std::fs::read_to_string(file)
            .is_ok_and(|text| text.contains("\"ended\":\"yes\""))),
        "the `session.end` hook did not run: {kept:?}"
    );
}

/// While the store of the user's pins cannot be read, an extension found
/// that the registration cache keeps nothing of holds a gate on each gating
/// event its manifest as found lists: the cache has no approved version to
/// stand in for the store (the third re-check of the full review, L2).
fn an_unreadable_pin_store_falls_back_to_the_manifest_where_the_cache_has_none() {
    let world = World::new(
        "ext_hooks_pins_no_cache",
        one_call("c1", "read", json!({"path": "notes.md"})),
    );
    world.put("notes.md", "notes\n");
    let folder = write(&world, "hooks = [\"tool.before\"]", LIST_GATE);
    pin(&world, &folder);
    std::fs::write(state(&world).pins_file(), "version = 9\n").unwrap();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("read it").await;
        let (outcome, text) = last(&front);
        assert_eq!(
            outcome, "denied",
            "the manifest's gate is on every call: {text}"
        );
        front.close().await;
    });
}

/// A trusted project's extension approved and never registered (its script
/// throws) is known by the gating events its approved manifest lists: while
/// the trust store cannot be read and it is changed to list no hook, its
/// gate holds, as it did with the store read (the third re-check of the
/// full review, L1).
fn an_extension_never_registered_keeps_its_gates_while_the_trust_store_is_unread() {
    let mut responses = Vec::new();
    for id in ["c1", "c2"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_unregistered_unread", responses);
    let folder = crate::extensions_found::projects(&world);
    write_in(&folder, "hooks = [\"tool.before\"]", BROKEN);
    crate::extensions_found::trust(
        &world,
        &[crate::extensions_trust::named_definition(&folder, "probe")],
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("gated").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        std::fs::write(state(&world).trust_file(), "version = 9\n").unwrap();
        write_in(&folder, "", "// changed, its hooks dropped\n");
        front.turn("changed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        assert!(text.contains("`probe/hook.tool.before` failed"), "{text}");
        front.close().await;
    });
}

/// What is known of an extension that left the session for good (its pin
/// taken away) is forgotten: a store that cannot be read later gives it no
/// gate back (the third re-check of the full review).
fn an_extension_unpinned_gets_no_gate_back_when_the_pin_store_breaks() {
    let mut responses = Vec::new();
    for id in ["c1", "c2", "c3"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_unpinned_forgotten", responses);
    approved(&world, "hooks = [\"tool.before\"]", LIST_GATE);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("loaded").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        cineris_policy::PinStore::new(state(&world).pins_file(), clock(&world))
            .unpin(
                &cineris_protocol::methods::TrustResourceKind::Extension,
                "probe",
            )
            .unwrap();
        front.turn("unpinned").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        std::fs::remove_dir_all(users(&world)).unwrap();
        std::fs::write(state(&world).pins_file(), "version = 9\n").unwrap();
        front.turn("store broken").await;
        let (outcome, text) = last(&front);
        assert_eq!(
            outcome, "success",
            "an unpinned one's gate came back: {text}"
        );
        front.close().await;
    });
}

/// A main module that, once it has registered its hook, tries every host
/// call a first evaluation might, catching each refusal.
const TRIES_AS_IT_STARTS: &str = "cineris.on('turn.end', () => undefined);\n\
     const tried = (call) => { try { call().catch(() => {}); } catch (e) {} };\n\
     tried(() => cineris.fs.write('started.txt', 'x'));\n\
     tried(() => cineris.exec('gh', ['x']));\n\
     tried(() => cineris.http.fetch('https://api.example.com/x'));\n\
     tried(() => cineris.ask('may I?'));\n\
     tried(() => cineris.state.set('started', 'yes', { scope: 'project' }));\n\
     tried(() => cineris.notify('started after all'));\n\
     tried(() => cineris.log('started after all'));\n";

/// A runtime start a hook's run leaves to go on, once the session closes,
/// runs on, unawaited, to its own evaluation time (the maintainers, on the
/// third re-check of the full review, L3): held as its main module registers
/// its hook, past the close and past a grace's length, then let go within
/// its evaluation time,
/// - the close ends without it;
/// - its main module's host calls, made after the close, are each refused
///   with nothing done: it writes no file, keeps no value, tells nothing;
/// - it is never counted as left running, so a later session's hook of the
///   same extension, in the same process, runs, with a runtime of its own.
#[test]
fn a_runtime_start_the_close_lets_go_runs_on_and_does_nothing() {
    let world = World::new("ext_hooks_start_runs_on", Vec::<Response>::new());
    let extension = handed(
        &world,
        "hooks = [\"turn.end\"]\nwrite = [\"**\"]\nexec = [\"gh\"]\nnet = [\"api.example.com\"]",
        TRIES_AS_IT_STARTS,
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let abandoned = cineris_ext::engine::abandoned_threads();
    world.run(async {
        let front = start(&world, &core).await;
        let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
        {
            let mut run = std::pin::pin!(core.hook_runs_at_its_point(
                &front.session,
                "probe/hook.turn.end",
                1
            ));
            drive_until(run.as_mut(), || registers.wait_reached(Duration::ZERO)).await;
        }
        settle().await;
        // The close ends with the start still held where it stopped.
        front.close().await;
        assert!(
            registers.wait_stopped(1, Duration::ZERO),
            "the close waited for a runtime start it had let go"
        );
        // Past a grace, well within its evaluation time: nothing ends it.
        world.timer.advance(cineris_ext::engine::GRACE * 2);
        settle().await;
        assert!(registers.wait_stopped(1, Duration::ZERO));
        registers.release();
        drop(registers);
        // A later session of the same process runs the extension's hook.
        let next = start(&world, &core).await;
        let said = core
            .hook_runs_at_its_point(&next.session, "probe/hook.turn.end", 1)
            .await
            .unwrap();
        assert_eq!(said, [None], "a later session's hook did not run");
        next.close().await;
    });
    assert!(
        world.read("started.txt").is_none(),
        "a first evaluation's write was done"
    );
    assert!(
        crate::ext_state::project_files(&world).is_empty(),
        "a first evaluation's value was kept"
    );
    assert_eq!(
        cineris_ext::engine::abandoned_threads(),
        abandoned,
        "a start let go within its evaluation time was counted as left running"
    );
}

/// A runtime start the close let go that is still running past its
/// evaluation time and grace is left running and counted once, as any start
/// is (the maintainers, on the third re-check of the full review, L3).
#[test]
fn a_runtime_start_the_close_lets_go_past_its_time_counts_once() {
    let world = World::new("ext_hooks_start_past_time", Vec::<Response>::new());
    let extension = handed(&world, "hooks = [\"turn.end\"]", QUIET);
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let abandoned = cineris_ext::engine::abandoned_threads();
    world.run(async {
        let front = start(&world, &core).await;
        let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
        {
            let mut run = std::pin::pin!(core.hook_runs_at_its_point(
                &front.session,
                "probe/hook.turn.end",
                1
            ));
            drive_until(run.as_mut(), || registers.wait_reached(Duration::ZERO)).await;
        }
        front.close().await;
        assert!(
            registers.wait_stopped(1, Duration::ZERO),
            "the close waited for a runtime start it had let go"
        );
        let evaluation = cineris_ext::engine::EVALUATION_TIME;
        let grace = cineris_ext::engine::GRACE;
        let mut idle = std::pin::pin!(std::future::pending::<()>());
        drive_until(idle.as_mut(), || {
            world.timer.running().contains(&evaluation)
        })
        .await;
        // Let go and evaluating: it counts toward the cap.
        assert_eq!(cineris_ext::engine::starts_let_go(), 1);
        assert_eq!(cineris_ext::engine::abandoned_threads(), abandoned);
        world.timer.advance(evaluation);
        drive_until(idle.as_mut(), || world.timer.running().contains(&grace)).await;
        world.timer.advance(grace);
        drive_until(idle.as_mut(), || {
            cineris_ext::engine::abandoned_threads() > abandoned
        })
        .await;
        registers.release();
        drop(registers);
        settle().await;
    });
    // Left running: the one count, no longer a start let go too.
    assert_eq!(cineris_ext::engine::abandoned_threads(), abandoned + 1);
    assert_eq!(cineris_ext::engine::starts_let_go(), 0);
}

/// The core's shutdown does not wait for a runtime start a closed session's
/// hook left to go on: it ends while the start is still held where its main
/// module registers its hook (the maintainers, on the third re-check of the
/// full review, L3).
#[test]
fn the_cores_shutdown_ends_while_a_runtime_start_runs_on() {
    let world = World::new("ext_hooks_start_shutdown", Vec::<Response>::new());
    let extension = handed(&world, "hooks = [\"turn.end\"]", QUIET);
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = start(&world, &core).await;
        let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
        {
            let mut run = std::pin::pin!(core.hook_runs_at_its_point(
                &front.session,
                "probe/hook.turn.end",
                1
            ));
            drive_until(run.as_mut(), || registers.wait_reached(Duration::ZERO)).await;
        }
        settle().await;
        front.close().await;
        let _ = front
            .op(Op::Shutdown(cineris_protocol::methods::Empty {}))
            .await;
        assert!(
            registers.wait_stopped(1, Duration::ZERO),
            "the core's shutdown waited for a runtime start"
        );
        registers.release();
        drop(registers);
    });
}

/// Starts let go cannot stack (the fourth look at the script hooks, (b)):
/// each start a closed session let go counts toward the cap of four while
/// it evaluates. Four sessions of one process each close with a start of
/// `probe` held; a fifth session's gate, another extension's, then cannot
/// start its runtime: its `tool.before` refuses the call, failing closed,
/// and the user is told once that it clears within the evaluation time;
/// once one start ends within its time, it is released, and the gate runs
/// again.
#[test]
fn starts_let_go_count_toward_the_cap_and_a_gate_holds_meanwhile() {
    let mut responses = Vec::new();
    for id in ["c1", "c2", "c3"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_starts_cap", responses);
    let probe = handed(&world, "hooks = [\"turn.end\"]", QUIET);
    let gate =
        crate::ext_exec::extension_running(&world, "gate", "hooks = [\"tool.before\"]", LIST_GATE);
    let core = world.core_with(|options| options.extensions = vec![probe, gate]);
    let abandoned = cineris_ext::engine::abandoned_threads();
    world.run(async {
        let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
        for held in 1..=4 {
            let front = start(&world, &core).await;
            {
                let mut run = std::pin::pin!(core.hook_runs_at_its_point(
                    &front.session,
                    "probe/hook.turn.end",
                    1
                ));
                drive_until(run.as_mut(), || {
                    registers.wait_reached_times(held, Duration::ZERO)
                })
                .await;
            }
            settle().await;
            front.close().await;
        }
        assert_eq!(cineris_ext::engine::starts_let_go(), 4);
        let front = start(&world, &core).await;
        let told =
            "The extension `gate` could not start: 4 extension starts that closed sessions let go";
        for prompt in ["capped", "still capped"] {
            front.turn(prompt).await;
            let (outcome, text) = last(&front);
            assert_eq!(outcome, "denied", "{prompt}: {text}");
            assert!(
                text.contains("`gate/hook.tool.before` failed")
                    && text.contains("extension starts that closed sessions let go")
                    && text.contains("it clears within the evaluation time"),
                "{text}"
            );
            front.drain();
            assert_eq!(told_times(&front, told), 1, "{:?}", front.notices());
        }
        // One start ends within its time: released, and the gate runs.
        registers.let_one_go();
        let mut idle = std::pin::pin!(std::future::pending::<()>());
        drive_until(idle.as_mut(), || cineris_ext::engine::starts_let_go() == 3).await;
        registers.release();
        drop(registers);
        front.turn("released").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "success", "{text}");
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::abandoned_threads(), abandoned);
}

/// While the store of the user's pins cannot be read, the registration
/// cache only ever adds to an extension's gates: a project's extension of
/// the same name, with no hook, cached, does not lift the user's `probe`'s
/// gate, which its manifest lists (the fourth look at the script hooks,
/// L1).
fn a_same_named_cache_entry_does_not_lift_a_gate_under_an_unreadable_pin_store() {
    let world = World::new(
        "ext_hooks_cache_same_name",
        one_call("c1", "list", json!({"path": "."})),
    );
    let theirs = world.dir.join("elsewhere").join("probe");
    write_in(&theirs, "", "// a project's, with no hook\n");
    cache(&world, &theirs);
    let folder = write(&world, "hooks = [\"tool.before\"]", LIST_GATE);
    pin(&world, &folder);
    std::fs::write(state(&world).pins_file(), "version = 9\n").unwrap();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("list it").await;
        let (outcome, text) = last(&front);
        assert_eq!(
            outcome, "denied",
            "a same-named cache entry lifted the gate: {text}"
        );
        front.close().await;
    });
}

/// The count of starts let go cannot outlive a start on any path (the fifth
/// look at the script hooks, M1): two runs of one extension's `turn.end`
/// hook at once, its runtime not started; the first's start held as its
/// main module registers the hook, the second's waiting behind it for the
/// runtime; both let go. Only the start that runs counts. Let go on, the
/// first ends, the second finds the runtime alive and starts nothing, and
/// once all has ended and the session closed, the count is 0 and nothing
/// is left running.
#[test]
fn the_count_of_starts_let_go_ends_at_zero_when_a_start_waits_behind_another() {
    let world = World::new("ext_hooks_count_ends", Vec::<Response>::new());
    let extension = handed(&world, "hooks = [\"turn.end\"]", QUIET);
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let abandoned = cineris_ext::engine::abandoned_threads();
    world.run(async {
        let front = start(&world, &core).await;
        let registers = Hold::new(cineris_ext::engine::A_DEFINITION_IS_REGISTERED);
        {
            let mut runs = std::pin::pin!(core.hook_runs_at_its_point(
                &front.session,
                "probe/hook.turn.end",
                2
            ));
            drive_until(runs.as_mut(), || registers.wait_reached(Duration::ZERO)).await;
            // The second's start is waiting for the runtime meanwhile.
            for _ in 0..4 {
                let _ = poll_once(runs.as_mut()).await;
                settle().await;
            }
        }
        settle().await;
        assert_eq!(
            cineris_ext::engine::starts_let_go(),
            1,
            "only the start that runs counts"
        );
        registers.release();
        drop(registers);
        let mut idle = std::pin::pin!(std::future::pending::<()>());
        drive_until(idle.as_mut(), || cineris_ext::engine::starts_let_go() == 0).await;
        settle().await;
        front.close().await;
    });
    assert_eq!(cineris_ext::engine::starts_let_go(), 0);
    assert_eq!(cineris_ext::engine::abandoned_threads(), abandoned);
}

/// A trusted project's extension changed before the session holds its gate
/// from the registration the cache keeps at its pin; known by it, it keeps
/// that gate once the trust store cannot be read, though the session never
/// loaded it (the fifth look at the script hooks, L1).
fn a_gate_held_from_the_cache_is_kept_when_the_trust_store_breaks() {
    let mut responses = Vec::new();
    for id in ["c1", "c2"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_cache_gate_trust", responses);
    let folder = crate::extensions_found::projects(&world);
    write_in(&folder, "hooks = [\"tool.before\"]", LIST_GATE);
    crate::extensions_found::trust(
        &world,
        &[crate::extensions_trust::named_definition(&folder, "probe")],
    );
    cache(&world, &folder);
    write_in(
        &folder,
        "hooks = [\"tool.before\"]",
        &format!("// changed\n{LIST_GATE}"),
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("changed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        std::fs::write(state(&world).trust_file(), "version = 9\n").unwrap();
        front.turn("store broken").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "the cache's gate lapsed: {text}");
        front.close().await;
    });
}

/// The user's extension removed before the session holds its gate from the
/// registration the cache keeps at its pin; known by it, it keeps that gate
/// once the pin store cannot be read, though the session never loaded it
/// (the fifth look at the script hooks, L1).
fn a_gate_held_from_the_cache_is_kept_when_the_pin_store_breaks() {
    let mut responses = Vec::new();
    for id in ["c1", "c2"] {
        responses.extend(one_call(id, "list", json!({"path": "."})));
    }
    let world = World::new("ext_hooks_cache_gate_pins", responses);
    let folder = approved(&world, "hooks = [\"tool.before\"]", LIST_GATE);
    std::fs::remove_dir_all(&folder).unwrap();
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("removed").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "{text}");
        std::fs::write(state(&world).pins_file(), "version = 9\n").unwrap();
        front.turn("store broken").await;
        let (outcome, text) = last(&front);
        assert_eq!(outcome, "denied", "the cache's gate lapsed: {text}");
        front.close().await;
    });
}
