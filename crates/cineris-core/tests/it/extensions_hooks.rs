//! An extension's hooks (step 28): the functions a loaded extension's
//! script registered with `cineris.on`, on the loop's points beside the hook
//! commands and on the session's own events (`docs/steps/28/answered.md`,
//! points 2, 6 and 8; `docs/architecture/12-extensions.md`, "Tier 3:
//! hooks"). End to end, through the core: the user's extension found,
//! judged by its pin and taken from the registration cache, as
//! `extensions_found` has it, each test's story the turns of one session.
//! None starts a process: a hook's `cineris.exec` is a phase of the hook
//! commands' chained `sys_` test.

use std::path::PathBuf;
use std::time::Duration;

use cineris_core::ClientInfo;
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::methods::TrustResponse;
use cineris_testkit::Response;
use serde_json::{Value, json};

use crate::extensions_found::{cache, open_as, pin, start, users};
use crate::{Front, Outcome, World, calls, says, settle};

/// Writes the user's extension `probe`, with these capabilities and
/// `script` as its main module. Its folder.
fn write(world: &World, capabilities: &str, script: &str) -> PathBuf {
    let folder = users(world);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain \
             = \"main.js\"\n[capabilities]\n{capabilities}\n"
        ),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), script).unwrap();
    folder
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
#[test]
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
#[test]
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
        assert!(
            text.contains("`probe/hook.tool.before.2` failed")
                && text.contains("did not answer within 1 s"),
            "{text}"
        );
        assert!(
            text.contains("`probe/hook.tool.before` failed")
                && text.contains("did not answer within 3 s"),
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
#[test]
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
/// hooks do not start; `docs/outstanding/the-policys-guards-of-a-stop-from-a-run-have-no-test.md`).
/// With `fail_on_denial`, the extension's tool's write nobody can be asked
/// about stops the turn through the policy; the call's `tool.after` hooks,
/// in the same turn, start nothing: neither the one that would read nor the
/// one that would only answer adds a note, and the user is told each was
/// not started. Only the policy's stop of the turn says so: the call's
/// result alone ends no hook.
#[test]
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
#[test]
fn an_extensions_hooks_runs_are_its_own_and_bounded() {
    let world = World::new("ext_hooks_let_go", Vec::<Response>::new());
    // Handed to the core as approved, so that its hooks are on their
    // events as the session opens, with no turn to wait for them.
    let folder = write(
        &world,
        "hooks = [\"compaction.before\", \"turn.end\"]",
        LET_GO,
    );
    let snapshot = std::sync::Arc::new(cineris_ext::folder::Snapshot::read(&folder).unwrap());
    let (manifest, registration) = world
        .run(cineris_ext::engine::register(
            std::sync::Arc::clone(&snapshot),
            &world.timer,
        ))
        .unwrap();
    let extension = cineris_core::ext::ApprovedExtension {
        source: cineris_ext::source::Source::User,
        folder,
        hash: snapshot.hash(),
        manifest,
        registration,
    };
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
#[test]
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

/// A `session.start` hook that adds context, and a `session.end` hook that
/// keeps a value of the project's.
const SESSION: &str = "cineris.on('session.start', (e) => \
       ({ context: 'opened as ' + e.event + ' of a session ' + typeof e.session_id }));\n\
     cineris.on('session.end', async () => { \
       await cineris.state.set('ended', 'yes', { scope: 'project' }); });\n";

/// An extension's hooks on the session's own events (`docs/steps/28/answered.md`,
/// point 6): its `session.start` hook is heard before the first turn, its
/// context a notice of the hook's in the turn's first request; its
/// `session.end` hook runs as the session closes.
#[test]
fn an_extensions_hooks_run_as_its_session_opens_and_closes() {
    let world = World::new("ext_hooks_session", [says("Hello.")]);
    approved(
        &world,
        "hooks = [\"session.start\", \"session.end\"]",
        SESSION,
    );
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("hello").await;
        let sent = format!("{:?}", world.provider.requests()[0]);
        assert!(
            sent.contains("`probe/hook.session.start`")
                && sent.contains("opened as session.start of a session string"),
            "{sent}"
        );
        front.close().await;
    });
    let kept: Vec<String> = crate::ext_state::project_files(&world)
        .iter()
        .map(|file| std::fs::read_to_string(file).unwrap())
        .collect();
    assert!(
        kept.iter().any(|file| file.contains("\"ended\":\"yes\"")),
        "{kept:?}"
    );
}
